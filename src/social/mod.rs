#![allow(dead_code)]

pub mod instagram;
pub mod threads;
pub mod webhook;
pub mod poller;
pub mod forward;

use std::collections::HashMap;
use crate::agent::AgentRegistry;
use crate::config::{InstagramConfig, SocialRule, ThreadsConfig};
use crate::session::manager::{SenderInfo, SessionManager};
use crate::session::{Priority, SessionKey};
use crate::state::{SocialInboxRow, StateDb};
use crate::error::Result;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info, warn};

// ── Platform enum ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialPlatform {
    Instagram,
    Threads,
}

impl SocialPlatform {
    pub fn as_str(&self) -> &'static str {
        match self {
            SocialPlatform::Instagram => "instagram",
            SocialPlatform::Threads => "threads",
        }
    }
}

impl std::fmt::Display for SocialPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── SocialItem ───────────────────────────────────────────────────────────────

/// A normalized social event from Instagram or Threads (comment, mention, reply, message).
#[derive(Debug, Clone)]
pub struct SocialItem {
    pub platform: SocialPlatform,
    /// Platform-assigned event/object ID (used for dedup).
    pub platform_id: String,
    /// "comment" | "mention" | "reply" | "message"
    pub event_type: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    /// Parent post/media ID.
    pub media_id: Option<String>,
    pub text: Option<String>,
    pub metadata: serde_json::Value,
}

// ── ResolvedAction ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ResolvedAction {
    Forward,
    AutoReply { agent: String },
    AutoReplyTemplate { template_name: String, text: String },
    Ignore,
}

// ── Action Router ─────────────────────────────────────────────────────────────

/// Pure rule-matching — no LLM involved.
/// Iterates rules in order; first match wins. Falls back to Ignore if no rule matches.
pub fn resolve_action(
    item: &SocialItem,
    rules: &[SocialRule],
    templates: &HashMap<String, String>,
    default_agent: &str,
) -> ResolvedAction {
    for rule in rules {
        if !rule_matches(rule, item) {
            continue;
        }
        match rule.action.as_str() {
            "forward" => return ResolvedAction::Forward,
            "ignore" => return ResolvedAction::Ignore,
            "auto_reply" => {
                let agent = rule
                    .agent
                    .clone()
                    .unwrap_or_else(|| default_agent.to_string());
                return ResolvedAction::AutoReply { agent };
            }
            "auto_reply_template" => {
                let template_name = rule
                    .template
                    .clone()
                    .unwrap_or_else(|| "default".to_string());
                let text = templates
                    .get(&template_name)
                    .cloned()
                    .unwrap_or_else(|| {
                        warn!(
                            "social rule references unknown template '{}', falling back to Forward",
                            template_name
                        );
                        String::new()
                    });
                if text.is_empty() {
                    // Template missing — fall through to next rule.
                    continue;
                }
                return ResolvedAction::AutoReplyTemplate { template_name, text };
            }
            unknown => {
                warn!(action = unknown, "unknown social rule action, skipping");
            }
        }
    }
    ResolvedAction::Ignore
}

fn rule_matches(rule: &SocialRule, item: &SocialItem) -> bool {
    // match_type: "*" matches everything; otherwise must equal event_type.
    let type_ok = rule.match_type == "*" || rule.match_type == item.event_type;
    if !type_ok {
        return false;
    }
    // Optional keyword filter (case-insensitive substring match on text).
    if let Some(kw) = &rule.keyword {
        let text_lower = item.text.as_deref().unwrap_or("").to_lowercase();
        if !text_lower.contains(&kw.to_lowercase()) {
            return false;
        }
    }
    true
}

// ── Ingest orchestrator ───────────────────────────────────────────────────────

/// Insert-or-ignore the event into the DB.
/// Returns `true` if newly inserted, `false` if duplicate.
pub fn dedup_insert(db: &StateDb, item: &SocialItem) -> Result<bool> {
    let mut row = SocialInboxRow::new(
        item.platform.as_str(),
        &item.platform_id,
        &item.event_type,
    );
    row.author_id = item.author_id.clone();
    row.author_name = item.author_name.clone();
    row.media_id = item.media_id.clone();
    row.text = item.text.clone();
    row.metadata = Some(item.metadata.to_string());
    db.insert_social_inbox(&row)
}

// ── Config helpers ────────────────────────────────────────────────────────────

/// Returns (`ig_rules`, `ig_templates`, `ig_agent`) if instagram config exists.
pub fn instagram_rule_set(cfg: &InstagramConfig) -> (&[SocialRule], &HashMap<String, String>, &str) {
    (&cfg.rules, &cfg.templates, &cfg.agent)
}

