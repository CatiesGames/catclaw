/// Shared types for the tool approval system.
///
/// Flow:
///   claude subprocess → PreToolUse hook (catclaw hook pre-tool)
///     → gateway WS: approval.request
///       → broadcast approval.pending to all TUI clients
///       → optionally notify originating channel (Discord/Telegram)
///     ← user responds via TUI or channel: approval.respond
///   hook receives approval.result push event → exits 0 (allow) or 2 (block)
///
/// For codex agents the flow has no hook subprocess — `mcp_server.rs::
/// handle_codex_tool_call` constructs the same [`PendingApproval`] directly
/// in-process, then waits on the same `oneshot::Receiver<ApprovalDecision>`
/// the hook waits on, so the channel-adapter / TUI surface is identical
/// across runtimes (codex-runtime-plan.md §3.3).
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
    /// Agent id resolved at request creation time. None for legacy hook
    /// callers that don't carry an explicit agent (Phase A pre-existing
    /// approvals were always implicit on the session-key path).
    #[allow(dead_code)] // wired in B.4.2 (handle_codex_tool_call → channel renderers)
    pub agent_id: Option<String>,
    /// Codex `turn_id` from `_meta.x-codex-turn-metadata`. None for Claude
    /// (the hook path doesn't carry a per-turn id; the Claude tool-use id
    /// is `_meta.claudecode/toolUseId` but is unused server-side).
    #[allow(dead_code)] // wired in B.4.2 (handle_codex_tool_call → channel renderers)
    pub turn_id: Option<String>,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub created_at: std::time::Instant,
    /// 3-way decision (codex-runtime-plan.md §3.3 / B.3.1):
    ///   Approved          — let the tool run
    ///   Denied { reason } — admin rejected, optionally with a reason
    ///   Timeout           — no admin acted within `timeout_secs`
    pub response_tx: tokio::sync::oneshot::Sender<ApprovalDecision>,
}

/// 3-way result of an approval decision. Replaces the historical
/// `oneshot::Sender<bool>` so deny-with-reason and timeout are distinguishable
/// downstream (cmd_hook surfaces different messages to claude/codex).
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
    Timeout,
}

impl ApprovalDecision {
    /// Backwards-compatible "is allowed" check — true only for Approved.
    /// Used in places that previously read a plain bool.
    pub fn is_approved(&self) -> bool {
        matches!(self, ApprovalDecision::Approved)
    }

    /// Wire-format discriminator string for [`ApprovalResultEvent::decision`].
    /// Keeps the wire schema stable: `approved` | `denied` | `timeout`.
    pub fn as_wire_str(&self) -> &'static str {
        match self {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Denied { .. } => "denied",
            ApprovalDecision::Timeout => "timeout",
        }
    }

    /// Extract a denial reason if any. None for Approved / Timeout / Denied
    /// without a reason.
    pub fn reason(&self) -> Option<&str> {
        match self {
            ApprovalDecision::Denied { reason } => reason.as_deref(),
            _ => None,
        }
    }
}

/// The approval.pending event data broadcast to TUI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalPendingEvent {
    pub request_id: String,
    pub session_key: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub expires_secs: u64,
    /// Resolved agent id when the gateway knew it at request time. Wire-level
    /// optional so old clients ignore it; new TUI/channel renderers display
    /// "Agent: foo" when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Codex turn_id when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// The approval.result event sent back to the waiting hook.
///
/// Wire stability rule: `approved: bool` is **never removed** even after
/// `decision` lands. cmd_hook.rs (separate binary, may be older than the
/// gateway briefly during deploy) still reads `approved`. New clients
/// (gateway internal, TUI) read `decision` for the 3-way distinction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalResultEvent {
    pub request_id: String,
    pub approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Discriminator added in B.3.1 — `approved` | `denied` | `timeout`.
    /// Optional for back-compat; if absent, derive from `approved` bool
    /// (true → approved; false → denied — timeout collapses into denied
    /// for old clients, which is the conservative choice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

impl ApprovalResultEvent {
    /// Build from an [`ApprovalDecision`] preserving the optional reason.
    pub fn from_decision(request_id: String, decision: &ApprovalDecision) -> Self {
        ApprovalResultEvent {
            request_id,
            approved: decision.is_approved(),
            reason: decision.reason().map(String::from),
            decision: Some(decision.as_wire_str().to_string()),
        }
    }
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

/// Tagged enum describing the three kinds of approval card the gateway can
/// render. Channel adapters (Discord/Telegram/Slack/LINE) and the TUI render
/// each variant with the appropriate fields — buttons share the same callback
/// IDs (`approve:`/`deny:`/`revise:` prefix + draft_id or request_id).
///
/// `Tool` is the synchronous-blocking kind backed by [`PendingApproval`].
/// `SocialPost` and `ContactReply` are the async-draft kinds backed by
/// `social_drafts` / `contact_drafts` DB tables; for those the gateway
/// returns immediately and the draft sits in the table until an admin acts.
///
/// codex-runtime-plan.md §3.2 — the UI is unified across runtimes; the
/// time-model (blocking vs draft) is the distinguishing axis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(dead_code)] // wired in B.4.x / B.5.x
pub enum ApprovalCard {
    /// Tool call awaiting admin approval (synchronous-blocking model).
    Tool {
        approval_id: String,
        agent_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// Social-media draft awaiting admin review (async-draft model).
    SocialPost {
        draft_id: String,
        agent_id: String,
        /// "instagram" | "threads"
        platform: String,
        caption_preview: String,
        media_count: u32,
        /// URLs admin can preview before approving. Channel adapters may use
        /// these as embed thumbnails / inline image previews.
        #[serde(default)]
        media_urls: Vec<String>,
    },
    /// Reply to a bound contact awaiting admin review (async-draft model).
    ContactReply {
        draft_id: String,
        agent_id: String,
        contact_id: String,
        contact_display_name: String,
        /// Platform of the destination channel (discord/telegram/line/slack).
        platform: String,
        body_preview: String,
    },
}
