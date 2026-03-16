/// Shared types for the tool approval system.
///
/// Flow:
///   claude subprocess → PreToolUse hook (catclaw hook pre-tool)
///     → gateway WS: approval.request
///       → broadcast approval.pending to all TUI clients
///       → optionally notify originating channel (Discord/Telegram)
///     ← user responds via TUI or channel: approval.respond
///   hook receives approval.result push event → exits 0 (allow) or 2 (block)
use serde::{Deserialize, Serialize};

/// Sent from the hook subprocess to the gateway via `approval.request`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub session_key: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
}

/// Stored in the gateway while waiting for a user decision.
pub struct PendingApproval {
    pub request_id: String,
    pub session_key: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub created_at: std::time::Instant,
    /// Sending `true` allows, `false` blocks.
    pub response_tx: tokio::sync::oneshot::Sender<bool>,
}

/// The approval.pending event data broadcast to TUI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPendingEvent {
    pub request_id: String,
    pub session_key: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub expires_secs: u64,
}

/// The approval.result event sent back to the waiting hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResultEvent {
    pub request_id: String,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Stdin JSON received by the hook from Claude Code.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    pub session_id: String,
}
