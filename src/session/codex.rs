//! Handle to a running `codex exec` subprocess.
//!
//! Mirrors [`ClaudeHandle`](super::claude::ClaudeHandle) — same lifecycle,
//! same event-channel pattern, same `wait_for_result` semantics. The on-wire
//! NDJSON format from codex is different though, so this file owns the
//! Codex-specific event parser.
//!
//! Codex event shape (from `codex exec --json`):
//!   - `{"type":"thread.started","thread_id":"..."}`
//!     — emitted once at startup, equivalent to Claude's `system/init`.
//!   - `{"type":"turn.started"}` — internal marker, mapped to a synthetic
//!     `Unknown` event so the manager can ignore it.
//!   - `{"type":"item.started","item":{...}}` — tool call beginning. The
//!     `item.type` discriminates: `command_execution` is codex's native shell;
//!     `mcp_tool_call` is an MCP tool (we surface as `mcp__{server}__{tool}`
//!     so transcripts read consistently with Claude's tool naming).
//!   - `{"type":"item.completed","item":{...}}` — completion of either an
//!     agent message (final text), a shell command, or an MCP tool call.
//!   - `{"type":"turn.completed","usage":{...}}` — end of one turn,
//!     mapped to `Result` with the accumulated last `agent_message`.
//!   - `{"type":"turn.failed","error":{...}}` — turn failed; mapped to
//!     `Result` with the error message so the manager surfaces it.
//!
//! The parser is intentionally lenient — any unrecognised shape falls through
//! to `Unknown(value)` so codex schema changes don't crash the gateway.

use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::runtime::RuntimeEvent;
use crate::error::{CatClawError, Result};

/// Handle to a running `codex exec` subprocess. See module docs.
#[allow(dead_code)]
pub struct CodexHandle {
    child: Child,
    /// Open stdin handle for `codex exec resume … -` mode where the prompt
    /// arrives on stdin. None for first-turn spawns where the prompt was
    /// passed as a CLI argument (stdin was nulled to avoid the "Reading
    /// additional input from stdin..." stall that codex exhibits otherwise).
    stdin: Option<tokio::process::ChildStdin>,
    rx: mpsc::Receiver<RuntimeEvent>,
    /// Cached thread_id from the most recent `thread.started` event. Codex's
    /// thread_id is the session identifier (used for `codex exec resume`).
    pub session_id: Option<String>,
    /// Accumulator for the final agent message text — codex doesn't emit a
    /// single "result" event, we synthesise one from the last
    /// `item.completed` of type `agent_message` followed by `turn.completed`.
    last_agent_message: String,
}

impl CodexHandle {
    /// Spawn a fresh codex session (`codex exec --json … "PROMPT"`).
    ///
    /// The prompt is passed as a CLI argument and stdin is nulled, matching
    /// what the [`Phase B.1.2` PoC](../../tasks/codex-runtime-plan.md) verified
    /// works without the "Reading additional input from stdin..." stall.
    #[allow(dead_code)]
    pub async fn spawn_with_prompt(
        mut args: Vec<String>,
        prompt: &str,
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        // Prompt is the last positional arg after all `-c key=value` flags.
        args.push(prompt.to_string());
        Self::spawn_inner(args, env, /* stdin_piped = */ false).await
    }

