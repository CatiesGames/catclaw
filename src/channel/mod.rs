pub mod discord;
pub mod reaction;
pub mod slack;
pub mod telegram;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Channel type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Discord,
    Telegram,
    Slack,
    Tui,
}

impl ChannelType {
    pub fn as_str(&self) -> &str {
        match self {
            ChannelType::Discord => "discord",
            ChannelType::Telegram => "telegram",
            ChannelType::Slack => "slack",
            ChannelType::Tui => "tui",
        }
    }
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Standardized inbound message from any channel
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MsgContext {
    pub channel_type: ChannelType,
    pub channel_id: String,
    pub peer_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub reply_to: Option<ReplyContext>,
    pub thread_id: Option<String>,
    pub is_direct_message: bool,
    pub raw_event: serde_json::Value,
    /// Human-readable channel name (e.g. Discord channel name, Telegram chat title).
    /// Used by router to build context_id for session keys.
    pub channel_name: Option<String>,
    /// Guild/server ID (Discord guild, etc.). Used for MCP tool context.
    pub guild_id: Option<String>,
    /// Platform message ID of the user's original message. Used for reaction status.
    pub message_id: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Attachment {
    pub filename: String,
    pub url: String,
    pub content_type: Option<String>,
    /// File size in bytes (if available from the platform).
    pub size: Option<u64>,
    /// Authorization header value for platforms that require auth to download (e.g. Slack).
    pub auth_header: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ReplyContext {
    pub message_id: String,
    pub text: Option<String>,
}

/// Outbound message to send via a channel
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OutboundMessage {
    pub channel_type: ChannelType,
    pub channel_id: String,
    pub peer_id: String,
    pub text: String,
    pub thread_id: Option<String>,
    pub reply_to_message_id: Option<String>,
}

/// What features a channel adapter supports
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChannelCapabilities {
    pub threading: bool,
    pub typing_indicator: bool,
    pub message_editing: bool,
    pub max_message_length: usize,
    pub attachments: bool,
    /// Whether the adapter supports native streaming (e.g. Slack AI streaming API).
    pub streaming: bool,
}

/// Guard that stops typing indicator when dropped
pub struct TypingGuard {
    _cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TypingGuard {
    pub fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        TypingGuard {
            _cancel: Some(cancel),
        }
    }

    pub fn noop() -> Self {
        TypingGuard { _cancel: None }
    }
}

/// Describes a platform-specific action that an adapter supports.
/// Used by the MCP server to generate tool schemas.
#[derive(Debug, Clone)]
pub struct ActionInfo {
    pub name: String,
    pub description: String,
    pub params_schema: serde_json::Value,
}

/// Trait that all channel adapters must implement
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Start the adapter, sending inbound messages to msg_tx
    async fn start(&self, msg_tx: tokio::sync::mpsc::Sender<MsgContext>) -> Result<()>;

    /// Send an outbound message
    async fn send(&self, msg: OutboundMessage) -> Result<()>;

    /// Start a typing indicator (returns guard that stops on drop)
    async fn start_typing(&self, channel_id: &str, peer_id: &str) -> Result<TypingGuard>;

    /// Create a thread in a channel
    #[allow(dead_code)]
    async fn create_thread(&self, channel_id: &str, title: &str) -> Result<String>;

    /// Adapter name
    fn name(&self) -> &str;

    /// Supported capabilities
    fn capabilities(&self) -> ChannelCapabilities;

    /// Send an approval request to the channel.
    /// Default implementation sends a plain-text fallback message.
    async fn send_approval(
        &self,
        channel_id: &str,
        peer_id: &str,
        thread_id: Option<&str>,
        request_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> Result<()> {
        let input_preview = serde_json::to_string_pretty(tool_input)
            .unwrap_or_else(|_| tool_input.to_string());
        let text = format!(
            "🔒 Approval Required\nTool: `{}`\n```json\n{}\n```\nReply 'approve {}' or 'deny {}'",
            tool_name, input_preview, request_id, request_id
        );
        self.send(OutboundMessage {
            channel_type: ChannelType::Tui, // placeholder, adapter ignores this field
            channel_id: channel_id.to_string(),
            peer_id: peer_id.to_string(),
            text,
            thread_id: thread_id.map(String::from),
            reply_to_message_id: None,
        })
        .await
    }

    /// Start a native streaming message (for adapters that support it).
    /// Returns the message timestamp/ID that subsequent append/stop calls reference.
    #[allow(dead_code)]
    async fn send_stream_start(&self, _channel_id: &str, _thread_ts: &str) -> Result<String> {
        Err(crate::error::CatClawError::Channel(
            "streaming not supported by this adapter".into(),
        ))
    }

    /// Append text to an ongoing stream.
    #[allow(dead_code)]
    async fn send_stream_append(&self, _msg_ts: &str, _channel_id: &str, _text: &str) -> Result<()> {
        Err(crate::error::CatClawError::Channel(
            "streaming not supported by this adapter".into(),
        ))
    }

    /// Stop (finalize) a streaming message. Optional final text replaces the full message body.
    #[allow(dead_code)]
    async fn send_stream_stop(&self, _msg_ts: &str, _channel_id: &str, _text: Option<&str>) -> Result<()> {
        Err(crate::error::CatClawError::Channel(
            "streaming not supported by this adapter".into(),
        ))
    }

    /// Execute a platform-specific action (for MCP tool calls).
    /// `action`: operation name (e.g. "get_messages", "create_channel")
    /// `params`: JSON parameters
    /// Returns JSON result.
    async fn execute(&self, action: &str, _params: serde_json::Value) -> Result<serde_json::Value> {
        Err(crate::error::CatClawError::Channel(format!(
            "action '{}' not supported by {}",
            action,
            self.name()
        )))
    }

    /// List actions this adapter supports (MCP server uses this to generate tool schemas).
    fn supported_actions(&self) -> Vec<ActionInfo> {
        vec![]
    }

    /// Create a reaction status controller for the given message.
    /// Only Discord implements this; other adapters return None.
    async fn create_reaction_handle(
        &self,
        _channel_id: &str,
        _message_id: &str,
    ) -> Option<reaction::ReactionHandle> {
        None
    }

    /// Platform name used to match admin_channel config (e.g. "discord", "telegram", "slack").
    fn platform_name(&self) -> &str {
        self.name()
    }

    /// Send a Social Inbox forward/draft card with action buttons.
    /// Returns the platform message ID (for tracking button responses), or None if unsupported.
    async fn send_social_card(
        &self,
        channel_id: &str,
        card: &crate::social::forward::ForwardCard,
    ) -> crate::error::Result<Option<String>> {
        // Default: send plain text fallback without buttons.
        // channel_type is a placeholder — concrete adapters ignore this field (see send_approval).
        let text = format!("[{}] @{}: {}", card.title, card.author, card.text);
        self.send(OutboundMessage {
            channel_type: ChannelType::Tui, // placeholder, adapter ignores
            channel_id: channel_id.to_string(),
            peer_id: String::new(),
            text,
            thread_id: None,
            reply_to_message_id: None,
        })
        .await?;
        Ok(None)
    }
}

/// Hot-reloadable filter settings shared between adapter and gateway.
/// Wrapped in `Arc<std::sync::RwLock<AdapterFilter>>` so gateway can update
/// and handler can read without restart.
#[derive(Debug, Clone)]
pub struct AdapterFilter {
    pub activation: String,
    pub overrides: Vec<(String, String)>,
    pub guilds: Vec<u64>,
    pub dm_policy: String,
    pub dm_allow: Vec<String>,
    pub dm_deny: Vec<String>,
    pub group_policy: String,
    pub group_allow: Vec<String>,
    pub group_deny: Vec<String>,
}

impl AdapterFilter {
    pub fn from_config(config: &crate::config::ChannelConfig) -> Self {
        let guilds: Vec<u64> = config
            .guilds
            .iter()
            .filter_map(|g| g.parse().ok())
            .collect();
        let overrides: Vec<(String, String)> = config
            .overrides
            .iter()
            .map(|o| (o.pattern.clone(), o.activation.clone()))
            .collect();
        AdapterFilter {
            activation: config.activation.clone(),
            overrides,
            guilds,
            dm_policy: config.dm_policy.clone(),
            dm_allow: config.dm_allow.clone(),
            dm_deny: config.dm_deny.clone(),
            group_policy: config.group_policy.clone(),
            group_allow: config.group_allow.clone(),
            group_deny: config.group_deny.clone(),
        }
    }

    /// Determine activation mode for a given channel (checks overrides first).
    pub fn activation_for(&self, prefix: &str, channel_id: &str) -> &str {
        self.overrides
            .iter()
            .find(|(pattern, _)| pattern == &format!("{}:{}", prefix, channel_id))
            .map(|(_, act)| act.as_str())
            .unwrap_or(&self.activation)
    }

    /// Check whether a sender is allowed based on DM/group policy.
    /// Returns true if the message should be processed.
    pub fn is_sender_allowed(&self, is_dm: bool, sender_id: &str) -> bool {
        if is_dm {
            // Deny list always wins
            if self.dm_deny.iter().any(|id| id == sender_id) {
                return false;
            }
            match self.dm_policy.as_str() {
                "disabled" => false,
                "allowlist" => self.dm_allow.iter().any(|id| id == sender_id),
                _ => true, // "open"
            }
        } else {
            if self.group_deny.iter().any(|id| id == sender_id) {
                return false;
            }
            match self.group_policy.as_str() {
                "allowlist" => self.group_allow.iter().any(|id| id == sender_id),
                _ => true, // "open"
            }
        }
    }
}

/// Split text at natural boundaries to fit within max length
pub fn split_at_boundaries(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining);
            break;
        }

        // Find a good split point: prefer double newline, then single newline, then space
        let search_end = floor_char_boundary(remaining, max_len);
        let split_at = remaining[..search_end]
            .rfind("\n\n")
            .map(|i| i + 2)
            .or_else(|| remaining[..search_end].rfind('\n').map(|i| i + 1))
            .or_else(|| remaining[..search_end].rfind(' ').map(|i| i + 1))
            .unwrap_or(search_end);

        chunks.push(&remaining[..split_at]);
        remaining = &remaining[split_at..];
    }

    chunks
}

/// Find the largest byte index `<= n` that is a valid UTF-8 char boundary.
/// Prevents panics when slicing multi-byte characters (CJK, emoji, etc.).
fn floor_char_boundary(s: &str, n: usize) -> usize {
    if n >= s.len() {
        return s.len();
    }
    let mut i = n;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
