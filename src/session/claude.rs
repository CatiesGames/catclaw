use serde::Serialize;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::error::{CatClawError, Result};

/// A handle to a running `claude -p` subprocess.
///
/// Uses `--output-format stream-json --verbose` to get NDJSON events.
/// Event types from the Claude CLI:
///   - `system` (subtype: "init") — session initialization, contains session_id
///   - `assistant` — complete assistant message with content blocks
///   - `result` — final result with .result text
///   - `stream_event` — partial streaming events (only with --include-partial-messages)
#[allow(dead_code)]
pub struct ClaudeHandle {
    child: Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout_rx: tokio::sync::mpsc::Receiver<ClaudeEvent>,
    pub session_id: Option<String>,
}

/// Events from the claude CLI stream-json output.
/// We use a loosely-typed approach to handle format changes gracefully.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ClaudeEvent {
    /// Session init event — contains session_id
    SystemInit { session_id: String },
    /// Complete assistant message
    Assistant { content: Vec<ContentBlock> },
    /// Final result
    Result { result: String, session_id: String },
    /// Incremental text delta from streaming (parsed from stream_event)
    TextDelta { text: String },
    /// Tool use start from streaming (parsed from stream_event)
    ToolUseStart { name: String, input: serde_json::Value },
    /// Raw streaming event we don't specifically parse
    StreamEvent { event: serde_json::Value },
    /// Any event we don't recognize
    Unknown(serde_json::Value),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ContentBlock {
    Text(String),
    ToolUse {
        name: String,
        input: serde_json::Value,
    },
    Other(serde_json::Value),
}

/// Input message for stream-json input format.
/// Format: {"type":"user","message":{"role":"user","content":"..."}}
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct StreamInput {
    #[serde(rename = "type")]
    msg_type: &'static str,
    message: StreamInputMessage,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct StreamInputMessage {
    role: &'static str,
    content: String,
}

/// Truncate a string for log display, adding "..." if truncated.
fn truncate_for_log(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let truncated: String = trimmed.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

/// Log a parsed ClaudeEvent with its content visible in logs.
/// Most events are debug-level to reduce noise; only session lifecycle at info.
fn log_claude_event(event: &ClaudeEvent) {
    match event {
        ClaudeEvent::SystemInit { session_id } => {
            info!(session_id = %session_id, "claude → SystemInit");
        }
        ClaudeEvent::Result { result, session_id } => {
            info!(session_id = %session_id, len = result.len(), "claude → Result");
            debug!(session_id = %session_id, "claude → Result: {}", truncate_for_log(result, 200));
        }
        ClaudeEvent::TextDelta { text } => {
            debug!("claude → TextDelta: {}", truncate_for_log(text, 80));
        }
        ClaudeEvent::ToolUseStart { name, input } => {
            debug!(tool = %name, "claude → ToolUse: {} input={}", name, truncate_for_log(&input.to_string(), 200));
        }
        ClaudeEvent::Assistant { content } => {
            for block in content {
                match block {
                    ContentBlock::Text(t) => {
                        debug!("claude → Assistant text: {}", truncate_for_log(t, 300));
                    }
                    ContentBlock::ToolUse { name, input } => {
                        debug!("claude → Assistant tool_use: {} input={}", name, truncate_for_log(&input.to_string(), 200));
                    }
                    ContentBlock::Other(v) => {
                        debug!("claude → Assistant block: {}", truncate_for_log(&v.to_string(), 200));
                    }
                }
            }
        }
        ClaudeEvent::StreamEvent { event } => {
            debug!("claude → StreamEvent: {}", truncate_for_log(&event.to_string(), 200));
        }
        ClaudeEvent::Unknown(v) => {
            debug!("claude → Unknown: {}", truncate_for_log(&v.to_string(), 200));
        }
    }
}

/// Parse a raw JSON line into a ClaudeEvent
fn parse_event(value: &serde_json::Value) -> ClaudeEvent {
    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        "system" => {
            let subtype = value.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            if subtype == "init" {
                let session_id = value
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                ClaudeEvent::SystemInit { session_id }
            } else {
                ClaudeEvent::Unknown(value.clone())
            }
        }
        "assistant" => {
            let mut blocks = Vec::new();
            if let Some(content) = value.get("content").and_then(|c| c.as_array()) {
                for block in content {
                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match block_type {
                        "text" => {
                            let text = block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            blocks.push(ContentBlock::Text(text));
                        }
                        "tool_use" => {
                            let name = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input = block
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);
                            blocks.push(ContentBlock::ToolUse { name, input });
                        }
                        _ => {
                            blocks.push(ContentBlock::Other(block.clone()));
                        }
                    }
                }
            }
            // If content is a string directly (simpler format)
            if blocks.is_empty() {
                if let Some(text) = value.get("content").and_then(|c| c.as_str()) {
                    blocks.push(ContentBlock::Text(text.to_string()));
                }
            }
            ClaudeEvent::Assistant { content: blocks }
        }
        "result" => {
            let is_error = value.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
            let result = value
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            let session_id = value
                .get("session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();

            // If result is empty, log the raw JSON for debugging
            if result.is_empty() {
                warn!("claude result event has empty result, raw JSON: {}", value);
            }
            if is_error {
                // Extract error details — result field contains the error message
                let error_msg = if result.is_empty() {
                    format!("claude returned error (raw: {})", value)
                } else {
                    result.clone()
                };
                warn!(error = %error_msg, "claude returned error result");
                // Return as Result with the error text so it surfaces to the user
                ClaudeEvent::Result { result: if result.is_empty() { error_msg } else { result }, session_id }
            } else {
                ClaudeEvent::Result { result, session_id }
            }
        }
        "stream_event" => {
            // Parse stream_event for text deltas and tool use
            // Format: { "type": "stream_event", "event": { "type": "content_block_delta", "delta": { "type": "text_delta", "text": "..." } } }
            // Or: { "type": "stream_event", "event": { "type": "content_block_start", "content_block": { "type": "tool_use", "name": "...", "input": {} } } }
            if let Some(event) = value.get("event") {
                let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match event_type {
                    "content_block_delta" => {
                        if let Some(delta) = event.get("delta") {
                            let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if delta_type == "text_delta" {
                                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                    return ClaudeEvent::TextDelta { text: text.to_string() };
                                }
                            }
                        }
                        ClaudeEvent::StreamEvent { event: event.clone() }
                    }
                    "content_block_start" => {
                        if let Some(block) = event.get("content_block") {
                            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if block_type == "tool_use" {
                                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);
                                return ClaudeEvent::ToolUseStart { name, input };
                            }
                        }
                        ClaudeEvent::StreamEvent { event: event.clone() }
                    }
                    _ => ClaudeEvent::StreamEvent { event: event.clone() },
                }
            } else {
                ClaudeEvent::StreamEvent { event: serde_json::Value::Null }
            }
        }
        _ => {
            // Events without a "type" field (e.g. rate_limit_info) — silently ignore
            if value.get("rate_limit_info").is_some() {
                ClaudeEvent::StreamEvent { event: value.clone() }
            } else {
                ClaudeEvent::Unknown(value.clone())
            }
        }
    }
}

