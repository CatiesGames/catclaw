use chrono::Utc;
use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::Agent;
use crate::error::{CatClawError, Result};
use crate::state::{SessionRow, StateDb};

use super::claude::{ClaudeEvent, ClaudeHandle};
use super::queue::SessionQueue;
use super::transcript::TranscriptLog;
use super::{Priority, SessionEvent, SessionKey};

/// Sender metadata for transcript logging and channel forwarding
#[derive(Debug, Clone, Default)]
pub struct SenderInfo {
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    /// Platform-native channel ID (for approval forwarding back to origin channel)
    pub channel_id: Option<String>,
}

/// Manages session lifecycle: create, resume, fork, archive
pub struct SessionManager {
    state_db: Arc<StateDb>,
    queue: SessionQueue,
    /// Active session handles: session_key → kill sender.
    /// Sending on the channel triggers process termination.
    active_handles: Arc<DashMap<String, tokio::sync::oneshot::Sender<()>>>,
    /// Sessions currently being set up (between acquire permit and registering active_handle).
    /// Prevents two concurrent requests for the same session key from both starting a process.
    in_flight: Arc<DashSet<String>>,
    /// Port of the built-in MCP server (injected into claude CLI args).
    mcp_port: Option<u16>,
    /// Path to catclaw config file (for hook subprocess --config arg).
    config_path: Option<std::path::PathBuf>,
}

#[allow(dead_code)]
impl SessionManager {
    pub fn new(
        state_db: Arc<StateDb>,
        max_concurrent: usize,
    ) -> Self {
        SessionManager {
            state_db,
            queue: SessionQueue::new(max_concurrent),
            active_handles: Arc::new(DashMap::new()),
            in_flight: Arc::new(DashSet::new()),
            mcp_port: None,
            config_path: None,
        }
    }

    pub fn with_mcp_port(mut self, port: u16) -> Self {
        self.mcp_port = Some(port);
        self
    }

    pub fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Access the underlying state database.
    pub fn state_db(&self) -> &StateDb {
        &self.state_db
    }

