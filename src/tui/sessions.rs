use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tui_textarea::TextArea;
use tokio::sync::mpsc;
use tracing::{info, error};

use super::chat::{self, ChatMessage};
use super::theme::Theme;
use super::{Action, Component};
use crate::agent::AgentLoader;
use crate::agent::models::{self, KNOWN_MODELS};
use crate::config::Config;
use crate::ws_client::GatewayClient;
use crate::ws_protocol::WsEvent;

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/new", "Start a new session (archives current)"),
    ("/model", "Switch model (opus, sonnet, haiku, clear)"),
    ("/help", "Show available commands"),
];

const MODEL_COMPLETIONS: &[&str] = &["opus", "sonnet", "haiku", "clear", "default"];

/// Viewing mode
enum Mode {
    /// Session list
    List,
    /// Chat — full panel with inline input
    Chat,
}

/// Messages from background tasks
#[allow(dead_code)]
enum ChatEvent {
    Response(String),
    Error(String),
    /// Incremental text delta (streaming)
    Delta(String),
    /// Tool use indicator (streaming)
    ToolUse { name: String, input: serde_json::Value },
    /// Sessions refreshed from server
    SessionsLoaded(Vec<SessionInfo>),
    /// Transcript loaded
    TranscriptLoaded(Vec<ChatMessage>),
    /// Default agent resolved — create pending session and enter chat
    NewSession(String),
    /// Create pending session but stay in list mode (auto-created default)
    NewSessionQuiet(String),
}

/// A not-yet-created session (first message will create it)
#[allow(dead_code)]
struct PendingSession {
    agent_id: String,
    session_key: String,
    /// Model override set before first message (applied on session creation)
    model: Option<String>,
}

/// Session info from WebSocket (replaces direct SessionRow dependency)
#[derive(Debug, Clone)]
struct SessionInfo {
    session_key: String,
    session_id: String,
    agent_id: String,
    origin: String,
    context_id: String,
    state: String,
    last_activity_at: String,
    model: Option<String>,
}

pub struct SessionsPanel {
    client: Arc<GatewayClient>,
    sessions: Vec<SessionInfo>,
    selected: usize,
    mode: Mode,
    messages: Vec<ChatMessage>,
    chat_scroll: u16,
    textarea: TextArea<'static>,
    loading: bool,
    pending_session: Option<PendingSession>,
    response_rx: mpsc::UnboundedReceiver<ChatEvent>,
    response_tx: mpsc::UnboundedSender<ChatEvent>,
    /// Receiver for WS push events (session.response / session.error / session.delta / session.tool_use)
    ws_event_rx: mpsc::UnboundedReceiver<WsEvent>,
    /// When the current loading started (for elapsed time display)
    loading_since: Option<std::time::Instant>,
    /// Frame counter for spinner animation
    tick: u64,
    /// Whether initial load has been triggered
    loaded: bool,
    /// Whether streaming mode is enabled (from config)
    streaming_enabled: bool,
    /// Slash command autocomplete suggestions
    slash_completions: Vec<String>,
    /// Selected slash completion index
    slash_idx: usize,
    /// agent_id → workspace path, for loading skills into slash completions
    agent_workspaces: std::collections::HashMap<String, std::path::PathBuf>,
    /// shared workspace root (for skill pool)
    workspace_root: std::path::PathBuf,
    /// Pending tool approval requests from the agent
    pending_approvals: Vec<PendingApprovalItem>,
    /// Index of selected approval when reviewing
    approval_selected: usize,
    /// 0 = Approve selected, 1 = Deny selected
    approval_choice: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PendingApprovalItem {
    request_id: String,
    session_key: String,
    tool_name: String,
    tool_input: serde_json::Value,
    expires_secs: u64,
    received_at: std::time::Instant,
}

impl SessionsPanel {
    pub fn new(
        client: Arc<GatewayClient>,
        ws_event_rx: mpsc::UnboundedReceiver<WsEvent>,
        streaming_enabled: bool,
        config: &Config,
    ) -> Self {
        let (response_tx, response_rx) = mpsc::unbounded_channel();
        let agent_workspaces = config.agents.iter()
            .map(|a| (a.id.clone(), a.workspace.clone()))
            .collect();
        let workspace_root = config.general.workspace.clone();

        SessionsPanel {
            client,
            sessions: Vec::new(),
            selected: 0,
            mode: Mode::List,
            messages: Vec::new(),
            chat_scroll: 0,
            textarea: make_textarea(),
            loading: false,
            pending_session: None,
            response_rx,
            response_tx,
            ws_event_rx,
            loading_since: None,
            tick: 0,
            loaded: false,
            streaming_enabled,
            slash_completions: Vec::new(),
            slash_idx: 0,
            agent_workspaces,
            workspace_root,
            pending_approvals: Vec::new(),
            approval_selected: 0,
            approval_choice: 0,
        }
    }

    fn refresh(&mut self) {
        let client = self.client.clone();
        let tx = self.response_tx.clone();
        tokio::spawn(async move {
            match client.request("sessions.list", json!({})).await {
                Ok(val) => {
                    let sessions = parse_sessions(&val);
                    let _ = tx.send(ChatEvent::SessionsLoaded(sessions));
                }
                Err(e) => {
                    error!(error = %e, "failed to fetch sessions");
                }
            }
        });
    }

