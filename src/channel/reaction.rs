//! Reaction-based status indicator for AI processing stages.
//!
//! Adds emoji reactions to the user's original message to show what the AI
//! is currently doing: thinking, using tools, done, error, stalled, etc.
//! Supports Discord (Unicode emoji) and Slack (shortcode emoji via Web API).

use async_trait::async_trait;
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
    /// Unicode emoji for Discord.
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

    /// Slack shortcode name (without colons).
    pub fn slack_name(&self) -> &'static str {
        match self {
            ReactionState::Queued => "eyes",
            ReactionState::Thinking => "thinking_face",
            ReactionState::Coding => "technologist",
            ReactionState::Web => "zap",
            ReactionState::Tool => "fire",
            ReactionState::Compacting => "writing_hand",
            ReactionState::Done => "thumbsup",
            ReactionState::Error => "scream",
            ReactionState::StallSoft => "yawning_face",
            ReactionState::StallHard => "fearful",
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

// ── Backend trait ────────────────────────────────────────────────────────

/// Abstraction over platform-specific reaction APIs.
#[async_trait]
trait ReactionBackend: Send + Sync + 'static {
    /// Add a reaction. Returns the identifier to use when removing it.
    async fn add(&self, emoji_key: &str) -> Result<(), String>;

    /// Remove a reaction.
    async fn remove(&self, emoji_key: &str) -> Result<(), String>;

    /// Map a ReactionState to the platform's emoji key.
    fn emoji_key(&self, state: ReactionState) -> &'static str;
}

// ── Discord backend ─────────────────────────────────────────────────────

use serenity::all::{ChannelId, Http, MessageId, ReactionType};
use std::sync::Arc;

struct DiscordBackend {
    http: Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
}

#[async_trait]
impl ReactionBackend for DiscordBackend {
    async fn add(&self, emoji_key: &str) -> Result<(), String> {
        let reaction = ReactionType::Unicode(emoji_key.to_string());
        self.http
            .create_reaction(self.channel_id, self.message_id, &reaction)
            .await
            .map_err(|e| e.to_string())
    }

    async fn remove(&self, emoji_key: &str) -> Result<(), String> {
        let reaction = ReactionType::Unicode(emoji_key.to_string());
        self.http
            .delete_reaction_me(self.channel_id, self.message_id, &reaction)
            .await
            .map_err(|e| e.to_string())
    }

    fn emoji_key(&self, state: ReactionState) -> &'static str {
        state.emoji()
    }
}

// ── Slack backend ───────────────────────────────────────────────────────

struct SlackBackend {
    http: reqwest::Client,
    bot_token: String,
    channel_id: String,
    /// Slack message timestamp (e.g. "1234567890.123456")
    timestamp: String,
}

#[async_trait]
impl ReactionBackend for SlackBackend {
    async fn add(&self, emoji_key: &str) -> Result<(), String> {
        let url = "https://slack.com/api/reactions.add";
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .form(&[
                ("channel", self.channel_id.as_str()),
                ("timestamp", self.timestamp.as_str()),
                ("name", emoji_key),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(())
        } else {
            let err = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            // "already_reacted" is not a real error
            if err == "already_reacted" {
                Ok(())
            } else {
                Err(format!("reactions.add: {}", err))
            }
        }
    }

    async fn remove(&self, emoji_key: &str) -> Result<(), String> {
        let url = "https://slack.com/api/reactions.remove";
        let resp = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .form(&[
                ("channel", self.channel_id.as_str()),
                ("timestamp", self.timestamp.as_str()),
                ("name", emoji_key),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        if json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            Ok(())
        } else {
            let err = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            // "no_reaction" means it was already removed — not a real error
            if err == "no_reaction" {
                Ok(())
            } else {
                Err(format!("reactions.remove: {}", err))
            }
        }
    }

    fn emoji_key(&self, state: ReactionState) -> &'static str {
        state.slack_name()
    }
}

// ── Handle and state machine ────────────────────────────────────────────

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

/// Spawn a reaction controller for a Discord message.
pub fn spawn(
    http: Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
) -> ReactionHandle {
    spawn_with_backend(DiscordBackend {
        http,
        channel_id,
        message_id,
    })
}

/// Spawn a reaction controller for a Slack message.
pub fn spawn_slack(
    http: reqwest::Client,
    bot_token: String,
    channel_id: String,
    timestamp: String,
) -> ReactionHandle {
    spawn_with_backend(SlackBackend {
        http,
        bot_token,
        channel_id,
        timestamp,
    })
}

fn spawn_with_backend<B: ReactionBackend>(backend: B) -> ReactionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(reaction_loop(Box::new(backend), rx));
    ReactionHandle { tx }
}

