use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;
use crate::agent::AgentRegistry;
use crate::channel::{
    split_at_boundaries, Attachment, ChannelAdapter, MsgContext, OutboundMessage,
};
use crate::config::BindingConfig;
use crate::error::Result;
use crate::session::manager::{SenderInfo, SessionManager};
use crate::session::{Priority, SessionKey};

/// Maximum file size to auto-download (50 MB).
const MAX_DOWNLOAD_SIZE: u64 = 50 * 1024 * 1024;

/// Routes inbound messages to the correct agent and session
pub struct MessageRouter {
    session_manager: Arc<SessionManager>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    /// Bindings from catclaw.toml
    bindings: Vec<BindingEntry>,
    default_agent_id: String,
    /// Workspace root for storing downloaded attachments
    workspace: PathBuf,
    /// HTTP client for downloading attachments
    http_client: reqwest::Client,
    /// Adapter map for contact forward mirroring + manual reply (set by gateway).
    adapters: Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
}

#[derive(Debug, Clone)]
struct BindingEntry {
    pattern: String,
    agent_id: String,
    specificity: usize,
}

impl MessageRouter {
    pub fn new(
        session_manager: Arc<SessionManager>,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
        config_bindings: &[BindingConfig],
        default_agent_id: String,
        workspace: PathBuf,
    ) -> Self {
        let mut bindings: Vec<BindingEntry> = config_bindings
            .iter()
            .map(|b| BindingEntry {
                pattern: b.pattern.clone(),
                agent_id: b.agent.clone(),
                specificity: pattern_specificity(&b.pattern),
            })
            .collect();

        // Sort by specificity (most specific first)
        bindings.sort_by(|a, b| b.specificity.cmp(&a.specificity));

        MessageRouter {
            session_manager,
            agent_registry,
            bindings,
            default_agent_id,
            workspace,
            http_client: reqwest::Client::new(),
            adapters: Arc::new(HashMap::new()),
        }
    }

    /// Inject the adapter map (after gateway constructs it). Call once at startup.
    pub fn set_adapters(&mut self, adapters: Arc<HashMap<String, Arc<dyn ChannelAdapter>>>) {
        self.adapters = adapters;
    }