    fn load_transcript(&mut self) {
        self.messages.clear();
        self.chat_scroll = 0;

        let session = match self.sessions.get(self.selected) {
            Some(s) => s.clone(),
            None => return,
        };

        let client = self.client.clone();
        let tx = self.response_tx.clone();
        tokio::spawn(async move {
            match client
                .request(
                    "sessions.transcript",
                    json!({
                        "agent_id": session.agent_id,
                        "session_id": session.session_id,
                    }),
                )
                .await
            {
                Ok(val) => {
                    let messages = parse_transcript(&val, &session.agent_id);
                    let _ = tx.send(ChatEvent::TranscriptLoaded(messages));
                }
                Err(e) => {
                    error!(error = %e, "failed to load transcript");
                }
            }
        });
    }

    fn send_message(&mut self, text: String) {
        let now = chrono::Utc::now();

        let (agent_id, key_str, pending_model) = if let Some(pending) = &self.pending_session {
            (pending.agent_id.clone(), pending.session_key.clone(), pending.model.clone())
        } else if let Some(session) = self.sessions.get(self.selected) {
            (session.agent_id.clone(), session.session_key.clone(), None)
        } else {
            return;
        };

        self.messages.push(ChatMessage {
            sender: "You".to_string(),
            text: text.clone(),
            is_user: true,
            timestamp: now.format("%H:%M").to_string(),
            streaming: false,
        });
        self.chat_scroll = u16::MAX;
        self.loading = true;
        self.loading_since = Some(std::time::Instant::now());

        let client = self.client.clone();
        let tx = self.response_tx.clone();
        let key = key_str.clone();
        let stream = self.streaming_enabled;

        info!(session_key = %key, agent = %agent_id, stream = stream, "TUI: sending message via WS");

        tokio::spawn(async move {
            let mut send_params = json!({
                "key": key,
                "agent_id": agent_id,
                "message": text,
                "stream": stream,
            });
            // Pass pending model to sessions.send so it's applied on session creation
            if let Some(model) = pending_model {
                send_params["model"] = serde_json::Value::String(model);
            }

            match client.request("sessions.send", send_params).await {
                Ok(val) => {
                    let _request_id = val.get("request_id").and_then(|v| v.as_u64());
                    // Response will arrive via WsEvent channel
                }
                Err(e) => {
                    error!(error = %e, "sessions.send failed");
                    let _ = tx.send(ChatEvent::Error(e));
                }
            }
        });
    }

    fn new_session(&mut self) {
        // Get default agent via WS
        let client = self.client.clone();

        // Check if there's already a TUI session we can jump to
        for (idx, s) in self.sessions.iter().enumerate() {
            if s.origin == "tui" && s.context_id == "default" {
                self.selected = idx;
                self.load_transcript();
                self.mode = Mode::Chat;
                return;
            }
        }

        // Need to create a pending session — get default agent
        let sessions = self.sessions.clone();
        let response_tx = self.response_tx.clone();
        tokio::spawn(async move {
            match client.request("agents.default", json!({})).await {
                Ok(val) => {
                    if let Some(agent_id) = val.get("id").and_then(|v| v.as_str()) {
                        let key = format!("catclaw:{}:tui:default", agent_id);
                        // Check again (race-safe)
                        if sessions.iter().any(|s| s.session_key == key) {
                            return; // already exists, handled above
                        }
                        let _ = response_tx.send(ChatEvent::NewSession(
                            agent_id.to_string(),
                        ));
                    }
                }
                Err(e) => {
                    error!(error = %e, "failed to get default agent");
                }
            }
        });
    }

