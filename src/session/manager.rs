use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agent::Agent;
use crate::config::Config;
use crate::error::{CatClawError, Result};
use crate::state::{SessionRow, StateDb};

use super::queue::SessionQueue;
use super::runtime::RuntimeEvent;
use super::transcript::TranscriptLog;
use super::{Priority, SessionEvent, SessionKey};

/// Sender metadata for transcript logging and channel forwarding
#[derive(Debug, Clone, Default)]
pub struct SenderInfo {
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    /// Platform-native channel ID (for approval forwarding back to origin channel)
    pub channel_id: Option<String>,
    /// Thread ID (Slack thread_ts, Discord thread channel ID) for approval thread targeting
    pub thread_id: Option<String>,
}

/// Hook the manager calls after writing each transcript turn so the diary
/// subsystem can decide whether to fire a rolling extraction. Kept abstract
/// to avoid a circular dependency between `session::manager` and `scheduler`.
///
/// `user_turns_since_marker` is the count returned by `log_user` — the diary
/// subsystem compares it against its threshold and triggers when met.
#[async_trait]
pub trait DiaryTrigger: Send + Sync {
    async fn on_turn_complete(
        &self,
        agent: &Agent,
        session_key: &str,
        session_id: &str,
        user_turns_since_marker: u32,
    );
}