    /// Send a message to a session, creating or resuming as needed.
    /// Returns the response text.
    ///
    /// `initial_model`: model override for new sessions (from TUI pending session).
    /// For existing sessions, the model is read from DB metadata instead.
    pub async fn send_and_wait(
        &self,
        key: &SessionKey,
        agent: &Agent,
        message: &str,
        priority: Priority,
        sender: &SenderInfo,
        initial_model: Option<&str>,
    ) -> Result<String> {
        // Acquire concurrency permit
        let _permit = self.queue.acquire(priority).await;

        let session_key = key.to_key_string();

        // Guard: prevent two concurrent sends for the same session key
        if !self.in_flight.insert(session_key.clone()) {
            return Err(crate::error::CatClawError::Session(
                "session is already being processed, please wait".to_string()
            ));
        }
        struct InFlightGuard(Arc<DashSet<String>>, String);
        impl Drop for InFlightGuard { fn drop(&mut self) { self.0.remove(&self.1); } }
        let _in_flight_guard = InFlightGuard(self.in_flight.clone(), session_key.clone());

        // Check for existing session
        let existing = self.state_db.get_session(&session_key)?;

        // Read session-level model override from metadata (existing session)
        // or use initial_model (new session from pending)
        let session_model = existing.as_ref().and_then(|r| r.model())
            .or_else(|| initial_model.map(String::from));

        let (session_id, is_resume) = match existing {
            Some(row) if row.state != "archived" => {
                info!(session_key = %session_key, session_id = %row.session_id, "resuming session");
                (row.session_id, true)
            }
            _ => {
                let new_id = Uuid::new_v4().to_string();
                info!(session_key = %session_key, session_id = %new_id, "creating new session");

                // Build initial metadata with model and platform IDs
                let initial_metadata = {
                    let mut meta = serde_json::Map::new();
                    if let Some(m) = initial_model {
                        meta.insert("model".to_string(), serde_json::Value::String(m.to_string()));
                    }
                    if let Some(ref cid) = sender.channel_id {
                        meta.insert("channel_id".to_string(), serde_json::Value::String(cid.clone()));
                    }
                    if let Some(ref sid) = sender.sender_id {
                        meta.insert("sender_id".to_string(), serde_json::Value::String(sid.clone()));
                    }
                    if meta.is_empty() { None } else { Some(serde_json::Value::Object(meta).to_string()) }
                };

                // Save to DB
                let now = Utc::now().to_rfc3339();
                self.state_db.upsert_session(&SessionRow {
                    session_key: session_key.clone(),
                    session_id: new_id.clone(),
                    agent_id: agent.id.clone(),
                    origin: key.origin.clone(),
                    context_id: key.context_id.clone(),
                    parent_session_id: None,
                    state: "active".to_string(),
                    last_activity_at: now.clone(),
                    created_at: now,
                    metadata: initial_metadata,
                })?;

                // If there's a BOOT.md with content, prepend it to the message
                let boot_path = agent.workspace.join("BOOT.md");
                if let Ok(boot) = std::fs::read_to_string(&boot_path) {
                    if !boot.trim().is_empty() {
                        // Combine boot + user message into one prompt
                        let combined = format!(
                            "{}\n\n---\n\nUser message:\n{}",
                            boot.trim(),
                            message
                        );
                        let args =
                            agent.claude_args_with_mcp(&new_id, session_model.as_deref(), self.mcp_port, Some(&session_key), self.config_path.as_deref());
                        let mut handle =
                            ClaudeHandle::spawn_with_prompt(args, &combined).await?;
                        let response = handle.wait_for_result().await?;

                        // Determine final session ID
                        let final_id = handle
                            .session_id
                            .as_deref()
                            .unwrap_or(&new_id);

                        // Log transcript (new session — include label for readable filename)
                        let label = super::transcript::label_from_session_key(&session_key);
                        let transcript =
                            TranscriptLog::open_with_label(&agent.workspace, final_id, Some(&label)).await?;
                        let (ch_type, ch_name) = parse_session_key_channel(&session_key);
                        transcript.log_session_start(&session_key, ch_type, None, ch_name).await;
                        transcript
                            .log_user(
                                message,
                                sender.sender_id.as_deref(),
                                sender.sender_name.as_deref(),
                            )
                            .await;
                        transcript.log_assistant(&response, None).await;

                        // Update session_id if claude returned one
                        if let Some(real_id) = &handle.session_id {
                            let mut row = self.state_db.get_session(&session_key)?.unwrap();
                            row.session_id = real_id.clone();
                            row.state = "idle".to_string();
                            row.last_activity_at = Utc::now().to_rfc3339();
                            self.state_db.upsert_session(&row)?;
                        } else {
                            self.state_db
                                .update_session_state(&session_key, "idle")?;
                        }

                        return Ok(response);
                    }
                }

                (new_id, false)
            }
        };

        // Build args
        let args = if is_resume {
            agent.claude_resume_args_with_mcp(&session_id, session_model.as_deref(), self.mcp_port, Some(&session_key), self.config_path.as_deref())
        } else {
            agent.claude_args_with_mcp(&session_id, session_model.as_deref(), self.mcp_port, Some(&session_key), self.config_path.as_deref())
        };

        // Update state to active + register kill channel
        self.state_db
            .update_session_state(&session_key, "active")?;
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
        self.active_handles.insert(session_key.clone(), kill_tx);

        // Spawn claude with the prompt as CLI argument
        let mut handle = ClaudeHandle::spawn_with_prompt(args, message).await?;

        // Wait for result, but also listen for kill signal
        let response = tokio::select! {
            result = handle.wait_for_result() => {
                match result {
                    Ok(r) => Ok(r),
                    Err(e) if is_resume => {
                        // Resume failed — likely a stale session. Archive it so
                        // the next attempt creates a fresh session.
                        warn!(
                            session_key = %session_key,
                            session_id = %session_id,
                            error = %e,
                            "resume failed, archiving stale session"
                        );
                        self.state_db.update_session_state(&session_key, "archived")?;
                        self.active_handles.remove(&session_key);
                        return Err(CatClawError::Claude(
                            "Session expired — please resend your message.".to_string()
                        ));
                    }
                    Err(e) => Err(e),
                }
            },
            _ = &mut kill_rx => {
                warn!(session_key = %session_key, "session stopped by user");
                handle.kill().await.ok();
                self.active_handles.remove(&session_key);
                self.state_db.update_session_state(&session_key, "idle")?;
                return Err(CatClawError::Session("session stopped by user".to_string()));
            }
        }?;

        // Determine final session ID for transcript
        let final_id = handle
            .session_id
            .as_deref()
            .unwrap_or(&session_id);

        // Log transcript (pass label for new sessions so filename is readable)
        let label = if !is_resume {
            Some(super::transcript::label_from_session_key(&session_key))
        } else {
            None
        };
        let transcript = TranscriptLog::open_with_label(
            &agent.workspace,
            final_id,
            label.as_deref(),
        ).await?;
        if !is_resume {
            let (ch_type, ch_name) = parse_session_key_channel(&session_key);
            transcript.log_session_start(&session_key, ch_type, None, ch_name).await;
        }
        transcript
            .log_user(
                message,
                sender.sender_id.as_deref(),
                sender.sender_name.as_deref(),
            )
            .await;
        transcript.log_assistant(&response, None).await;

        // Update session_id if claude returned one (may differ from what we set)
        if let Some(real_id) = &handle.session_id {
            if *real_id != session_id {
                let mut row = self.state_db.get_session(&session_key)?.unwrap();
                row.session_id = real_id.clone();
                row.state = "idle".to_string();
                row.last_activity_at = Utc::now().to_rfc3339();
                self.state_db.upsert_session(&row)?;
            } else {
                self.state_db
                    .update_session_state(&session_key, "idle")?;
            }
        } else {
            self.state_db
                .update_session_state(&session_key, "idle")?;
        }

        self.active_handles.remove(&session_key);

        Ok(response)
    }