    /// Poll for background responses. Called every tick, even when Sessions tab is not active.
    pub fn poll_responses(&mut self) {
        // Poll internal chat events
        while let Ok(event) = self.response_rx.try_recv() {
            match event {
                ChatEvent::Delta(text) => {
                    // First delta: turn off loading spinner, create streaming message
                    if self.loading {
                        self.loading = false;
                        self.loading_since = None;
                        let now = chrono::Utc::now();
                        let agent_name = self.current_agent_name();
                        self.messages.push(ChatMessage {
                            sender: agent_name,
                            text: String::new(),
                            is_user: false,
                            timestamp: now.format("%H:%M").to_string(),
                            streaming: true,
                        });
                    }
                    // Append delta to the last streaming message
                    if let Some(last) = self.messages.last_mut() {
                        if last.streaming {
                            last.text.push_str(&text);
                        }
                    }
                    self.chat_scroll = u16::MAX;
                }
                ChatEvent::ToolUse { name, input: _ } => {
                    // Finalize any current streaming message, then show tool use indicator
                    if let Some(last) = self.messages.last_mut() {
                        if last.streaming {
                            last.streaming = false;
                        }
                    }
                    let now = chrono::Utc::now();
                    self.messages.push(ChatMessage {
                        sender: "tool".to_string(),
                        text: format!("Using {}...", name),
                        is_user: false,
                        timestamp: now.format("%H:%M").to_string(),
                        streaming: false,
                    });
                    // Start new streaming message for subsequent text
                    self.loading = true;
                    self.loading_since = Some(std::time::Instant::now());
                    self.chat_scroll = u16::MAX;
                }
                ChatEvent::Response(text) => {
                    self.loading = false;
                    self.loading_since = None;
                    // Finalize any streaming message
                    if let Some(last) = self.messages.last_mut() {
                        if last.streaming {
                            last.streaming = false;
                            // If streaming was active, we already have the text
                            // The Response just confirms completion
                            self.chat_scroll = u16::MAX;
                            if self.pending_session.take().is_some() {
                                self.refresh();
                            } else {
                                self.refresh();
                            }
                            continue;
                        }
                    }
                    // Non-streaming: add complete message
                    let now = chrono::Utc::now();
                    let agent_name = self.current_agent_name();
                    self.messages.push(ChatMessage {
                        sender: agent_name,
                        text,
                        is_user: false,
                        timestamp: now.format("%H:%M").to_string(),
                        streaming: false,
                    });
                    self.chat_scroll = u16::MAX;
                    self.pending_session.take();
                    self.refresh();
                }
                ChatEvent::Error(err) => {
                    self.loading = false;
                    self.loading_since = None;
                    // Finalize any streaming message
                    if let Some(last) = self.messages.last_mut() {
                        if last.streaming {
                            last.streaming = false;
                        }
                    }
                    let now = chrono::Utc::now();
                    self.messages.push(ChatMessage {
                        sender: "error".to_string(),
                        text: err,
                        is_user: false,
                        timestamp: now.format("%H:%M").to_string(),
                        streaming: false,
                    });
                }
                ChatEvent::NewSession(agent_id) => {
                    let key = format!("catclaw:{}:tui:default", agent_id);
                    self.pending_session = Some(PendingSession {
                        agent_id,
                        session_key: key,
                        model: None,
                    });
                    self.messages.clear();
                    self.chat_scroll = 0;
                    self.mode = Mode::Chat;
                }
                ChatEvent::NewSessionQuiet(agent_id) => {
                    let key = format!("catclaw:{}:tui:default", agent_id);
                    // Only create if not already present
                    let already = self.pending_session.as_ref().map_or(false, |p| p.session_key == key)
                        || self.sessions.iter().any(|s| s.session_key == key);
                    if !already {
                        self.pending_session = Some(PendingSession {
                            agent_id,
                            session_key: key,
                            model: None,
                        });
                    }
                    // Stay in current mode (List)
                }
                ChatEvent::SessionsLoaded(sessions) => {
                    let was_empty = self.sessions.is_empty();
                    self.sessions = sessions;
                    if self.selected >= self.sessions.len() && !self.sessions.is_empty() {
                        self.selected = self.sessions.len() - 1;
                    }
                    // Auto-create a default pending session if list is empty
                    // (shows in list but doesn't enter chat)
                    if was_empty && self.sessions.is_empty() && self.pending_session.is_none() {
                        let client = self.client.clone();
                        let tx = self.response_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(val) = client.request("agents.default", json!({})).await {
                                if let Some(agent_id) = val.get("id").and_then(|v| v.as_str()) {
                                    let _ = tx.send(ChatEvent::NewSessionQuiet(agent_id.to_string()));
                                }
                            }
                        });
                    }
                }
                ChatEvent::TranscriptLoaded(messages) => {
                    self.messages = messages;
                    self.chat_scroll = u16::MAX;
                }
            }
        }