    /// Route a message: resolve agent, create/resume session, get response
    pub async fn route(
        &self,
        ctx: &MsgContext,
        adapter: &dyn ChannelAdapter,
    ) -> Result<()> {
        // 0a. Contact lookup: if this sender is a known contact, mirror inbound
        //     to forward channel and (when ai_paused) skip agent dispatch.
        let db = self.session_manager.state_db();
        let platform = ctx.channel_type.as_str();
        let contact = db
            .get_contact_by_platform_user(platform, &ctx.sender_id)
            .ok()
            .flatten();
        if let Some(ref c) = contact {
            // Touch last_active for last-active routing in pipeline.
            let _ = db.touch_contact_channel(platform, &ctx.sender_id);
            // Mirror inbound to forward channel (best effort).
            let attachments: Vec<String> = ctx
                .attachments
                .iter()
                .map(|a| format!("{} ({})", a.filename, a.url))
                .collect();
            crate::contacts::pipeline::mirror_inbound(
                &self.adapters, c, platform, &ctx.text, attachments,
            )
            .await;
            if c.ai_paused {
                tracing::info!(
                    contact_id = %c.id,
                    sender = %ctx.sender_name,
                    "skipping agent dispatch — contact is ai_paused"
                );
                return Ok(());
            }
        }

        // 0b. Manual reply detection: if this message comes from a forward channel
        //     (admin's monitoring channel), forward it to the corresponding contact
        //     instead of routing through the agent.
        if contact.is_none() {
            // forward_channel format: "{platform}:{channel_id}" or "{platform}:{guild}/{channel_id}"
            if crate::contacts::pipeline::try_manual_reply(
                db,
                &self.adapters,
                platform,
                &ctx.channel_id,
                &ctx.text,
                &ctx.sender_id,
            )
            .await
            .is_some()
            {
                return Ok(());
            }
        }

        // 1. Start typing indicator
        let _typing = adapter.start_typing(&ctx.channel_id, &ctx.peer_id).await?;

        // 2. Resolve agent
        let (agent_id, is_explicit_binding) = self.resolve_agent(ctx);

        // Backend channel requires an explicit binding — never fall through to
        // the default agent, which may have elevated permissions.
        if ctx.channel_type == crate::channel::ChannelType::Backend && !is_explicit_binding {
            tracing::warn!(
                tenant = %ctx.channel_id,
                "backend message rejected: no binding for tenant (configure with: catclaw bind \"backend:channel:{}\" <agent>)",
                ctx.channel_id,
            );
            return Ok(());
        }

        let agent = {
            let registry = self.agent_registry.read().unwrap();
            registry
                .get(&agent_id)
                .or_else(|| registry.default_agent())
                .cloned()
                .ok_or_else(|| {
                    crate::error::CatClawError::Agent(format!("agent '{}' not found", agent_id))
                })?
        };

        // 3. Build session key with human-readable context_id
        // Include guild_id prefix for non-DM channels to prevent collisions
        // when the bot is in multiple servers/workspaces with same-named channels.
        let origin = ctx.channel_type.as_str();
        let guild_prefix = if !ctx.is_direct_message {
            ctx.guild_id.as_deref().unwrap_or("")
        } else {
            ""
        };
        let context_id = if ctx.channel_type == crate::channel::ChannelType::Backend {
            // Backend adapter pre-builds the context_id as peer_id: "{tenant}.user.{uid}"
            ctx.peer_id.clone()
        } else if ctx.is_direct_message {
            format!("dm.{}", ctx.sender_name)
        } else if let Some(ref thread_id) = ctx.thread_id {
            let channel_name = ctx
                .channel_name
                .as_deref()
                .unwrap_or(&ctx.channel_id);
            if guild_prefix.is_empty() {
                format!("{}.thread.{}", channel_name, thread_id)
            } else {
                format!("{}.{}.thread.{}", guild_prefix, channel_name, thread_id)
            }
        } else {
            let channel_name = ctx
                .channel_name
                .clone()
                .unwrap_or_else(|| ctx.channel_id.clone());
            if guild_prefix.is_empty() {
                channel_name
            } else {
                format!("{}.{}", guild_prefix, channel_name)
            }
        };

        let session_key = SessionKey::new(&agent.id, origin, &context_id);

        // 4. Handle /stop and /new commands
        // Strip Telegram bot mention suffix (e.g. "/stop@BotName" → "/stop")
        let text_trimmed = ctx.text.trim().split('@').next().unwrap_or("");
        if text_trimmed == "/stop" {
            let key_str = session_key.to_key_string();
            let stopped = self.session_manager.stop_session(&key_str);
            let reply = if stopped {
                "Session stopped.".to_string()
            } else {
                "No active session to stop.".to_string()
            };
            adapter
                .send(OutboundMessage {
                    channel_type: ctx.channel_type,
                    channel_id: ctx.channel_id.clone(),
                    peer_id: ctx.peer_id.clone(),
                    text: reply,
                    thread_id: ctx.thread_id.clone(),
                    reply_to_message_id: None,
                })
                .await?;
            return Ok(());
        }

        if text_trimmed == "/new" {
            let key_str = session_key.to_key_string();
            let session_row = self
                .session_manager
                .state_db()
                .get_session(&key_str)?;
            let is_active = session_row
                .as_ref()
                .map(|row| row.state != "archived")
                .unwrap_or(false);
            let reply = if is_active {
                // Stop any running process first and wait briefly for cleanup
                if self.session_manager.stop_session(&key_str) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                // Archive immediately so next message starts a fresh session
                self.session_manager.archive(&key_str).await?;

                // Diary extraction in background (doesn't block the user)
                let agent_clone = agent.clone();
                let row = session_row.unwrap(); // safe: is_active implies Some
                let db = self.session_manager.state_db_arc();
                let no_embedder: Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>> = None;
                tokio::spawn(async move {
                    crate::scheduler::extract_diary_for_session(&agent_clone, &row, &db, &no_embedder).await;
                });

                "Session archived. Next message starts a new session.".to_string()
            } else {
                "No active session.".to_string()
            };
            adapter
                .send(OutboundMessage {
                    channel_type: ctx.channel_type,
                    channel_id: ctx.channel_id.clone(),
                    peer_id: ctx.peer_id.clone(),
                    text: reply,
                    thread_id: ctx.thread_id.clone(),
                    reply_to_message_id: None,
                })
                .await?;
            return Ok(());
        }

        // 5. Determine priority
        let priority = if ctx.is_direct_message {
            Priority::Direct
        } else {
            Priority::Mention
        };

        // 6. Build context header + download attachments + compose message
        let mut context_header = build_context_header(ctx);
        if let Some(ref c) = contact {
            let tags_str = if c.tags.is_empty() {
                String::new()
            } else {
                format!(", tags=[{}]", c.tags.join(","))
            };
            let ext_str = if c.external_ref.as_object().map(|m| m.is_empty()).unwrap_or(true) {
                String::new()
            } else {
                format!("\n[Contact external_ref: {}]", c.external_ref)
            };
            let meta_str = if c.metadata.as_object().map(|m| m.is_empty()).unwrap_or(true) {
                String::new()
            } else {
                format!("\n[Contact metadata: {}]", c.metadata)
            };
            context_header = format!(
                "{}\n[Contact: id={}, name={}, role={}{}]{}{}",
                context_header,
                c.id,
                c.display_name,
                c.role.as_str(),
                tags_str,
                ext_str,
                meta_str,
            );
        }
        let reply_line = ctx.reply_to.as_ref().and_then(|r| {
            r.text.as_ref().map(|t| {
                let preview: String = t.chars().take(200).collect();
                format!("[Replying to: \"{}\"]\n", preview)
            })
        }).unwrap_or_default();
        let mut message = format!("{}\n{}{}", context_header, reply_line, ctx.text);
        if !ctx.attachments.is_empty() {
            let att_dir = self.workspace.join("attachments");
            let _ = std::fs::create_dir_all(&att_dir);
            for att in &ctx.attachments {
                let meta = download_attachment(&self.http_client, att, &att_dir).await;
                message.push_str(&format!("\n{}", meta));
            }
        }

        // Send to session and wait for response
        let sender = SenderInfo {
            sender_id: Some(ctx.sender_id.clone()),
            sender_name: Some(ctx.sender_name.clone()),
            channel_id: Some(ctx.channel_id.clone()),
            thread_id: ctx.thread_id.clone(),
        };

        // Set up Discord reaction status indicator if message_id is available
        let reaction_handle = if let Some(ref mid) = ctx.message_id {
            adapter.create_reaction_handle(&ctx.channel_id, mid).await
        } else {
            None
        };

        // Set queued state immediately
        if let Some(ref rh) = reaction_handle {
            rh.set_state(crate::channel::reaction::ReactionState::Queued);
        }

        // Create event observer for reaction status updates
        let event_observer = reaction_handle.as_ref().map(|rh| {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<crate::session::claude::ClaudeEvent>();
            let rh_clone = rh.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    use crate::session::claude::ClaudeEvent;
                    use crate::channel::reaction::{ReactionState, resolve_tool_state};
                    match &event {
                        ClaudeEvent::SystemInit { .. } => {
                            rh_clone.set_state(ReactionState::Thinking);
                        }
                        ClaudeEvent::TextDelta { .. } => {
                            rh_clone.set_state(ReactionState::Thinking);
                        }
                        ClaudeEvent::ToolUseStart { name, .. } => {
                            rh_clone.set_state(resolve_tool_state(name));
                        }
                        ClaudeEvent::StreamEvent { event } => {
                            // Check for thinking_delta
                            if let Some(delta) = event.get("delta") {
                                if delta.get("thinking").is_some() {
                                    rh_clone.set_state(ReactionState::Thinking);
                                }
                            }
                            // Check for context_management compaction
                            if let Some(cm) = event.get("context_management") {
                                if cm.get("applied_edits").and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false) {
                                    rh_clone.set_state(ReactionState::Compacting);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
            tx
        });

        let response = self
            .session_manager
            .send_and_wait(&session_key, &agent, &message, priority, &sender, None, event_observer)
            .await;

        // Signal done or error to reaction controller
        match &response {
            Ok(_) => {
                if let Some(ref rh) = reaction_handle {
                    rh.done();
                }
            }
            Err(_) => {
                if let Some(ref rh) = reaction_handle {
                    rh.error();
                }
            }
        }

        let response = response?;

        // 7. Send response back through adapter (chunked if needed)
        // NO_REPLY is a convention: agent decided no response is needed.
        // Empty response means Claude already sent its reply via tool use
        // (e.g. MCP discord_send_message, discord_upload_file) — nothing to send.
        if response.trim() == "NO_REPLY" || response.trim().is_empty() {
            return Ok(());
        }
        let max_len = adapter.capabilities().max_message_length.saturating_sub(100);
        let chunks = split_at_boundaries(&response, max_len);

        for chunk in chunks {
            adapter
                .send(OutboundMessage {
                    channel_type: ctx.channel_type,
                    channel_id: ctx.channel_id.clone(),
                    peer_id: ctx.peer_id.clone(),
                    text: chunk.to_string(),
                    thread_id: ctx.thread_id.clone(),
                    reply_to_message_id: None,
                })
                .await?;
        }

        Ok(())
    }

    /// Resolve which agent handles this message using binding table
    /// Resolve which agent should handle a message.
    /// Returns (agent_id, is_explicit_binding).
    fn resolve_agent(&self, ctx: &MsgContext) -> (String, bool) {
        let channel_type = ctx.channel_type.as_str();

        // Build candidate patterns from most specific to least
        let candidates = vec![
            // Thread-specific
            ctx.thread_id.as_ref().map(|t| {
                format!("{}:channel:{}:thread:{}", channel_type, ctx.channel_id, t)
            }),
            // Channel-specific
            Some(format!("{}:channel:{}", channel_type, ctx.channel_id)),
            // Guild-specific (from raw_event)
            ctx.raw_event
                .get("guild_id")
                .and_then(|v| v.as_str())
                .map(|g| format!("{}:guild:{}", channel_type, g)),
            // Platform wildcard
            Some(format!("{}:*", channel_type)),
            // Global wildcard
            Some("*".to_string()),
        ];

        // Find the most specific matching binding
        for candidate in candidates.into_iter().flatten() {
            for binding in &self.bindings {
                if binding.pattern == candidate {
                    return (binding.agent_id.clone(), true);
                }
            }
        }

        (self.default_agent_id.clone(), false)
    }
}

/// Format a byte count into a human-readable string (e.g., "12.3 KB", "1.5 MB").
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Build a one-line context header so the agent knows where this message came from.
/// Example: `[Context: discord #chat (1234567890) | sender: Boze (354982148165337089)]`
fn build_context_header(ctx: &MsgContext) -> String {
    let platform = ctx.channel_type.as_str();
    let channel = if ctx.is_direct_message {
        format!("DM with {} ({})", ctx.sender_name, ctx.sender_id)
    } else {
        let name = ctx.channel_name.as_deref().unwrap_or("unknown");
        format!("#{} ({})", name, ctx.channel_id)
    };
    let thread = ctx.thread_id.as_ref()
        .map(|t| format!(" thread:{}", t))
        .unwrap_or_default();
    format!(
        "[Context: {} {}{}| sender: {} ({})]",
        platform, channel, thread, ctx.sender_name, ctx.sender_id
    )
}

/// Download an attachment to the workspace and return a metadata string for the agent.
/// Does NOT read the file content — only provides the local path so the agent can decide.
async fn download_attachment(
    client: &reqwest::Client,
    att: &Attachment,
    att_dir: &Path,
) -> String {
    let size_str = att.size.map(format_file_size).unwrap_or_else(|| "unknown size".into());
    let type_str = att.content_type.as_deref().unwrap_or("unknown");

    // Skip if too large
    if let Some(size) = att.size {
        if size > MAX_DOWNLOAD_SIZE {
            return format!(
                "[Attachment: {} ({}, {})]\n  Status: Too large to download (limit: {}). The user may need to share it another way.",
                att.filename, size_str, type_str, format_file_size(MAX_DOWNLOAD_SIZE)
            );
        }
    }

    // Generate unique local filename: YYYY-MM-DD_{short_uuid}_{original_name}
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    // Sanitize filename: keep only safe chars
    let safe_name: String = att.filename.chars().map(|c| {
        if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
            c
        } else {
            '_'
        }
    }).collect();
    let local_name = format!("{}_{}_{}",date, short_id, safe_name);
    let local_path = att_dir.join(&local_name);

    // Download (with optional auth header for platforms like Slack)
    let mut req = client.get(&att.url);
    if let Some(ref auth) = att.auth_header {
        req = req.header(reqwest::header::AUTHORIZATION, auth);
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.bytes().await {
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&local_path, &bytes) {
                        warn!(error = %e, filename = %att.filename, "failed to save attachment");
                        return format!(
                            "[Attachment: {} ({}, {})]\n  Status: Download failed ({})",
                            att.filename, size_str, type_str, e
                        );
                    }
                    let actual_size = format_file_size(bytes.len() as u64);
                    let abs_path = std::fs::canonicalize(&local_path)
                        .unwrap_or(local_path);
                    format!(
                        "[Attachment: {} ({}, {})]\n  Path: {}",
                        att.filename, actual_size, type_str, abs_path.display()
                    )
                }
                Err(e) => {
                    warn!(error = %e, filename = %att.filename, "failed to read attachment body");
                    format!(
                        "[Attachment: {} ({}, {})]\n  Status: Download failed ({})",
                        att.filename, size_str, type_str, e
                    )
                }
            }
        }
        Ok(resp) => {
            format!(
                "[Attachment: {} ({}, {})]\n  Status: Download failed (HTTP {})",
                att.filename, size_str, type_str, resp.status()
            )
        }
        Err(e) => {
            warn!(error = %e, filename = %att.filename, "failed to download attachment");
            format!(
                "[Attachment: {} ({}, {})]\n  Status: Download failed ({})",
                att.filename, size_str, type_str, e
            )
        }
    }
}

/// Remove attachment files older than the given number of days.
/// Called from the scheduler during archive cleanup.
pub fn cleanup_old_attachments(workspace: &Path, max_age_days: u64) {
    let att_dir = workspace.join("attachments");
    if !att_dir.exists() {
        return;
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(max_age_days * 86400);
    if let Ok(entries) = std::fs::read_dir(&att_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                let modified = meta.modified().unwrap_or(std::time::SystemTime::now());
                if modified < cutoff {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

/// Calculate pattern specificity (more colons = more specific)
fn pattern_specificity(pattern: &str) -> usize {
    if pattern == "*" {
        return 0;
    }
    pattern.matches(':').count() + 1
}
