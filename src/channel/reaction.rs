//! Discord reaction-based status indicator for AI processing stages.
//!
//! Adds emoji reactions to the user's original message to show what the AI
//! is currently doing: thinking, using tools, done, error, stalled, etc.

use serenity::all::{ChannelId, Http, MessageId, ReactionType};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Processing states with their corresponding emoji.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionState {
    Queued,     // 👀
    Thinking,   // 🤔
    Coding,     // 👨‍💻
    Web,        // ⚡
    Tool,       // 🔥
    Compacting, // ✍
    Done,       // 👍
    Error,      // 😱
    StallSoft,  // 🥱
    StallHard,  // 😨
}

impl ReactionState {
    pub fn emoji(&self) -> &'static str {
        match self {
            ReactionState::Queued => "👀",
            ReactionState::Thinking => "🤔",
            ReactionState::Coding => "👨‍💻",
            ReactionState::Web => "⚡",
            ReactionState::Tool => "🔥",
            ReactionState::Compacting => "✍",
            ReactionState::Done => "👍",
            ReactionState::Error => "😱",
            ReactionState::StallSoft => "🥱",
            ReactionState::StallHard => "😨",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, ReactionState::Done | ReactionState::Error)
    }
}

/// Resolve which reaction state a tool name maps to.
pub fn resolve_tool_state(tool_name: &str) -> ReactionState {
    // Strip mcp__ prefix for matching
    let name = tool_name
        .strip_prefix("mcp__catclaw__")
        .or_else(|| tool_name.strip_prefix("mcp__"))
        .unwrap_or(tool_name);

    match name {
        "Bash" | "Read" | "Write" | "Edit" | "Glob" | "Grep" | "NotebookEdit" => {
            ReactionState::Coding
        }
        "WebFetch" | "WebSearch" => ReactionState::Web,
        _ => ReactionState::Tool,
    }
}

/// Command sent to the reaction controller background task.
#[allow(dead_code)]
enum ReactionCmd {
    SetState(ReactionState),
    Shutdown,
}

/// Handle to control the reaction controller from outside.
/// Drop or send Shutdown to stop the background task.
#[derive(Clone)]
pub struct ReactionHandle {
    tx: mpsc::UnboundedSender<ReactionCmd>,
}

impl ReactionHandle {
    /// Transition to a new state. Non-blocking.
    pub fn set_state(&self, state: ReactionState) {
        let _ = self.tx.send(ReactionCmd::SetState(state));
    }

    /// Signal done (success).
    pub fn done(&self) {
        self.set_state(ReactionState::Done);
    }

    /// Signal error.
    pub fn error(&self) {
        self.set_state(ReactionState::Error);
    }

    /// Shutdown the background task.
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        let _ = self.tx.send(ReactionCmd::Shutdown);
    }
}

const DEBOUNCE_MS: u64 = 700;
const STALL_SOFT_SECS: u64 = 10;
const STALL_HARD_SECS: u64 = 30;
const TERMINAL_HOLD_MS: u64 = 1500;

/// Spawn a reaction controller for a Discord message.
/// Returns a handle to control the state machine.
pub fn spawn(
    http: Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
) -> ReactionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(reaction_loop(http, channel_id, message_id, rx));
    ReactionHandle { tx }
}

/// Background task that manages reaction state transitions with debounce and stall detection.
async fn reaction_loop(
    http: Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
    mut rx: mpsc::UnboundedReceiver<ReactionCmd>,
) {
    let mut current_emoji: Option<&'static str> = None;
    let mut pending_state: Option<ReactionState> = None;
    let mut debounce_deadline: Option<Instant> = None;
    let mut last_state_change = Instant::now();
    let mut stall_level = 0u8; // 0=none, 1=soft, 2=hard
    // Terminal hold: after done/error, wait before removing
    let mut terminal_remove_at: Option<Instant> = None;

    loop {
        // Calculate next wakeup time
        let timeout = if terminal_remove_at.is_some() {
            Duration::from_millis(100) // poll for terminal hold expiry
        } else if let Some(deadline) = debounce_deadline {
            deadline.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(1) // stall check interval
        };

        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(ReactionCmd::SetState(state)) => {
                        // Ignore new commands during terminal hold
                        if terminal_remove_at.is_some() {
                            continue;
                        }

                        last_state_change = Instant::now();
                        stall_level = 0;

                        if state.is_terminal() {
                            // Terminal states apply immediately, no debounce
                            pending_state = None;
                            debounce_deadline = None;
                            apply_reaction(&http, channel_id, message_id, &mut current_emoji, state.emoji()).await;
                            // Schedule removal (non-blocking)
                            terminal_remove_at = Some(Instant::now() + Duration::from_millis(TERMINAL_HOLD_MS));
                        } else {
                            // Debounce: set/reset deadline
                            pending_state = Some(state);
                            debounce_deadline = Some(Instant::now() + Duration::from_millis(DEBOUNCE_MS));
                        }
                    }
                    Some(ReactionCmd::Shutdown) | None => {
                        remove_reaction(&http, channel_id, message_id, &mut current_emoji).await;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                // Check terminal hold expiry first
                if let Some(remove_at) = terminal_remove_at {
                    if Instant::now() >= remove_at {
                        remove_reaction(&http, channel_id, message_id, &mut current_emoji).await;
                        break; // Done, exit loop
                    }
                    continue;
                }

                // Debounce expired — apply pending state
                if let Some(state) = pending_state.take() {
                    debounce_deadline = None;
                    apply_reaction(&http, channel_id, message_id, &mut current_emoji, state.emoji()).await;
                } else {
                    // Stall detection
                    let elapsed = last_state_change.elapsed().as_secs();
                    if elapsed >= STALL_HARD_SECS && stall_level < 2 {
                        stall_level = 2;
                        apply_reaction(&http, channel_id, message_id, &mut current_emoji, ReactionState::StallHard.emoji()).await;
                    } else if elapsed >= STALL_SOFT_SECS && stall_level < 1 {
                        stall_level = 1;
                        apply_reaction(&http, channel_id, message_id, &mut current_emoji, ReactionState::StallSoft.emoji()).await;
                    }
                }
            }
        }
    }
}

/// Apply a reaction: remove the old one (if any), then add the new one.
async fn apply_reaction(
    http: &Http,
    channel_id: ChannelId,
    message_id: MessageId,
    current: &mut Option<&'static str>,
    new_emoji: &'static str,
) {
    if *current == Some(new_emoji) {
        return; // Already showing this emoji
    }

    // Remove old reaction
    if let Some(old) = current.take() {
        let reaction = ReactionType::Unicode(old.to_string());
        if let Err(e) = http.delete_reaction_me(channel_id, message_id, &reaction).await {
            debug!(error = %e, emoji = old, "failed to remove old reaction");
        }
    }

    // Add new reaction — only track as current if successful
    let reaction = ReactionType::Unicode(new_emoji.to_string());
    match http.create_reaction(channel_id, message_id, &reaction).await {
        Ok(_) => {
            *current = Some(new_emoji);
        }
        Err(e) => {
            warn!(error = %e, emoji = new_emoji, "failed to add reaction");
        }
    }
}

/// Remove the current reaction without adding a new one.
async fn remove_reaction(
    http: &Http,
    channel_id: ChannelId,
    message_id: MessageId,
    current: &mut Option<&'static str>,
) {
    if let Some(old) = current.take() {
        let reaction = ReactionType::Unicode(old.to_string());
        if let Err(e) = http.delete_reaction_me(channel_id, message_id, &reaction).await {
            debug!(error = %e, emoji = old, "failed to remove reaction on cleanup");
        }
    }
}