        // Poll WS push events
        while let Ok(event) = self.ws_event_rx.try_recv() {
            match event.event.as_str() {
                "session.delta" => {
                    if let Some(text) = event.data.get("text").and_then(|v| v.as_str()) {
                        let _ = self.response_tx.send(ChatEvent::Delta(text.to_string()));
                    }
                }
                "session.tool_use" => {
                    let name = event.data.get("tool").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let input = event.data.get("input").cloned().unwrap_or(serde_json::Value::Null);
                    let _ = self.response_tx.send(ChatEvent::ToolUse { name, input });
                }
                "session.response" => {
                    if let Some(text) = event.data.get("text").and_then(|v| v.as_str()) {
                        let _ = self.response_tx.send(ChatEvent::Response(text.to_string()));
                    }
                }
                "session.error" => {
                    if let Some(err) = event.data.get("error").and_then(|v| v.as_str()) {
                        let _ = self.response_tx.send(ChatEvent::Error(err.to_string()));
                    }
                }
                "approval.pending" => {
                    let request_id = event.data.get("request_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let session_key = event.data.get("session_key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let tool_name = event.data.get("tool_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let tool_input = event.data.get("tool_input").cloned().unwrap_or(serde_json::Value::Null);
                    let expires_secs = event.data.get("expires_secs").and_then(|v| v.as_u64()).unwrap_or(120);
                    if !request_id.is_empty() {
                        self.pending_approvals.push(PendingApprovalItem {
                            request_id,
                            session_key,
                            tool_name,
                            tool_input,
                            expires_secs,
                            received_at: std::time::Instant::now(),
                        });
                    }
                }
                "approval.result" => {
                    // Remove resolved approval from pending list
                    let rid = event.data.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
                    self.pending_approvals.retain(|a| a.request_id != rid);
                    if self.approval_selected >= self.pending_approvals.len() {
                        self.approval_selected = self.pending_approvals.len().saturating_sub(1);
                    }
                }
                "session.updated" => {
                    // Channel session changed — refresh list
                    self.refresh();
                }
                _ => {}
            }
        }

        // Evict expired approvals
        let now = std::time::Instant::now();
        self.pending_approvals.retain(|a| now.duration_since(a.received_at).as_secs() < a.expires_secs);
    }

    /// Scroll chat up by N lines (for mouse wheel)
    pub fn scroll_up(&mut self, lines: u16) {
        self.chat_scroll = self.chat_scroll.saturating_sub(lines);
    }

    /// Scroll chat down by N lines (for mouse wheel)
    pub fn scroll_down(&mut self, lines: u16) {
        self.chat_scroll = self.chat_scroll.saturating_add(lines);
    }

    /// Send an approval response for the currently selected pending approval.
    fn respond_approval(&mut self, approved: bool) {
        if let Some(item) = self.pending_approvals.get(self.approval_selected) {
            let client = self.client.clone();
            let request_id = item.request_id.clone();
            tokio::spawn(async move {
                let _ = client.request("approval.respond", serde_json::json!({
                    "request_id": request_id,
                    "approved": approved,
                })).await;
            });
        }
    }

    #[allow(dead_code)]
    pub fn has_pending_approvals(&self) -> bool {
        !self.pending_approvals.is_empty()
    }

    fn stop_current_session(&mut self) {
        let key_str = if let Some(pending) = &self.pending_session {
            pending.session_key.clone()
        } else if let Some(session) = self.sessions.get(self.selected) {
            session.session_key.clone()
        } else {
            return;
        };

        let client = self.client.clone();
        info!(session_key = %key_str, "TUI: stopping session");
        tokio::spawn(async move {
            let _ = client.request("sessions.stop", json!({ "key": key_str })).await;
        });
    }

    fn current_agent_name(&self) -> String {
        if let Some(pending) = &self.pending_session {
            pending.agent_id.clone()
        } else {
            self.sessions.get(self.selected)
                .map(|s| s.agent_id.clone())
                .unwrap_or_else(|| "assistant".to_string())
        }
    }

    fn current_session_model(&self) -> Option<String> {
        if let Some(pending) = &self.pending_session {
            return pending.model.clone();
        }
        self.sessions.get(self.selected).and_then(|s| s.model.clone())
    }

    fn handle_new_session(&mut self) {
        // Delete current session (archive it), then create a new pending session
        let current_key = if let Some(pending) = &self.pending_session {
            Some(pending.session_key.clone())
        } else {
            self.sessions.get(self.selected).map(|s| s.session_key.clone())
        };

        let client = self.client.clone();
        let tx = self.response_tx.clone();

        if let Some(key) = current_key {
            tokio::spawn(async move {
                // Delete the current session
                let _ = client.request("sessions.delete", json!({ "key": key })).await;
                // Refresh list
                if let Ok(val) = client.request("sessions.list", json!({})).await {
                    let _ = tx.send(ChatEvent::SessionsLoaded(parse_sessions(&val)));
                }
                // Create new session with the default agent
                if let Ok(val) = client.request("agents.default", json!({})).await {
                    if let Some(agent_id) = val.get("id").and_then(|v| v.as_str()) {
                        let _ = tx.send(ChatEvent::NewSession(agent_id.to_string()));
                    }
                }
            });
        }

        // Clear chat state immediately for responsiveness
        self.messages.clear();
        self.pending_session = None;
        self.loading = false;
        self.chat_scroll = 0;

        let now = chrono::Utc::now();
        self.messages.push(ChatMessage {
            sender: "system".to_string(),
            text: "Starting new session...".to_string(),
            is_user: false,
            timestamp: now.format("%H:%M").to_string(),
            streaming: false,
        });
    }

    fn handle_model_command(&mut self, text: &str) {
        let arg = text.strip_prefix("/model").unwrap().trim();
        let now = chrono::Utc::now();

        if arg.is_empty() {
            // Show current model + available models
            let current = self.current_session_model()
                .map(|m| models::model_display_name(&m).to_string())
                .unwrap_or_else(|| "(default)".to_string());

            let mut info = format!("Current model: {}\n\nAvailable models:", current);
            for &(short, full) in KNOWN_MODELS {
                info.push_str(&format!("\n  {} — {}", short, full));
            }
            info.push_str("\n\nUse /model <name> to switch. /model clear to reset.");

            self.messages.push(ChatMessage {
                sender: "system".to_string(),
                text: info,
                is_user: false,
                timestamp: now.format("%H:%M").to_string(),
                streaming: false,
            });
            return;
        }

        // "clear" removes the override
        let model_value = if arg.eq_ignore_ascii_case("clear") || arg.eq_ignore_ascii_case("default") {
            None
        } else {
            Some(models::resolve_model(arg))
        };

        let display = model_value.as_deref()
            .map(|m| models::model_display_name(m).to_string())
            .unwrap_or_else(|| "(default)".to_string());

        // For pending sessions (not yet in DB), store model locally — it will be
        // applied when the session is created by the first message send.
        if let Some(pending) = &mut self.pending_session {
            pending.model = model_value;
            self.messages.push(ChatMessage {
                sender: "system".to_string(),
                text: format!("Model set to: {} (will apply on first message)", display),
                is_user: false,
                timestamp: now.format("%H:%M").to_string(),
                streaming: false,
            });
            return;
        }

        let key_str = if let Some(session) = self.sessions.get(self.selected) {
            session.session_key.clone()
        } else {
            self.messages.push(ChatMessage {
                sender: "system".to_string(),
                text: "No active session to set model on.".to_string(),
                is_user: false,
                timestamp: now.format("%H:%M").to_string(),
                streaming: false,
            });
            return;
        };

        // Update local state immediately
        if let Some(session) = self.sessions.get_mut(self.selected) {
            session.model = model_value.clone();
        }

        self.messages.push(ChatMessage {
            sender: "system".to_string(),
            text: format!("Model set to: {}", display),
            is_user: false,
            timestamp: now.format("%H:%M").to_string(),
            streaming: false,
        });

        // Send to gateway
        let client = self.client.clone();
        tokio::spawn(async move {
            let params = match &model_value {
                Some(m) => json!({ "key": key_str, "model": m }),
                None => json!({ "key": key_str }),
            };
            let _ = client.request("sessions.set_model", params).await;
        });
    }

    fn update_slash_completions(&mut self) {
        let text: String = self.textarea.lines().join("\n");
        self.slash_completions.clear();
        self.slash_idx = 0;

        if !text.starts_with('/') {
            return;
        }

        if text.starts_with("/model ") {
            // Sub-completions for model argument
            let arg = text.strip_prefix("/model ").unwrap().to_lowercase();
            for &m in MODEL_COMPLETIONS {
                if arg.is_empty() || m.starts_with(&arg) {
                    self.slash_completions.push(m.to_string());
                }
            }
        } else if !text.contains(' ') {
            // Command-level completions: built-in commands + enabled skills
            let query = text.to_lowercase();

            // Built-in commands
            for &(cmd, desc) in SLASH_COMMANDS {
                if cmd.starts_with(&query) {
                    self.slash_completions.push(format!("{} — {}", cmd, desc));
                }
            }

            // Skills from the current session's agent workspace
            let workspace = self.pending_session.as_ref()
                .and_then(|p| self.agent_workspaces.get(&p.agent_id))
                .or_else(|| {
                    // Fall back to the selected session's agent
                    self.sessions.get(self.selected)
                        .and_then(|s| self.agent_workspaces.get(&s.agent_id))
                })
                .cloned();

            let workspace_root = self.workspace_root.clone();
            if let Some(ws) = workspace {
                let skills = AgentLoader::list_skills(&ws, &workspace_root);
                for skill in &skills {
                    if !skill.is_enabled {
                        continue;
                    }
                    let cmd = format!("/{}", skill.name);
                    if !cmd.starts_with(&query) {
                        continue;
                    }
                    let desc = skill.description.clone();
                    if desc.is_empty() {
                        self.slash_completions.push(format!("{} — skill", cmd));
                    } else {
                        // Truncate long descriptions for display
                        let short = if desc.chars().count() > 60 {
                            format!("{}…", desc.chars().take(60).collect::<String>())
                        } else {
                            desc
                        };
                        self.slash_completions.push(format!("{} — {}", cmd, short));
                    }
                }
            }
        }
    }

    fn accept_slash_completion(&mut self) {
        if let Some(completion) = self.slash_completions.get(self.slash_idx) {
            let text: String = self.textarea.lines().join("\n");
            let new_text = if text.starts_with("/model ") {
                // Model sub-completion: replace with full command
                format!("/model {}", completion)
            } else {
                // Command completion: extract command name from "cmd — desc"
                let cmd = completion.split(" — ").next().unwrap_or(completion);
                format!("{} ", cmd)
            };
            self.textarea = make_textarea();
            // Insert the text character by character
            for c in new_text.chars() {
                self.textarea.input(KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
            }
            self.slash_completions.clear();
            self.slash_idx = 0;
            // Re-compute completions for the new text
            self.update_slash_completions();
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        let active = self.sessions.iter().filter(|s| s.state == "active").count();
        let idle = self.sessions.iter().filter(|s| s.state == "idle").count();
        (active, idle)
    }
}

fn make_textarea() -> TextArea<'static> {
    let mut ta = TextArea::default();
    ta.set_cursor_line_style(Style::default());
    ta.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
    ta
}

fn parse_sessions(val: &serde_json::Value) -> Vec<SessionInfo> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|v| {
            Some(SessionInfo {
                session_key: v.get("session_key")?.as_str()?.to_string(),
                session_id: v.get("session_id")?.as_str()?.to_string(),
                agent_id: v.get("agent_id")?.as_str()?.to_string(),
                origin: v.get("origin")?.as_str()?.to_string(),
                context_id: v.get("context_id")?.as_str()?.to_string(),
                state: v.get("state")?.as_str()?.to_string(),
                last_activity_at: v.get("last_activity_at")?.as_str()?.to_string(),
                model: v.get("model").and_then(|v| v.as_str()).map(String::from),
            })
        })
        .collect()
}