    /// Send a message to a session with streaming events.
    /// Returns a receiver of SessionEvent for incremental updates.
    /// Used by TUI/WebUI streaming mode only.
    ///
    /// `initial_model`: model override for new sessions (from TUI pending session).
    pub async fn send_streaming(
        &self,
        key: &SessionKey,
        agent: &Agent,
        message: &str,
        priority: Priority,
        sender: &SenderInfo,
        initial_model: Option<&str>,
    ) -> Result<tokio::sync::mpsc::UnboundedReceiver<SessionEvent>> {
        // Acquire concurrency permit
        let _permit = self.queue.acquire(priority).await;

        let session_key = key.to_key_string();

        // Guard: prevent two concurrent sends for the same session key
        if !self.in_flight.insert(session_key.clone()) {
            return Err(crate::error::CatClawError::Session(
                "session is already being processed, please wait".to_string()
            ));
        }

        // Check for existing session
        let existing = self.state_db.get_session(&session_key)?;

        // Read session-level model override from metadata (existing session)
        // or use initial_model (new session from pending)
        let session_model = existing.as_ref().and_then(|r| r.model())
            .or_else(|| initial_model.map(String::from));

        let (session_id, is_resume) = match existing {
            Some(row) if row.state != "archived" => {
                info!(session_key = %session_key, session_id = %row.session_id, "resuming session (streaming)");
                (row.session_id, true)
            }
            _ => {
                let new_id = Uuid::new_v4().to_string();
                info!(session_key = %session_key, session_id = %new_id, "creating new session (streaming)");

                // Build initial metadata with model and platform IDs
                let initial_metadata = {
                    let mut meta = serde_json::Map::new();
                    if let Some(m) = initial_model {
                        meta.insert("model".to_string(), serde_json::Value::String(m.to_string()));
                    }
                    if let Some(ref cid) = sender.channel_id {
                        meta.insert("channel_id".to_string(), serde_json::Value::String(cid.clone()));
                    }
                    if let Some(ref sid) = sender.sender_id {
                        meta.insert("sender_id".to_string(), serde_json::Value::String(sid.clone()));
                    }
                    if meta.is_empty() { None } else { Some(serde_json::Value::Object(meta).to_string()) }
                };

                let now = Utc::now().to_rfc3339();
                self.state_db.upsert_session(&SessionRow {
                    session_key: session_key.clone(),
                    session_id: new_id.clone(),
                    agent_id: agent.id.clone(),
                    origin: key.origin.clone(),
                    context_id: key.context_id.clone(),
                    parent_session_id: None,
                    state: "active".to_string(),
                    last_activity_at: now.clone(),
                    created_at: now,
                    metadata: initial_metadata,
                })?;

                (new_id, false)
            }
        };

        // Handle BOOT.md for new sessions
        let actual_message = if !is_resume {
            let boot_path = agent.workspace.join("BOOT.md");
            if let Ok(boot) = std::fs::read_to_string(&boot_path) {
                if !boot.trim().is_empty() {
                    format!("{}\n\n---\n\nUser message:\n{}", boot.trim(), message)
                } else {
                    message.to_string()
                }
            } else {
                message.to_string()
            }
        } else {
            message.to_string()
        };

        // Build args
        let args = if is_resume {
            agent.claude_resume_args_with_mcp(&session_id, session_model.as_deref(), self.mcp_port, Some(&session_key), self.config_path.as_deref())
        } else {
            agent.claude_args_with_mcp(&session_id, session_model.as_deref(), self.mcp_port, Some(&session_key), self.config_path.as_deref())
        };

        // Update state to active + register kill channel
        self.state_db
            .update_session_state(&session_key, "active")?;
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();
        self.active_handles.insert(session_key.clone(), kill_tx);

        // Spawn claude
        let mut handle = ClaudeHandle::spawn_with_prompt(args, &actual_message).await?;

        // Create channel for streaming events
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();

        // Spawn background task to read events and forward them
        let state_db = self.state_db.clone();
        let active_handles = self.active_handles.clone();
        let in_flight = self.in_flight.clone();
        let agent_workspace = agent.workspace.clone();
        let sender_id = sender.sender_id.clone();
        let sender_name = sender.sender_name.clone();
        let message_owned = message.to_string();
        let session_key_owned = session_key.clone();
        let session_id_owned = session_id.clone();

        // Move permit into the spawned task so concurrency is held until completion
        tokio::spawn(async move {
            let _permit = _permit; // keep permit alive
            // Remove from in_flight when this task completes (via Drop)
            struct InFlightGuard(Arc<DashSet<String>>, String);
            impl Drop for InFlightGuard { fn drop(&mut self) { self.0.remove(&self.1); } }
            let _in_flight_guard = InFlightGuard(in_flight, session_key_owned.clone());

            let mut kill_rx = kill_rx;

            let mut result_text = String::new();
            let mut final_session_id = session_id_owned.clone();
            let mut stopped = false;
            let mut tool_uses: Vec<super::transcript::ToolUseEntry> = Vec::new();

            // Open transcript log and write user message immediately
            let label = if !is_resume {
                Some(super::transcript::label_from_session_key(&session_key_owned))
            } else {
                None
            };
            let transcript = TranscriptLog::open_with_label(&agent_workspace, &session_id_owned, label.as_deref()).await.ok();
            if let Some(ref t) = transcript {
                if !is_resume {
                    let (ch_type, ch_name) = parse_session_key_channel(&session_key_owned);
                    t.log_session_start(&session_key_owned, ch_type, None, ch_name).await;
                }
                t.log_user(&message_owned, sender_id.as_deref(), sender_name.as_deref()).await;
            }

            info!(session_key = %session_key_owned, is_running = handle.is_running(), "streaming: entering event loop");
            let mut got_first_event = false;
            let startup_timeout = tokio::time::sleep(std::time::Duration::from_secs(120));
            tokio::pin!(startup_timeout);

            loop {
                tokio::select! {
                    maybe_event = handle.recv_event() => {
                        let event = match maybe_event {
                            Some(e) => e,
                            None => {
                                info!(session_key = %session_key_owned, "streaming: recv_event returned None (process exited)");
                                break;
                            }
                        };
                        if !got_first_event {
                            got_first_event = true;
                            info!(session_key = %session_key_owned, "streaming: first event received");
                        }
                        match &event {
                            ClaudeEvent::SystemInit { session_id } => {
                                info!(session_id = %session_id, "streaming: got session init");
                                if !session_id.is_empty() {
                                    final_session_id = session_id.clone();
                                }
                                handle.session_id = Some(session_id.clone());
                            }
                            ClaudeEvent::TextDelta { text } => {
                                let _ = event_tx.send(SessionEvent::TextDelta { text: text.clone() });
                            }
                            ClaudeEvent::ToolUseStart { name, input } => {
                                let _ = event_tx.send(SessionEvent::ToolUse { name: name.clone(), input: input.clone() });
                                tool_uses.push(super::transcript::ToolUseEntry {
                                    name: name.clone(),
                                    input: input.clone(),
                                });
                            }
                            ClaudeEvent::Result { result, session_id } => {
                                if !session_id.is_empty() {
                                    final_session_id = session_id.clone();
                                }
                                result_text = result.clone();
                                break;
                            }
                            ClaudeEvent::Assistant { content } => {
                                for block in content {
                                    if let super::claude::ContentBlock::Text(text) = block {
                                        result_text.push_str(text);
                                    }
                                }
                            }
                            ClaudeEvent::StreamEvent { .. } => {
                                // Unrecognized stream events — skip silently
                            }
                            ClaudeEvent::Unknown(_) => {}
                        }
                    }
                    _ = &mut kill_rx => {
                        warn!(session_key = %session_key_owned, "streaming session stopped by user");
                        handle.kill().await.ok();
                        stopped = true;
                        break;
                    }
                    _ = &mut startup_timeout, if !got_first_event => {
                        warn!(session_key = %session_key_owned, "streaming: timeout waiting for first event from claude (120s)");
                        handle.kill().await.ok();
                        let _ = event_tx.send(SessionEvent::Error {
                            message: "Timeout: claude process did not respond within 120 seconds. Check logs for details.".to_string(),
                        });
                        let _ = state_db.update_session_state(&session_key_owned, "idle");
                        active_handles.remove(&session_key_owned);
                        return;
                    }
                }
            }

            if stopped {
                let _ = state_db.update_session_state(&session_key_owned, "idle");
                active_handles.remove(&session_key_owned);
                let _ = event_tx.send(SessionEvent::Error {
                    message: "session stopped by user".to_string(),
                });
                return;
            }

            info!(session_key = %session_key_owned, result_len = result_text.len(), "streaming: event loop ended");

            // Log assistant response with tool uses to transcript
            if let Some(ref t) = transcript {
                let tools = if tool_uses.is_empty() { None } else { Some(tool_uses) };
                t.log_assistant(&result_text, tools).await;
            }

            // Update DB
            if final_session_id != session_id_owned {
                if let Ok(Some(mut row)) = state_db.get_session(&session_key_owned) {
                    row.session_id = final_session_id;
                    row.state = "idle".to_string();
                    row.last_activity_at = Utc::now().to_rfc3339();
                    let _ = state_db.upsert_session(&row);
                }
            } else {
                let _ = state_db.update_session_state(&session_key_owned, "idle");
            }

            active_handles.remove(&session_key_owned);

            // Send final complete event
            if result_text.is_empty() {
                if is_resume {
                    // Resume failed with empty result — likely a stale session
                    // (claude CLI session was deleted but our DB still has it).
                    // Archive it so the next message creates a fresh session.
                    warn!(
                        session_key = %session_key_owned,
                        session_id = %session_id_owned,
                        "resume returned empty result, archiving stale session"
                    );
                    let _ = state_db.update_session_state(&session_key_owned, "archived");
                    let _ = event_tx.send(SessionEvent::Error {
                        message: "Session expired — please resend your message.".to_string(),
                    });
                } else {
                    let _ = event_tx.send(SessionEvent::Error {
                        message: "claude process ended without result".to_string(),
                    });
                }
            } else {
                info!(len = result_text.len(), "claude result received (streaming)");
                let _ = event_tx.send(SessionEvent::Complete { text: result_text });
            }
        });

        Ok(event_rx)
    }