/// Manages session lifecycle: create, resume, fork, archive
pub struct SessionManager {
    state_db: Arc<StateDb>,
    queue: SessionQueue,
    /// Active session handles: session_key → kill sender.
    /// Sending on the channel triggers process termination.
    active_handles: Arc<DashMap<String, tokio::sync::oneshot::Sender<()>>>,
    /// Per-session mutex: queues concurrent messages for the same session instead of rejecting them.
    session_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Port of the built-in MCP server (injected into claude CLI args).
    mcp_port: Option<u16>,
    /// Path to catclaw config file (for hook subprocess --config arg).
    config_path: Option<std::path::PathBuf>,
    /// Shared config for reading mcp_env at session spawn time.
    config: Option<Arc<std::sync::RwLock<Config>>>,
    /// Optional hook invoked after each completed turn — drives the rolling
    /// "every N user turns" diary trigger. None disables rolling diary.
    diary_trigger: Option<Arc<dyn DiaryTrigger>>,
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
            session_locks: Arc::new(DashMap::new()),
            mcp_port: None,
            config_path: None,
            config: None,
            diary_trigger: None,
        }
    }

    pub fn with_diary_trigger(mut self, trigger: Arc<dyn DiaryTrigger>) -> Self {
        self.diary_trigger = Some(trigger);
        self
    }

    pub fn diary_trigger(&self) -> Option<Arc<dyn DiaryTrigger>> {
        self.diary_trigger.clone()
    }

    /// Call the rolling-diary hook after a turn was written to the transcript.
    /// No-op when no trigger is configured.
    async fn notify_diary_trigger(
        &self,
        agent: &Agent,
        session_key: &str,
        session_id: &str,
        user_turns_since_marker: u32,
    ) {
        if let Some(trigger) = self.diary_trigger.as_ref() {
            trigger
                .on_turn_complete(agent, session_key, session_id, user_turns_since_marker)
                .await;
        }
    }

    /// Get or create a per-session mutex for serializing messages to the same session.
    fn session_lock(&self, session_key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.session_locks
            .entry(session_key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub fn with_mcp_port(mut self, port: u16) -> Self {
        self.mcp_port = Some(port);
        self
    }

    pub fn with_config_path(mut self, path: std::path::PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    pub fn with_config(mut self, config: Arc<std::sync::RwLock<Config>>) -> Self {
        self.config = Some(config);
        self
    }

    /// Access the underlying state database.
    pub fn state_db(&self) -> &StateDb {
        &self.state_db
    }

    /// Get an Arc reference to the state database (for spawning tasks).
    pub fn state_db_arc(&self) -> Arc<StateDb> {
        self.state_db.clone()
    }

    /// Get a clone of the shared config Arc, if any.
    pub fn config_arc(&self) -> Option<Arc<std::sync::RwLock<crate::config::Config>>> {
        self.config.clone()
    }

    /// Read the current mcp_env from config (or empty if no config).
    fn mcp_env(&self) -> HashMap<String, HashMap<String, String>> {
        self.config.as_ref()
            .map(|c| c.read().unwrap().mcp_env.clone())
            .unwrap_or_default()
    }

    /// Read the current env from config (or empty if no config).
    /// These are injected as OS-level env vars into claude subprocesses.
    fn subprocess_env(&self) -> HashMap<String, String> {
        self.config.as_ref()
            .map(|c| c.read().unwrap().env.clone())
            .unwrap_or_default()
    }

    /// Send a message to a session, creating or resuming as needed.
    /// Returns the response text.
    ///
    /// `initial_model`: model override for new sessions (from TUI pending session).
    /// For existing sessions, the model is read from DB metadata instead.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_and_wait(
        &self,
        key: &SessionKey,
        agent: &Agent,
        message: &str,
        priority: Priority,
        sender: &SenderInfo,
        initial_model: Option<&str>,
        event_observer: Option<tokio::sync::mpsc::UnboundedSender<super::runtime::RuntimeEvent>>,
    ) -> Result<String> {
        let session_key = key.to_key_string();
        let mcp_env = self.mcp_env();
        let subprocess_env = self.subprocess_env();

        // Per-session mutex: queue concurrent messages instead of rejecting them
        let lock = self.session_lock(&session_key);
        let _session_guard = lock.lock().await;

        // Acquire global concurrency permit (after session lock so waiters don't consume slots)
        let _permit = self.queue.acquire(priority).await;

        // Check for existing session
        let existing = self.state_db.get_session(&session_key)?;

        // Read session-level model override from metadata (existing session)
        // or use initial_model (new session from pending)
        let session_model = existing.as_ref().and_then(|r| r.model())
            .or_else(|| initial_model.map(String::from));

        // codex-runtime-plan.md §2.4: cross-runtime resume guard.
        // If the stored session was created with a different runtime than the
        // agent currently has, fail loudly rather than spawning the wrong CLI
        // with a session_id it cannot resume.
        if let Some(row) = existing.as_ref() {
            let stored_runtime = row.runtime_from_metadata().unwrap_or(crate::agent::Runtime::Claude);
            if stored_runtime != agent.runtime {
                return Err(CatClawError::Session(format!(
                    "session was created with runtime={:?} but agent is now {:?} — start a new session",
                    stored_runtime, agent.runtime
                )));
            }
        }

        let (session_id, is_resume) = match existing {
            Some(row) if row.state != "archived" => {
                info!(session_key = %session_key, session_id = %row.session_id, "resuming session");
                (row.session_id, true)
            }
            _ => {
                let new_id = Uuid::new_v4().to_string();
                info!(session_key = %session_key, session_id = %new_id, "creating new session");

                // Build initial metadata with model, platform IDs, and runtime
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
                    if let Some(ref tid) = sender.thread_id {
                        meta.insert("thread_id".to_string(), serde_json::Value::String(tid.clone()));
                    }
                    // codex-runtime-plan.md §2.4: store runtime so cross-runtime
                    // resume can detect mismatch. Always written for new sessions.
                    meta.insert(
                        "runtime".to_string(),
                        serde_json::Value::String(agent.runtime.as_str().to_string()),
                    );
                    Some(serde_json::Value::Object(meta).to_string())
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
                        let spawn_params = crate::session::runtime::SpawnParams {
                            session_id: &new_id,
                            model_override: session_model.as_deref(),
                            mcp_port: self.mcp_port,
                            hook_session_key: Some(&session_key),
                            config_path: self.config_path.as_deref(),
                            mcp_env: &mcp_env,
                            state_db: Some(&self.state_db),
                            is_resume: false,
                            resume_thread_id: None,
                        };

                        // Register kill channel so /stop works during BOOT.md execution
                        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
                        self.active_handles.insert(session_key.clone(), kill_tx);

                        let mut handle =
                            agent.spawn_session(&spawn_params, &combined, &subprocess_env).await?;

                        let response = tokio::select! {
                            result = handle.wait_for_result(event_observer.clone()) => result?,
                            _ = &mut kill_rx => {
                                warn!(session_key = %session_key, "boot session stopped by user");
                                handle.kill().await.ok();
                                self.active_handles.remove(&session_key);
                                self.state_db.update_session_state(&session_key, "idle")?;
                                return Err(CatClawError::Session("session stopped by user".to_string()));
                            }
                        };

                        self.active_handles.remove(&session_key);

                        // Determine final session ID
                        let returned_session_id = handle.session_id().map(String::from);
                        let final_id = returned_session_id.as_deref().unwrap_or(&new_id);

                        // Log transcript (skip for system sessions)
                        if !session_key.contains(":system:") {
                            let label = super::transcript::label_from_session_key(&session_key);
                            let transcript =
                                TranscriptLog::open_with_label(&agent.workspace, final_id, Some(&label)).await?;
                            let (ch_type, ch_name) = parse_session_key_channel(&session_key);
                            transcript.log_session_start(&session_key, ch_type, None, ch_name).await;
                            let turns = transcript
                                .log_user(
                                    message,
                                    sender.sender_id.as_deref(),
                                    sender.sender_name.as_deref(),
                                )
                                .await;
                            transcript.log_assistant(&response, None).await;
                            self.notify_diary_trigger(agent, &session_key, final_id, turns).await;
                        }

                        // Update session_id if claude returned one
                        if let Some(real_id) = &returned_session_id {
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
        // Update state to active + register kill channel
        self.state_db
            .update_session_state(&session_key, "active")?;
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
        self.active_handles.insert(session_key.clone(), kill_tx);

        // Spawn the agent's runtime (claude or codex) with the prompt
        let spawn_params = crate::session::runtime::SpawnParams {
            session_id: &session_id,
            model_override: session_model.as_deref(),
            mcp_port: self.mcp_port,
            hook_session_key: Some(&session_key),
            config_path: self.config_path.as_deref(),
            mcp_env: &mcp_env,
            state_db: Some(&self.state_db),
            is_resume,
            resume_thread_id: None,
        };
        let mut handle = agent.spawn_session(&spawn_params, message, &subprocess_env).await?;

        // Wait for result, but also listen for kill signal
        let response = tokio::select! {
            result = handle.wait_for_result(event_observer) => {
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
        let returned_session_id = handle.session_id().map(String::from);
        let final_id = returned_session_id.as_deref().unwrap_or(&session_id);

        // Log transcript (skip for system-originated sessions: heartbeat, cron tasks)
        let is_system = session_key.contains(":system:");
        if !is_system {
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
            let turns = transcript
                .log_user(
                    message,
                    sender.sender_id.as_deref(),
                    sender.sender_name.as_deref(),
                )
                .await;
            transcript.log_assistant(&response, None).await;
            self.notify_diary_trigger(agent, &session_key, final_id, turns).await;
        }

        // Update session_id if the runtime returned one (may differ from what we set)
        if let Some(real_id) = &returned_session_id {
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

    /// Run claude one-shot without creating a persistent session row.
    ///
    /// Used for transient tasks (e.g. social AI auto-reply) where the whole
    /// execution is a single prompt → single tool call pattern and no resume,
    /// transcript, or diary is wanted. The `hook_session_key` is passed through
    /// to the PreToolUse hook so approval rules still resolve the right agent,
    /// but nothing is inserted into the `sessions` table.
    ///
    /// Acquires the global concurrency permit so ephemeral calls queue behind
    /// regular sessions instead of overwhelming the claude subprocess pool.
    pub async fn ephemeral_run(
        &self,
        agent: &Agent,
        prompt: &str,
        hook_session_key: &str,
        priority: Priority,
    ) -> Result<String> {
        let mcp_env = self.mcp_env();
        let subprocess_env = self.subprocess_env();

        let _permit = self.queue.acquire(priority).await;

        let session_id = Uuid::new_v4().to_string();
        // Route through spawn_session so codex-runtime agents pick the right
        // CLI. Previously this called ClaudeHandle::spawn_with_prompt directly,
        // which silently fell back to `claude -p` for a codex agent and broke
        // the whole social auto-reply pipeline when an agent was switched to
        // codex. is_resume=false because ephemeral runs never resume.
        let params = super::runtime::SpawnParams {
            session_id: &session_id,
            model_override: None,
            mcp_port: self.mcp_port,
            hook_session_key: Some(hook_session_key),
            config_path: self.config_path.as_deref(),
            mcp_env: &mcp_env,
            state_db: Some(&self.state_db),
            is_resume: false,
            resume_thread_id: None,
        };
        let mut handle = agent.spawn_session(&params, prompt, &subprocess_env).await?;

        // For codex agents only: write a transient `archived` SessionRow so
        // the MCP intercept (mcp_server::resolve_agent_from_session) can
        // resolve `_meta.x-codex-turn-metadata.session_id` back to this agent
        // when the codex subprocess calls a catclaw MCP tool. Without this
        // row, codex's first MCP call lands as "unknown codex session" and
        // the approval gate hard-errors. Wrote as `archived` to keep the
        // session out of TUI lists / resume picker (ephemeral semantics
        // preserved — nothing meaningful is shown).
        //
        // We can only do this AFTER spawn so codex has actually issued its
        // thread.started event; recv one event to get the thread_id, then
        // write the row, then proceed with wait_for_result for the rest.
        if matches!(agent.runtime, crate::agent::Runtime::Codex) {
            // Drain events until we see SystemInit (or give up after a few
            // to avoid hanging on a misbehaving subprocess).
            for _ in 0..16 {
                match handle.recv_event().await {
                    Some(super::runtime::RuntimeEvent::SystemInit { session_id: thread_id }) => {
                        let now = chrono::Utc::now().to_rfc3339();
                        let row = SessionRow {
                            session_key: format!("catclaw:{}:ephemeral:{}", agent.id, thread_id),
                            session_id: thread_id.clone(),
                            agent_id: agent.id.clone(),
                            origin: "ephemeral".to_string(),
                            context_id: thread_id.clone(),
                            parent_session_id: None,
                            state: "archived".to_string(),
                            last_activity_at: now.clone(),
                            created_at: now,
                            metadata: Some(
                                serde_json::json!({
                                    "runtime": agent.runtime.as_str(),
                                    "ephemeral": true,
                                })
                                .to_string(),
                            ),
                        };
                        let _ = self.state_db.upsert_session(&row);
                        break;
                    }
                    Some(_) => continue, // any other event before SystemInit — keep looking
                    None => break,       // subprocess died early
                }
            }
        }

        handle.wait_for_result(None).await
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
        let session_key = key.to_key_string();
        let mcp_env = self.mcp_env();
        let subprocess_env = self.subprocess_env();

        // Per-session mutex: queue concurrent messages instead of rejecting them
        let lock = self.session_lock(&session_key);
        let session_guard = lock.lock_owned().await;

        // Acquire global concurrency permit (after session lock so waiters don't consume slots)
        let _permit = self.queue.acquire(priority).await;

        // Check for existing session
        let existing = self.state_db.get_session(&session_key)?;

        // Read session-level model override from metadata (existing session)
        // or use initial_model (new session from pending)
        let session_model = existing.as_ref().and_then(|r| r.model())
            .or_else(|| initial_model.map(String::from));

        // codex-runtime-plan.md §2.4: cross-runtime resume guard.
        if let Some(row) = existing.as_ref() {
            let stored_runtime = row.runtime_from_metadata().unwrap_or(crate::agent::Runtime::Claude);
            if stored_runtime != agent.runtime {
                return Err(CatClawError::Session(format!(
                    "session was created with runtime={:?} but agent is now {:?} — start a new session",
                    stored_runtime, agent.runtime
                )));
            }
        }

        let (session_id, is_resume) = match existing {
            Some(row) if row.state != "archived" => {
                info!(session_key = %session_key, session_id = %row.session_id, "resuming session (streaming)");
                (row.session_id, true)
            }
            _ => {
                let new_id = Uuid::new_v4().to_string();
                info!(session_key = %session_key, session_id = %new_id, "creating new session (streaming)");

                // Build initial metadata with model, platform IDs, and runtime
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
                    if let Some(ref tid) = sender.thread_id {
                        meta.insert("thread_id".to_string(), serde_json::Value::String(tid.clone()));
                    }
                    // codex-runtime-plan.md §2.4: store runtime so cross-runtime
                    // resume can detect mismatch. Always written for new sessions.
                    meta.insert(
                        "runtime".to_string(),
                        serde_json::Value::String(agent.runtime.as_str().to_string()),
                    );
                    Some(serde_json::Value::Object(meta).to_string())
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

        // Update state to active + register kill channel
        self.state_db
            .update_session_state(&session_key, "active")?;
        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();
        self.active_handles.insert(session_key.clone(), kill_tx);

        // Spawn the agent's runtime with the prompt
        let spawn_params = crate::session::runtime::SpawnParams {
            session_id: &session_id,
            model_override: session_model.as_deref(),
            mcp_port: self.mcp_port,
            hook_session_key: Some(&session_key),
            config_path: self.config_path.as_deref(),
            mcp_env: &mcp_env,
            state_db: Some(&self.state_db),
            is_resume,
            resume_thread_id: None,
        };
        let mut handle = agent.spawn_session(&spawn_params, &actual_message, &subprocess_env).await?;

        // Create channel for streaming events
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<SessionEvent>();

        // Spawn background task to read events and forward them
        let state_db = self.state_db.clone();
        let active_handles = self.active_handles.clone();
        let agent_clone = agent.clone();
        let agent_workspace = agent.workspace.clone();
        let sender_id = sender.sender_id.clone();
        let sender_name = sender.sender_name.clone();
        let message_owned = message.to_string();
        let session_key_owned = session_key.clone();
        let session_id_owned = session_id.clone();
        let diary_trigger = self.diary_trigger.clone();


        // Move permit into the spawned task so concurrency is held until completion
        tokio::spawn(async move {
            let _permit = _permit; // keep permit alive
            let _session_guard = session_guard; // keep session lock until task completes

            let mut kill_rx = kill_rx;

            let mut result_text = String::new();
            let mut got_result_event = false;
            let mut final_session_id = session_id_owned.clone();
            let mut stopped = false;
            let mut tool_uses: Vec<super::transcript::ToolUseEntry> = Vec::new();

            // Open transcript log and write user message (skip for system sessions)
            let is_system = session_key_owned.contains(":system:");
            let mut user_turns_since_marker: u32 = 0;
            let transcript = if is_system {
                None
            } else {
                let label = if !is_resume {
                    Some(super::transcript::label_from_session_key(&session_key_owned))
                } else {
                    None
                };
                let t = TranscriptLog::open_with_label(&agent_workspace, &session_id_owned, label.as_deref()).await.ok();
                if let Some(ref t) = t {
                    if !is_resume {
                        let (ch_type, ch_name) = parse_session_key_channel(&session_key_owned);
                        t.log_session_start(&session_key_owned, ch_type, None, ch_name).await;
                    }
                    user_turns_since_marker =
                        t.log_user(&message_owned, sender_id.as_deref(), sender_name.as_deref()).await;
                }
                t
            };

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
                            RuntimeEvent::SystemInit { session_id } => {
                                info!(session_id = %session_id, "streaming: got session init");
                                if !session_id.is_empty() {
                                    final_session_id = session_id.clone();
                                }
                            }
                            RuntimeEvent::TextDelta { text } => {
                                let _ = event_tx.send(SessionEvent::TextDelta { text: text.clone() });
                            }
                            RuntimeEvent::ToolUseStart { name, input } => {
                                let _ = event_tx.send(SessionEvent::ToolUse { name: name.clone(), input: input.clone() });
                                tool_uses.push(super::transcript::ToolUseEntry {
                                    name: name.clone(),
                                    input: input.clone(),
                                });
                            }
                            RuntimeEvent::ToolResult { .. } => {
                                // Codex-only: ClaudeHandle never emits this. Phase A no-op;
                                // Phase B may surface tool results in transcript / streaming.
                            }
                            RuntimeEvent::Result { result, session_id } => {
                                if !session_id.is_empty() {
                                    final_session_id = session_id.clone();
                                }
                                got_result_event = true;
                                // Only overwrite if non-empty; empty result means
                                // the runtime already sent response via tool use / streaming
                                if !result.is_empty() {
                                    result_text = result.clone();
                                }
                                break;
                            }
                            RuntimeEvent::Assistant { content } => {
                                for block in content {
                                    if let super::claude::ContentBlock::Text(text) = block {
                                        result_text.push_str(text);
                                    }
                                }
                            }
                            RuntimeEvent::StreamEvent { .. } => {
                                // Unrecognized stream events — skip silently
                            }
                            RuntimeEvent::Unknown(_) => {}
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

            // Rolling-diary hook: notify trigger so it can decide whether to
            // fire an extraction based on `user_turns_since_marker`. Skipped
            // for system sessions (no transcript).
            if transcript.is_some() {
                if let Some(trigger) = diary_trigger.as_ref() {
                    trigger
                        .on_turn_complete(
                            &agent_clone,
                            &session_key_owned,
                            &final_session_id,
                            user_turns_since_marker,
                        )
                        .await;
                }
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
            if result_text.is_empty() && !got_result_event {
                // No result event at all — process died unexpectedly
                if is_resume {
                    // Resume failed — likely a stale session
                    // (claude CLI session was deleted but our DB still has it).
                    // Archive it so the next message creates a fresh session.
                    warn!(
                        session_key = %session_key_owned,
                        session_id = %session_id_owned,
                        "resume returned no result event, archiving stale session"
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

    /// Stop all in-flight sessions owned by a given agent.
    ///
    /// Used by `agents.delete` so a delete that races with an in-progress
    /// codex subprocess doesn't leave a zombie holding the `.codex-home/`
    /// auth.json symlink while we try to remove it. Returns the count of
    /// sessions that were actually signalled.
    ///
    /// Note: this only sends the kill signal — the spawned task may still be
    /// shutting down the subprocess when this returns. For `agents.delete`
    /// the caller can proceed immediately because the symlink removal in
    /// `cleanup_codex_home` is tolerant of in-use file handles (the kernel
    /// retains the inode until all FDs close).
    pub fn stop_all_for_agent(&self, agent_id: &str) -> usize {
        // Snapshot the keys first to avoid mutating the map while iterating.
        let prefix = format!("catclaw:{}:", agent_id);
        let to_stop: Vec<String> = self
            .active_handles
            .iter()
            .filter_map(|entry| {
                let k = entry.key();
                if k.starts_with(&prefix) {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect();
        let n = to_stop.len();
        for key in to_stop {
            self.stop_session(&key);
        }
        if n > 0 {
            info!(agent = %agent_id, stopped = n, "stop_all_for_agent");
        }
        n
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
            metadata: Some(
                serde_json::json!({ "runtime": agent.runtime.as_str() }).to_string(),
            ),
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
        self.session_locks.remove(session_key);
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