fn parse_transcript(val: &serde_json::Value, agent_id: &str) -> Vec<ChatMessage> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|v| {
            let role = v.get("role")?.as_str()?;
            let text = v.get("content")?.as_str()?;
            let sender = if role == "user" {
                v.get("sender_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("You")
                    .to_string()
            } else if role == "assistant" {
                agent_id.to_string()
            } else {
                return None;
            };
            let ts = v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            Some(ChatMessage {
                sender,
                text: text.to_string(),
                is_user: role == "user",
                timestamp: format_time(ts),
                streaming: false,
            })
        })
        .collect()
}

fn format_timestamp(ts: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(dt);
        if diff.num_seconds() < 60 {
            "just now".to_string()
        } else if diff.num_minutes() < 60 {
            format!("{}m ago", diff.num_minutes())
        } else if diff.num_hours() < 24 {
            format!("{}h ago", diff.num_hours())
        } else {
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
    } else {
        ts.to_string()
    }
}

fn format_time(ts: &str) -> String {
    if let Some(t_pos) = ts.find('T') {
        let after = &ts[t_pos + 1..];
        if after.len() >= 5 { after[..5].to_string() } else { after.to_string() }
    } else {
        ts.to_string()
    }
}

/// Read a skill's description from its SKILL.md frontmatter.