    /// Stop a running session by killing the underlying claude process.
    /// Works for both send_and_wait and send_streaming paths.
    /// Returns true if the session was running and stopped, false if not found.
    pub fn stop_session(&self, session_key: &str) -> bool {
        if let Some((_, kill_tx)) = self.active_handles.remove(session_key) {
            // Sending on the channel triggers the kill in the task
            let _ = kill_tx.send(());
            info!(session_key = %session_key, "stop_session: kill signal sent");
            true
        } else {
            info!(session_key = %session_key, "stop_session: session not active");
            false
        }
    }

    /// Fork a session into a new session key
    pub async fn fork(
        &self,
        source_key: &SessionKey,
        target_key: &SessionKey,
        agent: &Agent,
    ) -> Result<String> {
        let source_session = self
            .state_db
            .get_session(&source_key.to_key_string())?
            .ok_or_else(|| CatClawError::Session("source session not found".to_string()))?;

        let new_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.state_db.upsert_session(&SessionRow {
            session_key: target_key.to_key_string(),
            session_id: new_id.clone(),
            agent_id: agent.id.clone(),
            origin: target_key.origin.clone(),
            context_id: target_key.context_id.clone(),
            parent_session_id: Some(source_session.session_id.clone()),
            state: "suspended".to_string(),
            last_activity_at: now.clone(),
            created_at: now,
            metadata: None,
        })?;

        info!(
            source = %source_key,
            target = %target_key,
            parent_session = %source_session.session_id,
            new_session = %new_id,
            "forked session"
        );

        Ok(new_id)
    }

