//! Runtime abstraction layer for multi-runtime agent support.
//!
//! `RuntimeHandle` is an enum dispatch over the two supported CLIs:
//! - `Claude` (claude code CLI via [`ClaudeHandle`])
//! - `Codex`  (codex CLI via [`CodexHandle`])
//!
//! `RuntimeEvent` is the unified event stream; both backends map their native
//! NDJSON output into this shape. Phase A only implements the Claude path —
//! the Codex variants are stubs that will be filled in during Phase B.

use std::collections::HashMap;
use std::path::Path;
use tokio::sync::mpsc::UnboundedSender;

use super::claude::{ClaudeEvent, ClaudeHandle, ContentBlock};
use crate::error::Result;
use crate::state::StateDb;

/// Enum dispatch over the two supported agent runtimes.
///
/// Using a concrete enum (vs `Box<dyn ...>`) keeps the hot-path event loop
/// free of trait-object overhead and lets the compiler verify exhaustiveness
/// when adding variants.
#[allow(dead_code)]
#[allow(clippy::large_enum_variant)] // Codex variant is a placeholder until Phase B;
// once CodexHandle lands it will be similarly sized to ClaudeHandle.
pub enum RuntimeHandle {
    Claude(ClaudeHandle),
    // Codex variant added in Phase B (src/session/codex.rs::CodexHandle).
    // Holding the slot here so call-sites can match exhaustively from Phase A
    // without changing shape later.
    #[allow(dead_code)]
    Codex(/* CodexHandle */ ()),
}

#[allow(dead_code)]
impl RuntimeHandle {
    pub async fn recv_event(&mut self) -> Option<RuntimeEvent> {
        match self {
            RuntimeHandle::Claude(h) => h.recv_event().await.map(RuntimeEvent::from),
            RuntimeHandle::Codex(_) => unimplemented!("CodexHandle::recv_event in Phase B"),
        }
    }

    pub async fn wait_for_result(
        &mut self,
        observer: Option<UnboundedSender<RuntimeEvent>>,
    ) -> Result<String> {
        match self {
            RuntimeHandle::Claude(h) => {
                // ClaudeHandle::wait_for_result takes an Option<UnboundedSender<ClaudeEvent>>.
                // Wrap the caller's RuntimeEvent observer (if any) in a tee that converts
                // each ClaudeEvent to RuntimeEvent before forwarding.
                let claude_observer = observer.map(|runtime_tx| {
                    let (claude_tx, mut claude_rx) =
                        tokio::sync::mpsc::unbounded_channel::<ClaudeEvent>();
                    tokio::spawn(async move {
                        while let Some(ev) = claude_rx.recv().await {
                            if runtime_tx.send(RuntimeEvent::from(ev)).is_err() {
                                break;
                            }
                        }
                    });
                    claude_tx
                });
                h.wait_for_result(claude_observer).await
            }
            RuntimeHandle::Codex(_) => unimplemented!("CodexHandle::wait_for_result in Phase B"),
        }
    }

    pub async fn kill(&mut self) -> Result<()> {
        match self {
            RuntimeHandle::Claude(h) => h.kill().await,
            RuntimeHandle::Codex(_) => unimplemented!("CodexHandle::kill in Phase B"),
        }
    }

    pub fn is_running(&mut self) -> bool {
        match self {
            RuntimeHandle::Claude(h) => h.is_running(),
            RuntimeHandle::Codex(_) => unimplemented!("CodexHandle::is_running in Phase B"),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            RuntimeHandle::Claude(h) => h.session_id.as_deref(),
            RuntimeHandle::Codex(_) => unimplemented!("CodexHandle::session_id in Phase B"),
        }
    }
}

/// Unified event stream surfaced by `RuntimeHandle::recv_event`.
///
/// Maps both Claude's NDJSON and Codex's NDJSON into the same shape so the
/// rest of the gateway (transcript writer, streaming observers, channel
/// adapters) does not need to know which backend produced the event.
///
/// `ToolResult` is codex-only — Claude's stream-json output has no
/// corresponding event. We accept this transcript asymmetry by design
/// (see codex-runtime-plan.md §1.7).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum RuntimeEvent {
    SystemInit {
        session_id: String,
    },
    Assistant {
        content: Vec<ContentBlock>,
    },
    Result {
        result: String,
        session_id: String,
    },
    TextDelta {
        text: String,
    },
    ToolUseStart {
        name: String,
        input: serde_json::Value,
    },
    /// Codex-only: full tool result from `item.completed` event.
    /// ClaudeHandle never emits this variant.
    ToolResult {
        name: String,
        output: serde_json::Value,
        is_error: bool,
    },
    /// Raw streaming event we don't specifically parse (Claude `stream_event`
    /// passthrough; codex doesn't currently produce events landing here).
    StreamEvent {
        event: serde_json::Value,
    },
    /// Anything else — kept as raw JSON for forward compatibility.
    Unknown(serde_json::Value),
}

impl From<ClaudeEvent> for RuntimeEvent {
    fn from(e: ClaudeEvent) -> Self {
        match e {
            ClaudeEvent::SystemInit { session_id } => RuntimeEvent::SystemInit { session_id },
            ClaudeEvent::Assistant { content } => RuntimeEvent::Assistant { content },
            ClaudeEvent::Result { result, session_id } => {
                RuntimeEvent::Result { result, session_id }
            }
            ClaudeEvent::TextDelta { text } => RuntimeEvent::TextDelta { text },
            ClaudeEvent::ToolUseStart { name, input } => RuntimeEvent::ToolUseStart { name, input },
            ClaudeEvent::StreamEvent { event } => RuntimeEvent::StreamEvent { event },
            ClaudeEvent::Unknown(value) => RuntimeEvent::Unknown(value),
        }
    }
}

/// Parameters shared by all session spawns. Some fields are runtime-specific
/// and ignored by the other backend — that's intentional, the per-runtime
/// args builder reads what it needs and skips the rest.
#[allow(dead_code)]
pub struct SpawnParams<'a> {
    pub session_id: &'a str,
    pub model_override: Option<&'a str>,
    pub mcp_port: Option<u16>,
    /// Claude-only: hook session key for `--settings` PreToolUse injection.
    pub hook_session_key: Option<&'a str>,
    /// Claude-only: path to catclaw.toml for hook subprocess.
    pub config_path: Option<&'a Path>,
    pub mcp_env: &'a HashMap<String, HashMap<String, String>>,
    pub state_db: Option<&'a StateDb>,
    pub is_resume: bool,
    /// Codex-only: thread_id from a previous codex session, used on resume.
    /// For Claude resume, `session_id` itself is used with `--resume`.
    pub resume_thread_id: Option<&'a str>,
}