impl Component for SessionsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match &self.mode {
            Mode::List => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    let has_pending = self.pending_session.as_ref()
                        .map_or(false, |p| !self.sessions.iter().any(|s| s.session_key == p.session_key));
                    let total = self.sessions.len() + if has_pending { 1 } else { 0 };
                    if total > 0 {
                        self.selected = (self.selected + 1).min(total - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Enter => {
                    let has_pending = self.pending_session.as_ref()
                        .map_or(false, |p| !self.sessions.iter().any(|s| s.session_key == p.session_key));
                    let pending_at_0 = has_pending && self.selected == 0;
                    let session_idx = if has_pending { self.selected.saturating_sub(1) } else { self.selected };

                    if pending_at_0 {
                        // Enter pending session directly
                        self.mode = Mode::Chat;
                    } else if !self.sessions.is_empty() {
                        // Adjust selected for DB sessions if pending is prepended
                        if self.selected != session_idx {
                            self.selected = session_idx;
                        }
                        if !self.loading {
                            self.load_transcript();
                        }
                        self.mode = Mode::Chat;
                    }
                    Action::None
                }
                KeyCode::Char('n') => {
                    self.new_session();
                    Action::None
                }
                KeyCode::Char('r') => {
                    self.refresh();
                    Action::None
                }
                KeyCode::Char('d') => {
                    let has_pending = self.pending_session.as_ref()
                        .map_or(false, |p| !self.sessions.iter().any(|s| s.session_key == p.session_key));
                    let session_idx = if has_pending { self.selected.saturating_sub(1) } else { self.selected };
                    if let Some(session) = self.sessions.get(session_idx) {
                        let key = session.session_key.clone();
                        let client = self.client.clone();
                        let tx = self.response_tx.clone();
                        tokio::spawn(async move {
                            let _ = client.request("sessions.delete", json!({ "key": key })).await;
                            // Refresh after delete completes
                            if let Ok(val) = client.request("sessions.list", json!({})).await {
                                let _ = tx.send(ChatEvent::SessionsLoaded(parse_sessions(&val)));
                            }
                        });
                    }
                    Action::None
                }
                _ => Action::None,
            },
            Mode::Chat => {
                // Ctrl+K: stop the running session (kill claude process)
                if event.code == KeyCode::Char('k') && event.modifiers.contains(KeyModifiers::CONTROL) {
                    if self.loading {
                        self.stop_current_session();
                        self.loading = false;
                        self.loading_since = None;
                        // Finalize any streaming message
                        if let Some(last) = self.messages.last_mut() {
                            if last.streaming {
                                last.streaming = false;
                            }
                        }
                        let now = chrono::Utc::now();
                        self.messages.push(ChatMessage {
                            sender: "system".to_string(),
                            text: "Session stopped.".to_string(),
                            is_user: false,
                            timestamp: now.format("%H:%M").to_string(),
                            streaming: false,
                        });
                    }
                    return Action::None;
                }

                // Slash completion intercepts
                if !self.slash_completions.is_empty() {
                    match event.code {
                        KeyCode::Tab => {
                            self.accept_slash_completion();
                            return Action::None;
                        }
                        KeyCode::Down => {
                            let count = self.slash_completions.len();
                            if count > 0 {
                                self.slash_idx = (self.slash_idx + 1).min(count - 1);
                            }
                            return Action::None;
                        }
                        KeyCode::Up => {
                            self.slash_idx = self.slash_idx.saturating_sub(1);
                            return Action::None;
                        }
                        _ => {} // fall through to normal handling
                    }
                }

                // Enter sends; Shift+Enter inserts newline
                if event.code == KeyCode::Enter {
                    if event.modifiers.contains(KeyModifiers::SHIFT) {
                        // Shift+Enter → newline (pass to textarea below)
                        self.textarea.input(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
                        self.update_slash_completions();
                        return Action::None;
                    }
                    // If there are slash completions visible, Enter accepts the completion
                    // and stays in input mode (lets user continue typing or send again)
                    if !self.slash_completions.is_empty() {
                        self.accept_slash_completion();
                        return Action::None;
                    }
                    // Plain Enter with empty input + pending approval → confirm approval choice
                    let text: String = self.textarea.lines().join("\n").trim().to_string();
                    if text.is_empty() && !self.pending_approvals.is_empty() {
                        self.respond_approval(self.approval_choice == 0);
                        self.approval_choice = 0;
                        return Action::None;
                    }
                    if !text.is_empty() && !self.loading {
                        if text == "/new" {
                            self.handle_new_session();
                            self.textarea = make_textarea();
                        } else if text.starts_with("/model") {
                            self.handle_model_command(&text);
                            self.textarea = make_textarea();
                        } else if text == "/help" {
                            let now = chrono::Utc::now();
                            let mut help = String::from("Available commands:\n");
                            for &(cmd, desc) in SLASH_COMMANDS {
                                help.push_str(&format!("  {} — {}\n", cmd, desc));
                            }
                            help.push_str("\nSkills (send as-is to invoke):\n");
                            help.push_str("  /skill-name — loads skill into Claude's context");
                            self.messages.push(ChatMessage {
                                sender: "system".to_string(),
                                text: help,
                                is_user: false,
                                timestamp: now.format("%H:%M").to_string(),
                                streaming: false,
                            });
                            self.textarea = make_textarea();
                        } else {
                            // All other text (including /skill-name) → send to claude
                            self.send_message(text);
                            self.textarea = make_textarea();
                        }
                    }
                    self.slash_completions.clear();
                    self.slash_idx = 0;
                    return Action::None;
                }

                // Esc goes back to list (does NOT interrupt background work)
                if event.code == KeyCode::Esc {
                    self.mode = Mode::List;
                    self.slash_completions.clear();
                    self.slash_idx = 0;
                    // Don't clear pending_session or loading — background work continues
                    self.refresh();
                    return Action::None;
                }

                // Chat scroll: PageUp / PageDown
                match event.code {
                    KeyCode::PageUp => {
                        self.chat_scroll = self.chat_scroll.saturating_sub(10);
                        return Action::None;
                    }
                    KeyCode::PageDown => {
                        self.chat_scroll = self.chat_scroll.saturating_add(10);
                        return Action::None;
                    }
                    // Ctrl+Up / Ctrl+Down for line-by-line scroll
                    KeyCode::Up if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.chat_scroll = self.chat_scroll.saturating_sub(1);
                        return Action::None;
                    }
                    KeyCode::Down if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.chat_scroll = self.chat_scroll.saturating_add(1);
                        return Action::None;
                    }
                    _ => {}
                }

                // Approval interaction (when there are pending approvals)
                if !self.pending_approvals.is_empty() {
                    match (event.modifiers, event.code) {
                        // Left/Right/Tab toggles approve/deny selection
                        (_, KeyCode::Tab) | (KeyModifiers::NONE, KeyCode::Left) | (KeyModifiers::NONE, KeyCode::Right) => {
                            self.approval_choice = 1 - self.approval_choice;
                            return Action::None;
                        }
                        // Ctrl+J/K to switch between multiple pending approvals
                        (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                            if self.approval_selected + 1 < self.pending_approvals.len() {
                                self.approval_selected += 1;
                                self.approval_choice = 0;
                            }
                            return Action::None;
                        }
                        (KeyModifiers::CONTROL, KeyCode::Char('k')) if !self.loading => {
                            self.approval_selected = self.approval_selected.saturating_sub(1);
                            self.approval_choice = 0;
                            return Action::None;
                        }
                        _ => {}
                    }
                }

                // Pass other input to textarea
                self.textarea.input(*event);
                self.update_slash_completions();
                Action::None
            }
        }
    }

    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::Chat)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Trigger initial load
        if !self.loaded {
            self.loaded = true;
            self.refresh();
        }

        // poll_responses() is called from App::tick() every frame, not here
        self.tick = self.tick.wrapping_add(1);

        match &self.mode {
            Mode::List => self.render_list(frame, area),
            Mode::Chat => self.render_chat_view(frame, area),
        }
    }
}