/// Returns (`th_rules`, `th_templates`, `th_agent`) if threads config exists.
pub fn threads_rule_set(cfg: &ThreadsConfig) -> (&[SocialRule], &HashMap<String, String>, &str) {
    (&cfg.rules, &cfg.templates, &cfg.agent)
}

// ── Ingest pipeline background task ──────────────────────────────────────────

/// Background task: consumes SocialItems from the channel (webhook or poll),
/// deduplicates via DB, resolves action, dispatches (forward / auto_reply / template).
pub async fn run_ingest(
    mut rx: UnboundedReceiver<SocialItem>,
    db: Arc<StateDb>,
    config: Arc<std::sync::RwLock<crate::config::Config>>,
    adapters: Arc<Vec<Arc<dyn crate::channel::ChannelAdapter>>>,
    session_manager: Arc<SessionManager>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
) {
    while let Some(item) = rx.recv().await {
        let inserted = match dedup_insert(&db, &item) {
            Ok(v) => v,
            Err(e) => {
                error!(platform = %item.platform, id = %item.platform_id, error = %e, "social dedup insert failed");
                continue;
            }
        };
        if !inserted {
            // Duplicate event — already processed.
            continue;
        }

        // Read config inside a limited scope so the RwLock guard is dropped before any await.
        let (action, admin_channel_opt) = {
            let cfg = config.read().unwrap();
            let (action, admin) = match item.platform {
                SocialPlatform::Instagram => {
                    if let Some(ig_cfg) = &cfg.social.instagram {
                        let (rules, templates, default_agent) = instagram_rule_set(ig_cfg);
                        let action = resolve_action(&item, rules, templates, default_agent);
                        (action, Some(ig_cfg.admin_channel.clone()))
                    } else {
                        (ResolvedAction::Ignore, None)
                    }
                }
                SocialPlatform::Threads => {
                    if let Some(th_cfg) = &cfg.social.threads {
                        let (rules, templates, default_agent) = threads_rule_set(th_cfg);
                        let action = resolve_action(&item, rules, templates, default_agent);
                        (action, Some(th_cfg.admin_channel.clone()))
                    } else {
                        (ResolvedAction::Ignore, None)
                    }
                }
            };
            (action, admin)
        };

        let Some(admin_channel) = admin_channel_opt else {
            continue;
        };

        dispatch_action(
            action, item, &db, &config, &adapters,
            &session_manager, &agent_registry, &admin_channel,
        ).await;
    }
}

/// Dispatch a resolved action for a social item.
/// Called by both `run_ingest` (new items) and the reprocess handler (existing items).
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_action(
    action: ResolvedAction,
    item: SocialItem,
    db: &Arc<StateDb>,
    config: &Arc<std::sync::RwLock<crate::config::Config>>,
    adapters: &Arc<Vec<Arc<dyn crate::channel::ChannelAdapter>>>,
    session_manager: &Arc<SessionManager>,
    agent_registry: &Arc<std::sync::RwLock<AgentRegistry>>,
    admin_channel: &str,
) {
    match action {
        ResolvedAction::Ignore => {
            if let Err(e) = db.set_inbox_status(
                &item.platform.to_string(),
                &item.platform_id,
                "ignored",
            ) {
                error!(error = %e, "social: failed to update status to ignored");
            }
        }
        ResolvedAction::Forward => {
            match db.get_social_inbox_by_platform_id(
                &item.platform.to_string(),
                &item.platform_id,
            ) {
                Ok(Some(row)) => {
                    let inbox_id = row.id;
                    let card = forward::build_forward_card(&row);
                    let adapters_ref: &[Arc<dyn crate::channel::ChannelAdapter>] = adapters;
                    match forward::send_forward_card(card, admin_channel, adapters_ref).await {
                        Ok(Some(msg_ref)) => {
                            let _ = db.update_social_inbox_forward_ref(inbox_id, &msg_ref);
                        }
                        Ok(None) => {
                            let _ = db.set_inbox_status(
                                &item.platform.to_string(),
                                &item.platform_id,
                                "forwarded",
                            );
                        }
                        Err(e) => {
                            error!(error = %e, "social: failed to send forward card");
                        }
                    }
                }
                Ok(None) => error!("social: inbox row not found after insert"),
                Err(e) => error!(error = %e, "social: failed to fetch inbox row"),
            }
        }
        ResolvedAction::AutoReplyTemplate { template_name: _, text } => {
            let platform_str = item.platform.to_string();
            let platform_id_clone = item.platform_id.clone();
            let db_clone = db.clone();
            let cfg_clone = config.clone();
            tokio::spawn(async move {
                match send_template_reply(&item, &text, &cfg_clone).await {
                    Ok(reply_id) => {
                        let _ = db_clone.set_inbox_sent(&platform_str, &platform_id_clone, &reply_id);
                    }
                    Err(e) => {
                        error!(error = %e, "social: template reply failed");
                        let _ = db_clone.set_inbox_status(&platform_str, &platform_id_clone, "failed");
                    }
                }
            });
        }
        ResolvedAction::AutoReply { agent } => {
            let db_clone = db.clone();
            let cfg_clone = config.clone();
            let adapters_clone = adapters.clone();
            let sm_clone = session_manager.clone();
            let ar_clone = agent_registry.clone();
            let admin_ch = admin_channel.to_string();
            tokio::spawn(async move {
                if let Err(e) = execute_auto_reply(
                    item, agent, db_clone, cfg_clone, adapters_clone,
                    sm_clone, ar_clone, admin_ch,
                ).await {
                    error!(error = %e, "social: auto_reply failed");
                }
            });
        }
    }
}