    /// Archive a session, optionally generating a summary first
    pub async fn archive(&self, session_key: &str) -> Result<()> {
        self.state_db
            .update_session_state(session_key, "archived")?;
        info!(session_key = %session_key, "archived session");
        Ok(())
    }

    /// List all sessions
    pub fn list_sessions(&self) -> Result<Vec<SessionRow>> {
        self.state_db.list_sessions()
    }

    /// Find sessions that have been idle longer than the given duration
    pub fn find_stale_sessions(&self, max_idle_hours: u64) -> Result<Vec<SessionRow>> {
        let cutoff = Utc::now() - chrono::Duration::hours(max_idle_hours as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let all = self.state_db.list_sessions()?;
        Ok(all
            .into_iter()
            .filter(|s| {
                s.state != "archived" && s.last_activity_at < cutoff_str
            })
            .collect())
    }

    /// On startup, suspend all previously active sessions (subprocess died)
    pub fn recover_on_startup(&self) -> Result<usize> {
        let count = self.state_db.suspend_all_active_sessions()?;
        if count > 0 {
            info!(count = count, "suspended previously active sessions on startup");
        }
        Ok(count)
    }

    /// Set or clear the model for a session (persisted in metadata).
    pub fn set_session_model(&self, session_key: &str, model: Option<&str>) -> Result<()> {
        self.state_db.set_session_model(session_key, model)?;
        info!(session_key = %session_key, model = ?model, "session model updated");
        Ok(())
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.available_permits()
    }
}

/// Extract channel type and name from a session key string.
/// Key format: `catclaw:{agent_id}:{origin}:{context_id}`
/// Returns (Some(origin), Some(context_id)) or (None, None).
fn parse_session_key_channel(key: &str) -> (Option<&str>, Option<&str>) {
    let parts: Vec<&str> = key.splitn(4, ':').collect();
    if parts.len() >= 4 {
        (Some(parts[2]), Some(parts[3]))
    } else {
        (None, None)
    }
}
