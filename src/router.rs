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
        }
    }

    /// Route a message: resolve agent, create/resume session, get response
    pub async fn route(
        &self,
        ctx: &MsgContext,
        adapter: &dyn ChannelAdapter,
    ) -> Result<()> {
        // 1. Start typing indicator
        let _typing = adapter.start_typing(&ctx.channel_id, &ctx.peer_id).await?;

        // 2. Resolve agent
        let agent_id = self.resolve_agent(ctx);
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
        let origin = ctx.channel_type.as_str();
        let context_id = if ctx.is_direct_message {
            format!("dm.{}", ctx.sender_name)
        } else if let Some(ref thread_id) = ctx.thread_id {
            let channel_name = ctx
                .channel_name
                .as_deref()
                .unwrap_or(&ctx.channel_id);
            format!("{}.thread.{}", channel_name, thread_id)
        } else {
            ctx.channel_name
                .clone()
                .unwrap_or_else(|| ctx.channel_id.clone())
        };

        let session_key = SessionKey::new(&agent.id, origin, &context_id);

        // 4. Handle /stop command — kill running session
        let text_trimmed = ctx.text.trim();
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

        // 5. Determine priority
        let priority = if ctx.is_direct_message {
            Priority::Direct
        } else {
            Priority::Mention
        };

        // 6. Build context header + download attachments + compose message
        let context_header = build_context_header(ctx);
        let mut message = format!("{}\n{}", context_header, ctx.text);
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
        };
        let response = self
            .session_manager
            .send_and_wait(&session_key, &agent, &message, priority, &sender, None)
            .await?;

        // 7. Send response back through adapter (chunked if needed)
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
    fn resolve_agent(&self, ctx: &MsgContext) -> String {
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
                    return binding.agent_id.clone();
                }
            }
        }

        self.default_agent_id.clone()
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

    // Download
    match client.get(&att.url).send().await {
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