/// Background task that manages reaction state transitions with debounce and stall detection.
async fn reaction_loop(
    backend: Box<dyn ReactionBackend>,
    mut rx: mpsc::UnboundedReceiver<ReactionCmd>,
) {
    let mut current_key: Option<&'static str> = None;
    let mut pending_state: Option<ReactionState> = None;
    let mut debounce_deadline: Option<Instant> = None;
    let mut last_state_change = Instant::now();
    let mut stall_level = 0u8; // 0=none, 1=soft, 2=hard

    loop {
        // Calculate next wakeup time
        let timeout = if let Some(deadline) = debounce_deadline {
            deadline.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(1) // stall check interval
        };

        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(ReactionCmd::SetState(state)) => {
                        last_state_change = Instant::now();
                        stall_level = 0;

                        if state.is_terminal() {
                            // Terminal: just remove the current reaction and exit.
                            remove_current(&*backend, &mut current_key).await;
                            break;
                        } else {
                            // Debounce: set/reset deadline
                            pending_state = Some(state);
                            debounce_deadline = Some(Instant::now() + Duration::from_millis(DEBOUNCE_MS));
                        }
                    }
                    Some(ReactionCmd::Shutdown) | None => {
                        remove_current(&*backend, &mut current_key).await;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                // Debounce expired — apply pending state
                if let Some(state) = pending_state.take() {
                    debounce_deadline = None;
                    let key = backend.emoji_key(state);
                    apply(&*backend, &mut current_key, key).await;
                } else {
                    // Stall detection
                    let elapsed = last_state_change.elapsed().as_secs();
                    if elapsed >= STALL_HARD_SECS && stall_level < 2 {
                        stall_level = 2;
                        let key = backend.emoji_key(ReactionState::StallHard);
                        apply(&*backend, &mut current_key, key).await;
                    } else if elapsed >= STALL_SOFT_SECS && stall_level < 1 {
                        stall_level = 1;
                        let key = backend.emoji_key(ReactionState::StallSoft);
                        apply(&*backend, &mut current_key, key).await;
                    }
                }
            }
        }
    }
}

/// Apply a reaction: add the new one first, then remove the old one.
/// This order prevents the "empty gap" flicker that occurs when removing first.
async fn apply(
    backend: &dyn ReactionBackend,
    current: &mut Option<&'static str>,
    new_key: &'static str,
) {
    if *current == Some(new_key) {
        return; // Already showing this emoji
    }

    let old = *current;

    // Add new reaction first — only track as current if successful
    match backend.add(new_key).await {
        Ok(_) => {
            *current = Some(new_key);
        }
        Err(e) => {
            warn!(error = %e, emoji = new_key, "failed to add reaction");
            return; // Don't remove old if new failed
        }
    }

    // Then remove old reaction
    if let Some(old_key) = old {
        if let Err(e) = backend.remove(old_key).await {
            debug!(error = %e, emoji = old_key, "failed to remove old reaction");
        }
    }
}

/// Remove the current reaction without adding a new one.
async fn remove_current(
    backend: &dyn ReactionBackend,
    current: &mut Option<&'static str>,
) {
    if let Some(old) = current.take() {
        if let Err(e) = backend.remove(old).await {
            debug!(error = %e, emoji = old, "failed to remove reaction on cleanup");
        }
    }
}