    /// Resume an existing codex thread. The caller already includes
    /// `exec resume <thread_id>` + flags + a trailing `-` sentinel in `args`;
    /// stdin is piped and the prompt is written into it.
    ///
    /// We use the `-` sentinel form rather than the prompt-as-arg form so that
    /// a resume can land arbitrary multi-line content on stdin without shell
    /// escaping (matches `codex exec resume <id> -` documented behaviour).
    #[allow(dead_code)]
    pub async fn spawn_resume_with_prompt(
        args: Vec<String>,
        prompt: &str,
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut handle = Self::spawn_inner(args, env, /* stdin_piped = */ true).await?;
        if let Some(stdin) = handle.stdin.as_mut() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| CatClawError::Claude(format!("codex stdin write failed: {}", e)))?;
            // Codex reads until EOF when invoked with `-`; close stdin to signal end.
            stdin
                .shutdown()
                .await
                .map_err(|e| CatClawError::Claude(format!("codex stdin shutdown failed: {}", e)))?;
        }
        handle.stdin = None;
        Ok(handle)
    }

    async fn spawn_inner(
        args: Vec<String>,
        env: &HashMap<String, String>,
        stdin_piped: bool,
    ) -> Result<Self> {
        info!(args = ?args, "spawning codex process");

        let mut cmd = Command::new("codex");
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(env);

        if stdin_piped {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| CatClawError::Claude(format!("failed to spawn codex: {}", e)))?;

        let stdin = if stdin_piped { child.stdin.take() } else { None };

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture codex stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture codex stderr".to_string()))?;

        let (tx, rx) = mpsc::channel(256);

        // stdout reader: parse NDJSON → RuntimeEvent
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                debug!(line = %line, "codex stdout");
                match serde_json::from_str::<Value>(&line) {
                    Ok(value) => {
                        let event = parse_codex_event(&value);
                        if tx.send(event).await.is_err() {
                            warn!("codex event channel closed, stopping stdout reader");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, line = %line, "failed to parse codex JSON line");
                    }
                }
            }
            info!("codex stdout reader ended");
        });

        // stderr reader: log + auth-failure sniff. Codex prints rmcp /
        // websocket errors here including 401 Unauthorized when the
        // ChatGPT token has expired. Surfacing it through record_failure
        // makes the TUI subscription row reflect reality.
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    warn!(line = %line, "codex stderr");
                    if crate::memory::oneshot::is_auth_failure(&line) {
                        crate::subscription::record_failure(
                            crate::agent::Runtime::Codex,
                            format!("codex stderr: {}", &line.chars().take(200).collect::<String>()),
                        );
                    }
                }
            }
            info!("codex stderr reader ended");
        });

        Ok(CodexHandle {
            child,
            stdin,
            rx,
            session_id: None,
            last_agent_message: String::new(),
        })
    }

    /// Receive the next [`RuntimeEvent`] from the codex subprocess.
    /// Returns `None` when the subprocess exits.
    #[allow(dead_code)]
    pub async fn recv_event(&mut self) -> Option<RuntimeEvent> {
        let event = self.rx.recv().await?;
        // Side-effects on the handle: track session_id and accumulate the
        // running last-agent-message so we can synthesise a `Result` event
        // when `turn.completed` arrives.
        match &event {
            RuntimeEvent::SystemInit { session_id } => {
                self.session_id = Some(session_id.clone());
            }
            RuntimeEvent::Assistant { content } => {
                for block in content {
                    if let super::claude::ContentBlock::Text(t) = block {
                        self.last_agent_message.push_str(t);
                    }
                }
            }
            _ => {}
        }
        Some(event)
    }

    /// Wait for the turn to complete and return the accumulated final text.
    /// Behaves like [`ClaudeHandle::wait_for_result`] — caller can optionally
    /// tee every event into an observer channel (used by Slack reactions etc.).
    #[allow(dead_code)]
    pub async fn wait_for_result(
        &mut self,
        observer: Option<mpsc::UnboundedSender<RuntimeEvent>>,
    ) -> Result<String> {
        let mut got_result = false;
        let mut final_text = String::new();
        let mut error_text: Option<String> = None;

        while let Some(event) = self.recv_event().await {
            if let Some(ref tx) = observer {
                let _ = tx.send(event.clone());
            }
            match event {
                RuntimeEvent::SystemInit { .. } => {}
                RuntimeEvent::Assistant { content } => {
                    // Accumulate text; final value picked up at turn.completed.
                    for block in &content {
                        if let super::claude::ContentBlock::Text(t) = block {
                            final_text.push_str(t);
                        }
                    }
                }
                RuntimeEvent::Result { result, session_id } => {
                    if !session_id.is_empty() {
                        self.session_id = Some(session_id);
                    }
                    if !result.is_empty() {
                        final_text = result;
                    } else if !self.last_agent_message.is_empty() {
                        // codex turn.completed has no `result` field; fall
                        // back to the accumulator we tracked alongside.
                        final_text = std::mem::take(&mut self.last_agent_message);
                    }
                    got_result = true;
                    break;
                }
                RuntimeEvent::TextDelta { .. }
                | RuntimeEvent::ToolUseStart { .. }
                | RuntimeEvent::ToolResult { .. }
                | RuntimeEvent::StreamEvent { .. } => {}
                RuntimeEvent::Unknown(v) => {
                    // turn.failed shows up here if our parser couldn't pin it
                    // down — pull out the message if present so we surface it.
                    if let Some(msg) = v
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                    {
                        error_text = Some(msg.to_string());
                    }
                }
            }
        }

        if let Some(err) = error_text {
            return Err(CatClawError::Claude(format!("codex turn failed: {}", err)));
        }
        if !got_result {
            return Err(CatClawError::Claude(
                "codex process ended without turn.completed".to_string(),
            ));
        }
        Ok(final_text)
    }

    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .map(|s| s.is_none())
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub async fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .await
            .map_err(|e| CatClawError::Claude(format!("failed to kill codex: {}", e)))
    }
}