// ── Action dispatchers ────────────────────────────────────────────────────────

/// Send a template reply via Meta API.
async fn send_template_reply(
    item: &SocialItem,
    text: &str,
    config: &Arc<std::sync::RwLock<crate::config::Config>>,
) -> Result<String> {
    match item.platform {
        SocialPlatform::Instagram => {
            let (token, user_id) = {
                let cfg = config.read().unwrap();
                let ig = cfg.social.instagram.as_ref()
                    .ok_or_else(|| crate::error::CatClawError::Social("no instagram config".into()))?;
                let token = std::env::var(&ig.token_env)
                    .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
                (token, ig.user_id.clone())
            };
            let client = instagram::InstagramClient::new(token, user_id);
            let resp = client.reply_comment(&item.platform_id, text).await?;
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        SocialPlatform::Threads => {
            let (token, user_id) = {
                let cfg = config.read().unwrap();
                let th = cfg.social.threads.as_ref()
                    .ok_or_else(|| crate::error::CatClawError::Social("no threads config".into()))?;
                let token = std::env::var(&th.token_env)
                    .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", th.token_env)))?;
                (token, th.user_id.clone())
            };
            let client = threads::ThreadsClient::new(token, user_id);
            let resp = client.reply(&item.platform_id, text).await?;
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
    }
}

/// Publish a staged draft via the appropriate Meta API.
/// Returns the platform reply/post ID on success.
pub async fn execute_draft_publish(
    draft: &crate::state::SocialDraftRow,
    config: &crate::config::Config,
) -> Result<String> {
    let reply_to = draft.reply_to_id.as_deref().unwrap_or("");
    match (draft.platform.as_str(), draft.draft_type.as_str()) {
        ("instagram", "reply") => {
            let ig = config.social.instagram.as_ref()
                .ok_or_else(|| crate::error::CatClawError::Social("no instagram config".into()))?;
            let token = std::env::var(&ig.token_env)
                .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
            let resp = instagram::InstagramClient::new(token, ig.user_id.clone())
                .reply_comment(reply_to, &draft.content)
                .await?;
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        ("instagram", "post") => {
            let ig = config.social.instagram.as_ref()
                .ok_or_else(|| crate::error::CatClawError::Social("no instagram config".into()))?;
            let token = std::env::var(&ig.token_env)
                .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
            let client = instagram::InstagramClient::new(token, ig.user_id.clone());
            let resp = match draft.media_urls.len() {
                0 => return Err(crate::error::CatClawError::Social(
                    "instagram post requires at least one image — use instagram_upload_media first".into()
                )),
                1 => client.create_image_post(&draft.media_urls[0], &draft.content).await?,
                _ => {
                    let refs: Vec<&str> = draft.media_urls.iter().map(|s| s.as_str()).collect();
                    client.create_carousel_post(&refs, &draft.content).await?
                }
            };
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        ("instagram", "dm") => {
            let ig = config.social.instagram.as_ref()
                .ok_or_else(|| crate::error::CatClawError::Social("no instagram config".into()))?;
            let token = std::env::var(&ig.token_env)
                .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
            let resp = instagram::InstagramClient::new(token, ig.user_id.clone())
                .send_dm(reply_to, &draft.content)
                .await?;
            Ok(resp.get("message_id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        ("threads", "reply") => {
            let th = config.social.threads.as_ref()
                .ok_or_else(|| crate::error::CatClawError::Social("no threads config".into()))?;
            let token = std::env::var(&th.token_env)
                .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", th.token_env)))?;
            let resp = threads::ThreadsClient::new(token, th.user_id.clone())
                .reply(reply_to, &draft.content)
                .await?;
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        ("threads", "post") => {
            let th = config.social.threads.as_ref()
                .ok_or_else(|| crate::error::CatClawError::Social("no threads config".into()))?;
            let token = std::env::var(&th.token_env)
                .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", th.token_env)))?;
            let client = threads::ThreadsClient::new(token, th.user_id.clone());
            let resp = match draft.media_urls.len() {
                0 => client.create_post(&draft.content).await?,
                1 => client.create_image_post(&draft.media_urls[0], &draft.content).await?,
                _ => {
                    let refs: Vec<&str> = draft.media_urls.iter().map(|s| s.as_str()).collect();
                    client.create_carousel_post(&refs, &draft.content).await?
                }
            };
            Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
        }
        ("threads", "dm") => Err(crate::error::CatClawError::Social(
            "Threads does not support DMs via the public API".to_string()
        )),
        (p, t) => Err(crate::error::CatClawError::Social(format!(
            "execute_draft_publish: unsupported platform='{}' draft_type='{}'", p, t
        ))),
    }
}

/// Remove media_tmp files older than the given number of days.
pub fn cleanup_old_media(workspace: &std::path::Path, max_age_days: u64) {
    let dir = workspace.join("media_tmp");
    if !dir.exists() {
        return;
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(max_age_days * 86400);
    if let Ok(entries) = std::fs::read_dir(&dir) {
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

/// Delete the local media_tmp files referenced by a draft's media_urls.
pub fn cleanup_draft_media(workspace: &std::path::Path, media_urls: &[String]) {
    for url in media_urls {
        if let Some(filename) = url.rsplit('/').next() {
            // Safety: only delete from media_tmp, validate filename
            if !filename.contains("..") && !filename.contains('/') {
                let path = workspace.join("media_tmp").join(filename);
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Spawn a claude session that generates a reply via the publish tool
/// (`instagram_reply_comment` / `threads_reply`), which auto-stages a draft.
#[allow(clippy::too_many_arguments)]
async fn execute_auto_reply(
    item: SocialItem,
    agent_id: String,
    db: Arc<StateDb>,
    _config: Arc<std::sync::RwLock<crate::config::Config>>,
    adapters: Arc<Vec<Arc<dyn crate::channel::ChannelAdapter>>>,
    session_manager: Arc<SessionManager>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    admin_channel: String,
) -> Result<()> {
    // Fetch the inbox row to get the id.
    let row = match db.get_social_inbox_by_platform_id(
        &item.platform.to_string(),
        &item.platform_id,
    )? {
        Some(r) => r,
        None => return Ok(()),
    };

    let inbox_id = row.id;
    let platform_str = item.platform.to_string();
    let (publish_tool, reply_id_param) = match item.platform {
        SocialPlatform::Instagram => ("mcp__catclaw__instagram_reply_comment", "comment_id"),
        SocialPlatform::Threads => ("mcp__catclaw__threads_reply", "reply_to_id"),
    };
    let author = row.author_name.as_deref().unwrap_or("someone");
    let original_text = row.text.as_deref().unwrap_or("(no text)");
    let reply_to_id = &item.platform_id;

    // Mark as auto_replying.
    db.update_social_inbox_session(inbox_id, &format!("social:{}", inbox_id))?;

    // Build system prompt: guide agent to call the publish tool (which auto-stages a draft).
    let system_prompt = format!(
        "You are handling a social media reply task.\n\
         Platform: {platform}\n\
         Event type: {event_type}\n\
         From: @{author}\n\
         Content: {text}\n\n\
         IMPORTANT: You MUST call the `{publish_tool}` MCP tool to reply. Do NOT output text — use the tool.\n\
         Parameters:\n\
         - {reply_id_param}: \"{reply_to_id}\"\n\
         - message (or text): your reply text\n\n\
         The tool auto-stages a draft. It may be auto-approved or may require human review.\n\
         If it requires human review, you will receive a block signal — do NOT retry.",
        platform = platform_str,
        event_type = item.event_type,
        author = author,
        text = original_text,
        publish_tool = publish_tool,
        reply_to_id = reply_to_id,
        reply_id_param = reply_id_param,
    );

    // Look up agent.
    let agent = {
        let registry = agent_registry.read().unwrap();
        registry.get(&agent_id).cloned()
    };
    let agent = match agent {
        Some(a) => a,
        None => {
            warn!(agent_id = %agent_id, "social auto_reply: agent not found, falling back to ignore");
            return Ok(());
        }
    };

    let key = SessionKey::new(agent_id.clone(), "social", format!("{}", inbox_id));
    let sender = SenderInfo {
        sender_id: None,
        sender_name: None,
        channel_id: Some(admin_channel.clone()),
        thread_id: None,
    };

    info!(inbox_id, agent_id = %agent_id, "social auto_reply: spawning session");
    // Cap session time so hanging agents don't accumulate tasks indefinitely.
    let session_result = tokio::time::timeout(
        std::time::Duration::from_secs(300), // 5-minute hard cap
        session_manager.send_and_wait(&key, &agent, &system_prompt, Priority::Channel, &sender, None, None),
    ).await;
    // Helper: restore the forward card to its original state (with buttons) on failure.
    let restore_card = |db: &Arc<StateDb>, adapters: &Arc<Vec<Arc<dyn crate::channel::ChannelAdapter>>>, admin_channel: &str| {
        let db = db.clone();
        let adapters = adapters.clone();
        let admin_channel = admin_channel.to_string();
        async move {
            if let Ok(Some(r)) = db.get_social_inbox(inbox_id) {
                if let Some(ref fwd_ref) = r.forward_ref {
                    let card = forward::build_forward_card(&r);
                    forward::update_forward_card(card, fwd_ref, &admin_channel, &adapters).await;
                }
            }
        }
    };

    if session_result.is_err() {
        warn!(inbox_id, "social auto_reply: session timed out after 5 minutes");
        db.update_social_inbox_status(inbox_id, "forwarded")?;
        restore_card(&db, &adapters, &admin_channel).await;
        return Ok(());
    }

    // After session ends, check for staged drafts.
    let updated_row = db.get_social_inbox(inbox_id)?;
    if let Some(ref r) = updated_row {
        if r.draft.is_some() {
            // Legacy path: draft in inbox table
            let draft = r.draft.as_deref().unwrap();
            let card = forward::build_draft_card(r, draft);
            let adapters_ref: &[Arc<dyn crate::channel::ChannelAdapter>] = &adapters;
            if let Some(msg_ref) = forward::send_forward_card(card, &admin_channel, adapters_ref).await.ok().flatten() {
                let _ = db.update_social_inbox_forward_ref(inbox_id, &msg_ref);
            }
            db.update_social_inbox_status(inbox_id, "draft_ready")?;
            info!(inbox_id, "social auto_reply: draft ready (inbox)");
        } else {
            // Check social_drafts table — draft may have been staged via approval hook
            // (status could be "draft", "awaiting_approval", or "failed")
            let platform_str = item.platform.to_string();
            let has_draft = {
                use rusqlite::OptionalExtension;
                let conn = db.conn.lock().unwrap();
                conn.query_row(
                    "SELECT 1 FROM social_drafts WHERE platform=?1 AND reply_to_id=?2 AND status NOT IN ('sent','ignored') LIMIT 1",
                    rusqlite::params![platform_str, item.platform_id],
                    |_| Ok(true),
                ).optional().unwrap_or(None).is_some()
            };

            if has_draft {
                // Draft exists in social_drafts — don't restore the forward card.
                // The draft review card was already sent by the approval hook flow.
                // Just update inbox status.
                db.update_social_inbox_status(inbox_id, "draft_ready")?;
                info!(inbox_id, "social auto_reply: draft staged via approval hook");
            } else {
                // No draft anywhere — restore card so user can retry.
                warn!(inbox_id, "social auto_reply: session ended without staging a draft");
                db.update_social_inbox_status(inbox_id, "forwarded")?;
                restore_card(&db, &adapters, &admin_channel).await;
            }
        }
    }
    Ok(())
}

// ── DB helper (status + action, by platform_id) ──────────────────────────────

/// Convenience extension so ingest can update by (platform, platform_id) instead of row id.
trait SocialDbExt {
    fn set_inbox_status(
        &self,
        platform: &str,
        platform_id: &str,
        status: &str,
    ) -> Result<()>;

    fn set_inbox_sent(
        &self,
        platform: &str,
        platform_id: &str,
        reply_id: &str,
    ) -> Result<()>;
}

impl SocialDbExt for StateDb {
    fn set_inbox_status(&self, platform: &str, platform_id: &str, status: &str) -> Result<()> {
        self.set_social_inbox_status_by_platform_id(platform, platform_id, status)
    }

    fn set_inbox_sent(&self, platform: &str, platform_id: &str, reply_id: &str) -> Result<()> {
        self.set_social_inbox_sent_by_platform_id(platform, platform_id, reply_id)
    }
}