#[allow(dead_code)]
impl ClaudeHandle {
    /// Spawn a new claude -p subprocess with the given args.
    /// The args should NOT include the initial prompt — it will be sent via stdin
    /// if using stream-json input, or the caller should include it in args.
    pub async fn spawn(args: Vec<String>) -> Result<Self> {
        info!(args = ?args, "spawning claude process");

        let mut child = Command::new("claude")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("CLAUDECODE")
            .spawn()
            .map_err(|e| CatClawError::Claude(format!("failed to spawn claude: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture stdin".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture stderr".to_string()))?;

        // Channel for parsed events
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        // Spawn stdout reader — parse NDJSON lines
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                debug!(line = %line, "claude stdout");
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(value) => {
                        let event = parse_event(&value);
                        log_claude_event(&event);
                        if tx.send(event).await.is_err() {
                            warn!("claude event channel closed, stopping stdout reader");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, line = %line, "failed to parse claude JSON line");
                    }
                }
            }
            info!("claude stdout reader ended");
        });

        // Spawn stderr reader — log it (warn level so errors are visible)
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    warn!("claude stderr: {}", line);
                }
            }
            info!("claude stderr reader ended");
        });

        Ok(ClaudeHandle {
            child,
            stdin: Some(stdin),
            stdout_rx: rx,
            session_id: None,
        })
    }

    /// Spawn a claude process with the prompt passed as a CLI argument (simple mode).
    /// This is the easiest way — just `claude -p "message" --output-format stream-json`.
    /// stdin is set to null since prompt is passed as CLI arg, not via stdin.
    pub async fn spawn_with_prompt(
        mut base_args: Vec<String>,
        prompt: &str,
    ) -> Result<Self> {
        // Insert the prompt right after -p
        // base_args should already contain -p, --output-format, etc.
        // We just need to add the prompt as the positional arg after -p
        if let Some(pos) = base_args.iter().position(|a| a == "-p") {
            base_args.insert(pos + 1, prompt.to_string());
        } else {
            // Prepend -p "prompt"
            base_args.insert(0, "-p".to_string());
            base_args.insert(1, prompt.to_string());
        }

        Self::spawn_no_stdin(base_args).await
    }

    /// Like spawn() but with stdin set to null (for CLI-arg prompt mode).
    /// This prevents claude from hanging waiting for stdin input.
    async fn spawn_no_stdin(args: Vec<String>) -> Result<Self> {
        info!(args = ?args, "spawning claude process");

        let mut child = Command::new("claude")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("CLAUDECODE")
            .spawn()
            .map_err(|e| CatClawError::Claude(format!("failed to spawn claude: {}", e)))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CatClawError::Claude("failed to capture stderr".to_string()))?;

        // Channel for parsed events
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        // Spawn stdout reader — parse NDJSON lines
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                debug!(line = %line, "claude stdout");
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(value) => {
                        let event = parse_event(&value);
                        log_claude_event(&event);
                        if tx.send(event).await.is_err() {
                            warn!("claude event channel closed, stopping stdout reader");
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, line = %line, "failed to parse claude JSON line");
                    }
                }
            }
            info!("claude stdout reader ended");
        });

        // Spawn stderr reader
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    warn!("claude stderr: {}", line);
                }
            }
            info!("claude stderr reader ended");
        });

        Ok(ClaudeHandle {
            child,
            stdin: None,
            stdout_rx: rx,
            session_id: None,
        })
    }

    /// Send a user message via stdin (stream-json input mode).
    pub async fn send_message(&mut self, text: &str) -> Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            CatClawError::Claude("stdin not available (process may have exited)".to_string())
        })?;

        let input = StreamInput {
            msg_type: "user",
            message: StreamInputMessage {
                role: "user",
                content: text.to_string(),
            },
        };
        let json = serde_json::to_string(&input)?;
        debug!(msg = %json, "sending to claude stdin");

        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(|e| CatClawError::Claude(format!("failed to write to stdin: {}", e)))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| CatClawError::Claude(format!("failed to write newline: {}", e)))?;
        stdin
            .flush()
            .await
            .map_err(|e| CatClawError::Claude(format!("failed to flush stdin: {}", e)))?;

        Ok(())
    }

    /// Wait for the next event from claude
    pub async fn recv_event(&mut self) -> Option<ClaudeEvent> {
        self.stdout_rx.recv().await
    }

    /// Wait for the result event, returning the result text.
    /// Also captures session_id from system init events.
    pub async fn wait_for_result(&mut self) -> Result<String> {
        let mut result_text = String::new();

        while let Some(event) = self.recv_event().await {
            match event {
                ClaudeEvent::SystemInit { session_id } => {
                    debug!(session_id = %session_id, "got session init");
                    self.session_id = Some(session_id);
                }
                ClaudeEvent::Result {
                    result, session_id, ..
                } => {
                    if !session_id.is_empty() {
                        self.session_id = Some(session_id);
                    }
                    result_text = result;
                    break;
                }
                ClaudeEvent::Assistant { content } => {
                    // Accumulate text from content blocks
                    for block in &content {
                        if let ContentBlock::Text(text) = block {
                            result_text.push_str(text);
                        }
                    }
                }
                ClaudeEvent::TextDelta { .. } | ClaudeEvent::ToolUseStart { .. } | ClaudeEvent::StreamEvent { .. } => {
                    // Ignore streaming events in wait_for_result (used by Discord/Telegram)
                }
                ClaudeEvent::Unknown(_) => {
                    // Skip unknown events gracefully
                }
            }
        }

        if result_text.is_empty() {
            info!("claude process ended without result");
            return Err(CatClawError::Claude(
                "claude process ended without result".to_string(),
            ));
        }

        info!(len = result_text.len(), "claude result received");
        Ok(result_text)
    }

    /// Check if the child process is still running
    pub fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false)
    }

    /// Kill the child process
    pub async fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .await
            .map_err(|e| CatClawError::Claude(format!("failed to kill claude: {}", e)))
    }
}

impl Drop for ClaudeHandle {
    fn drop(&mut self) {
        // Best-effort kill on drop
        let _ = self.child.start_kill();
    }
}