// ── Rendering ──

impl SessionsPanel {
    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        // If there's a pending session not yet in DB, prepend it to the display list
        let pending_idx: Option<usize> = if let Some(p) = &self.pending_session {
            let already_in_list = self.sessions.iter().any(|s| s.session_key == p.session_key);
            if !already_in_list { Some(0) } else { None }
        } else {
            None
        };

        // Total display count: pending (if any) + db sessions
        let display_offset = pending_idx.map_or(0, |_| 1);
        let display_count = display_offset + self.sessions.len();
        let selected = self.selected.min(display_count.saturating_sub(1));

        let mut list_state = ListState::default();
        list_state.select(Some(selected));

        let mut items: Vec<ListItem> = Vec::new();

        // Pending session entry
        if let (Some(_), Some(p)) = (pending_idx, &self.pending_session) {
            let is_sel = selected == 0;
            let name_style = if is_sel {
                Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  ◌ ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled(&p.agent_id, Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" · ", Style::default().fg(Theme::SURFACE1)),
                Span::styled("tui:default", name_style),
                Span::styled("  new", Style::default().fg(Theme::OVERLAY0)),
            ])));
        }

        // DB sessions
        for (i, s) in self.sessions.iter().enumerate() {
            let display_i = i + display_offset;
            let (icon, icon_color) = match s.state.as_str() {
                "active" => ("●", Theme::GREEN),
                "idle" => ("●", Theme::YELLOW),
                "suspended" => ("○", Theme::OVERLAY1),
                _ => ("○", Theme::SURFACE2),
            };
            let display = format!("{}:{}", s.origin, s.context_id);
            let name_style = if display_i == selected {
                Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(icon_color)),
                Span::styled(&s.agent_id, Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" · ", Style::default().fg(Theme::SURFACE1)),
                Span::styled(display, name_style),
                Span::styled(
                    format!("  {}", format_timestamp(&s.last_activity_at)),
                    Style::default().fg(Theme::SURFACE2),
                ),
            ])));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Theme::SURFACE1))
                    .title(" Sessions ")
                    .title_style(Style::default().fg(Theme::MAUVE)),
            )
            .highlight_style(Style::default().bg(Theme::SURFACE0));

        frame.render_stateful_widget(list, rows[0], &mut list_state);

        // Help bar
        let help = Line::from(vec![
            Span::styled(" j/k", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Navigate  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("Enter", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Chat  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("n", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" New  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("d", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Delete  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("r", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Refresh", Style::default().fg(Theme::OVERLAY0)),
        ]);
        frame.render_widget(
            Paragraph::new(help).style(Style::default().bg(Theme::MANTLE)),
            rows[1],
        );
    }

    fn render_chat_view(&mut self, frame: &mut Frame, area: Rect) {
        let has_approval = !self.pending_approvals.is_empty();
        // Layout: [chat messages] [approval inline?] [input line] [help bar]
        let approval_height = if has_approval { 7u16 } else { 0u16 };
        let input_height = 3u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),                   // Chat
                Constraint::Length(approval_height),   // Inline approval
                Constraint::Length(input_height),      // Input
                Constraint::Length(1),                 // Help
            ])
            .split(area);

        let chat_area = chunks[0];
        let approval_area = chunks[1];
        let input_area = chunks[2];
        let help_area = chunks[3];

        // ── Chat area ──
        let agent_name = if let Some(p) = &self.pending_session {
            p.agent_id.clone()
        } else {
            self.sessions.get(self.selected)
                .map(|s| s.agent_id.clone())
                .unwrap_or_else(|| "chat".to_string())
        };

        let model_label = self.current_session_model()
            .map(|m| format!(" [{}]", models::model_display_name(&m)))
            .unwrap_or_default();

        let chat_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Theme::SURFACE1))
            .title(format!(" {}{} ", agent_name, model_label))
            .title_style(Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD));

        let inner = chat_block.inner(chat_area);
        frame.render_widget(chat_block, chat_area);

        if self.messages.is_empty() && !self.loading {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Start a conversation.",
                    Style::default().fg(Theme::OVERLAY0),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Type below and press Enter to send.",
                    Style::default().fg(Theme::SURFACE2),
                )),
            ]);
            frame.render_widget(empty, inner);
        } else {
            let elapsed_secs = self.loading_since
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            chat::render_chat(
                frame, inner, &self.messages,
                self.loading, &mut self.chat_scroll, self.tick, elapsed_secs,
            );
        }

        // ── Input area ──
        let input_border = if self.loading {
            Style::default().fg(Theme::SURFACE2)
        } else {
            Style::default().fg(Theme::MAUVE)
        };

        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(input_border)
                .title(if self.loading { " Waiting... " } else { " > " })
                .title_style(input_border)
                .padding(Padding::horizontal(1)),
        );

        frame.render_widget(&self.textarea, input_area);

        // ── Slash completion popup (above input area) ──
        if !self.slash_completions.is_empty() {
            let display_count = self.slash_completions.len().min(5) as u16;
            let popup_height = display_count + 2; // +2 for border
            // Position popup above the input area
            if chat_area.height > popup_height {
                let popup_area = Rect {
                    x: input_area.x + 1,
                    y: input_area.y.saturating_sub(popup_height),
                    width: input_area.width.min(50),
                    height: popup_height,
                };

                // Clear background
                frame.render_widget(Clear, popup_area);

                let items: Vec<ListItem> = self.slash_completions
                    .iter()
                    .enumerate()
                    .take(5)
                    .map(|(i, item)| {
                        let is_selected = i == self.slash_idx;
                        let (prefix, style) = if is_selected {
                            (
                                "  ▸ ",
                                Style::default()
                                    .fg(Theme::MAUVE)
                                    .bg(Theme::SURFACE0)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("    ", Style::default().fg(Theme::SUBTEXT0))
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(prefix, style),
                            Span::styled(item.as_str(), style),
                        ]))
                    })
                    .collect();

                let list = List::new(items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(Theme::MAUVE))
                        .title(" Completions ")
                        .title_style(Style::default().fg(Theme::MAUVE)),
                );
                frame.render_widget(list, popup_area);
            }
        }

        // ── Inline approval ──
        if has_approval {
            let item = &self.pending_approvals[self.approval_selected];
            let input_short = serde_json::to_string(&item.tool_input).unwrap_or_default();
            let input_display = if input_short.chars().count() > 80 {
                format!("{}…", input_short.chars().take(80).collect::<String>())
            } else {
                input_short
            };
            let remaining = item.expires_secs.saturating_sub(item.received_at.elapsed().as_secs());
            let count_str = if self.pending_approvals.len() > 1 {
                format!(" ({}/{})", self.approval_selected + 1, self.pending_approvals.len())
            } else {
                String::new()
            };

            let approve_style = if self.approval_choice == 0 {
                Style::default().fg(Theme::BASE).bg(Theme::GREEN).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::GREEN)
            };
            let deny_style = if self.approval_choice == 1 {
                Style::default().fg(Theme::BASE).bg(Theme::RED).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::RED)
            };

            let approval_widget = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(" ⚠ Approval Required", Style::default().fg(Theme::YELLOW).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{} — expires in {}s", count_str, remaining), Style::default().fg(Theme::OVERLAY1)),
                ]),
                Line::from(vec![
                    Span::styled("  Tool: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(&item.tool_name, Style::default().fg(Theme::PEACH).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("  Input: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(input_display, Style::default().fg(Theme::SUBTEXT0)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(" ✅ Approve ", approve_style),
                    Span::styled("   ", Style::default()),
                    Span::styled(" ❌ Deny ", deny_style),
                    Span::styled("    ←→ Select  Enter Confirm", Style::default().fg(Theme::OVERLAY0)),
                ]),
            ]).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Theme::YELLOW))
            ).style(Style::default().bg(Theme::SURFACE0));
            frame.render_widget(approval_widget, approval_area);
        }

        // ── Help bar ──
        let mut help_spans = vec![
            Span::styled(" Enter", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Send  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("⇧Enter", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Newline  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("Fn↑↓", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Scroll  ", Style::default().fg(Theme::OVERLAY0)),
            Span::styled("Esc", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
            Span::styled(" Back", Style::default().fg(Theme::OVERLAY0)),
        ];
        if self.loading {
            help_spans.push(Span::styled("  ⌃K", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)));
            help_spans.push(Span::styled(" Stop", Style::default().fg(Theme::OVERLAY0)));
        }
        let help = Line::from(help_spans);
        frame.render_widget(
            Paragraph::new(help).style(Style::default().bg(Theme::MANTLE)),
            help_area,
        );
    }
}
