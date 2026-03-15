pub mod claude;
pub mod manager;
pub mod queue;
pub mod transcript;

use std::fmt;

/// Unique key for a session: catclaw:{agent_id}:{origin}:{context_id}
///
/// | origin    | context_id examples                        |
/// |-----------|--------------------------------------------|
/// | tui       | "default"                                  |
/// | webui     | "default" or uuid                          |
/// | discord   | channel name, or "dm.username"             |
/// | telegram  | chat title, or "dm.username"               |
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub agent_id: String,
    pub origin: String,
    pub context_id: String,
}

impl SessionKey {
    pub fn new(
        agent_id: impl Into<String>,
        origin: impl Into<String>,
        context_id: impl Into<String>,
    ) -> Self {
        SessionKey {
            agent_id: agent_id.into(),
            origin: origin.into(),
            context_id: context_id.into(),
        }
    }

    /// Format as a string key for DB storage
    pub fn to_key_string(&self) -> String {
        format!(
            "catclaw:{}:{}:{}",
            self.agent_id, self.origin, self.context_id
        )
    }

    /// Parse a key string back into a SessionKey.
    /// Format: `catclaw:{agent_id}:{origin}:{context_id}`
    /// The context_id may contain `:` so we only split on the first 3 `:` delimiters.
    pub fn from_key_string(s: &str) -> Option<SessionKey> {
        let raw = s.strip_prefix("catclaw:").unwrap_or(s);
        let mut parts = raw.splitn(3, ':');
        let agent_id = parts.next()?.to_string();
        let origin = parts.next()?.to_string();
        let context_id = parts.next()?.to_string();
        if agent_id.is_empty() || origin.is_empty() || context_id.is_empty() {
            return None;
        }
        Some(SessionKey {
            agent_id,
            origin,
            context_id,
        })
    }
}

impl fmt::Display for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_key_string())
    }
}

/// Session lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SessionState {
    Active,
    Idle,
    Suspended,
    Archived,
}

#[allow(dead_code)]
impl SessionState {
    pub fn as_str(&self) -> &str {
        match self {
            SessionState::Active => "active",
            SessionState::Idle => "idle",
            SessionState::Suspended => "suspended",
            SessionState::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => SessionState::Active,
            "idle" => SessionState::Idle,
            "suspended" => SessionState::Suspended,
            "archived" => SessionState::Archived,
            _ => SessionState::Suspended,
        }
    }
}

/// Priority levels for the session queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum Priority {
    /// Direct messages — highest priority
    Direct = 4,
    /// Mentions (@CatClaw)
    Mention = 3,
    /// Regular channel messages
    Channel = 2,
    /// Heartbeat tasks
    Heartbeat = 1,
    /// Cron/scheduled tasks — lowest priority
    Cron = 0,
}

/// Events emitted during streaming response from a session.
/// Only used by TUI/WebUI streaming mode.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SessionEvent {
    /// Incremental text token
    TextDelta { text: String },
    /// Tool invocation
    ToolUse { name: String, input: serde_json::Value },
    /// Final complete response
    Complete { text: String },
    /// Error during session
    Error { message: String },
}