impl Drop for CodexHandle {
    fn drop(&mut self) {
        // Best-effort kill on drop — same pattern as ClaudeHandle.
        let _ = self.child.start_kill();
    }
}

/// Translate a single line of codex `--json` output into a [`RuntimeEvent`].
///
/// Lenient parser: anything we can't pin down maps to [`RuntimeEvent::Unknown`]
/// so codex schema drift doesn't crash the gateway. The transcript writer
/// already handles Unknown by skipping it.
fn parse_codex_event(value: &Value) -> RuntimeEvent {
    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        "thread.started" => {
            let session_id = value
                .get("thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            RuntimeEvent::SystemInit { session_id }
        }
        "turn.started" => {
            // Internal-only marker; let downstream skip it via Unknown.
            RuntimeEvent::Unknown(value.clone())
        }
        "item.started" => parse_item_started(value),
        "item.completed" => parse_item_completed(value),
        "turn.completed" => {
            // We don't carry the model usage in RuntimeEvent — the
            // accumulator `last_agent_message` (set by `item.completed` for
            // type=agent_message) supplies the text.
            RuntimeEvent::Result {
                result: String::new(),
                session_id: String::new(),
            }
        }
        "turn.failed" => {
            // Preserve the raw value so wait_for_result can extract .error.message.
            RuntimeEvent::Unknown(value.clone())
        }
        "error" => RuntimeEvent::Unknown(value.clone()),
        _ => RuntimeEvent::Unknown(value.clone()),
    }
}

fn parse_item_started(value: &Value) -> RuntimeEvent {
    let item = match value.get("item") {
        Some(i) => i,
        None => return RuntimeEvent::Unknown(value.clone()),
    };
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match item_type {
        "command_execution" => {
            let command = item
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            RuntimeEvent::ToolUseStart {
                name: "shell".to_string(),
                input: serde_json::json!({ "command": command }),
            }
        }
        "mcp_tool_call" => {
            let server = item.get("server").and_then(|s| s.as_str()).unwrap_or("");
            let tool = item.get("tool").and_then(|t| t.as_str()).unwrap_or("");
            let arguments = item.get("arguments").cloned().unwrap_or(Value::Null);
            RuntimeEvent::ToolUseStart {
                name: format!("mcp__{}__{}", server, tool),
                input: arguments,
            }
        }
        _ => RuntimeEvent::Unknown(value.clone()),
    }
}

fn parse_item_completed(value: &Value) -> RuntimeEvent {
    let item = match value.get("item") {
        Some(i) => i,
        None => return RuntimeEvent::Unknown(value.clone()),
    };
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match item_type {
        "agent_message" => {
            let text = item
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            RuntimeEvent::Assistant {
                content: vec![super::claude::ContentBlock::Text(text)],
            }
        }
        "command_execution" => {
            let exit_code = item.get("exit_code").and_then(|c| c.as_i64()).unwrap_or(0);
            let output = item
                .get("aggregated_output")
                .cloned()
                .unwrap_or(Value::Null);
            RuntimeEvent::ToolResult {
                name: "shell".to_string(),
                output,
                is_error: exit_code != 0,
            }
        }
        "mcp_tool_call" => {
            let server = item.get("server").and_then(|s| s.as_str()).unwrap_or("");
            let tool = item.get("tool").and_then(|t| t.as_str()).unwrap_or("");
            let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("");
            let result = item.get("result").cloned().unwrap_or(Value::Null);
            RuntimeEvent::ToolResult {
                name: format!("mcp__{}__{}", server, tool),
                output: result,
                is_error: status == "failed",
            }
        }
        _ => RuntimeEvent::Unknown(value.clone()),
    }
}
