use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use croner::Cron;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

use crate::agent::{Agent, AgentRegistry};
use crate::session::manager::{SenderInfo, SessionManager};
use crate::session::transcript::TranscriptLog;
use crate::session::{Priority, SessionKey};
use crate::state::{SessionRow, StateDb};

/// Tracks session IDs currently undergoing diary extraction to prevent duplicates.
/// Shared between scheduler ticks — if a `claude -p` call takes longer than 60s,
/// the next tick won't start a second extraction for the same session.
type DiaryInFlight = Arc<std::sync::Mutex<std::collections::HashSet<String>>>;

/// Scheduler configuration passed from gateway
pub struct SchedulerConfig {
    /// Whether heartbeat is enabled
    pub heartbeat_enabled: bool,
    /// Heartbeat interval in minutes
    pub heartbeat_interval_mins: u64,
    /// Archive timeout in hours (sessions idle longer than this get archived)
    pub archive_timeout_hours: u64,
    /// Archive check interval in minutes
    pub archive_check_interval_mins: u64,
    /// Workspace root path (for attachment cleanup)
    pub workspace: std::path::PathBuf,
    /// Social polling: send SocialItems into the ingest pipeline.
    pub social_item_tx: Option<Arc<tokio::sync::mpsc::UnboundedSender<crate::social::SocialItem>>>,
    /// Social config (for polling intervals and credentials).
    pub social_config: Option<Arc<std::sync::RwLock<crate::config::Config>>>,
    /// Log directory for scanning ERROR/WARN entries during heartbeat.
    pub log_dir: std::path::PathBuf,
    /// Embedding model for memory palace (lazy-loaded on first use).
    pub embedder: Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            heartbeat_enabled: true,
            heartbeat_interval_mins: 30,
            archive_timeout_hours: 168, // 7 days
            archive_check_interval_mins: 360, // 6 hours
            workspace: std::path::PathBuf::from("./workspace"),
            social_item_tx: None,
            social_config: None,
            log_dir: std::path::PathBuf::from("./workspace/logs"),
            embedder: None,
        }
    }
}

/// Runs the scheduler loop. Ticks every 60 seconds.
///
/// Two categories of tasks:
/// 1. **System tasks** (built-in, not in DB, cannot be deleted):
///    - Heartbeat: periodic poll to each agent via HEARTBEAT.md
///    - Archive: find and archive stale sessions with summaries
/// 2. **User tasks** (stored in `scheduled_tasks` table):
///    - prompt: one-shot, interval, or cron-based prompts sent to an agent
pub async fn run(
    state_db: Arc<StateDb>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    session_manager: Arc<SessionManager>,
    config: SchedulerConfig,
) {
    info!("scheduler started (tick interval: 60s)");

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

    // Track next run times for system tasks (in-memory, not in DB)
    let now = Utc::now();
    let mut next_heartbeat = if config.heartbeat_enabled {
        now + chrono::Duration::minutes(config.heartbeat_interval_mins as i64)
    } else {
        // Far future = never
        DateTime::<Utc>::MAX_UTC
    };
    let mut next_archive = now + chrono::Duration::minutes(config.archive_check_interval_mins as i64);
    let diary_in_flight: DiaryInFlight = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

    // Social polling next-run timestamps (DateTime::MAX = never / not configured).
    // Polling schedule is fixed at gateway startup — mode/interval changes require restart.
    let (ig_poll_interval, th_poll_interval) = {
        if let Some(ref sc) = config.social_config {
            let cfg = sc.read().unwrap();
            let ig = cfg.social.instagram.as_ref()
                .filter(|c| c.mode == "polling")
                .map(|c| c.poll_interval_mins);
            let th = cfg.social.threads.as_ref()
                .filter(|c| c.mode == "polling")
                .map(|c| c.poll_interval_mins);
            (ig, th)
        } else {
            (None, None)
        }
    };
    let mut next_ig_poll = ig_poll_interval
        .map(|m| now + chrono::Duration::minutes(m as i64))
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    let mut next_th_poll = th_poll_interval
        .map(|m| now + chrono::Duration::minutes(m as i64))
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    // Token refresh check runs once per day.
    // Token check: run once at startup (exchange short-lived), then every 24h for IG expiry.
    // Threads refresh cadence (30-day) is tracked in the DB via social_cursors.
    let mut next_token_check = now;

    loop {
        interval.tick().await;
        let now = Utc::now();

        // ── System: Heartbeat ──
        if config.heartbeat_enabled && now >= next_heartbeat {
            let agents: Vec<_> = agent_registry.read().unwrap().list().into_iter().cloned().collect();
            for agent in &agents {
                execute_heartbeat(agent, &session_manager, &config.log_dir).await;
            }
            next_heartbeat = now + chrono::Duration::minutes(config.heartbeat_interval_mins as i64);
        }

        // ── System: Archive stale sessions + clean old attachments ──
        if now >= next_archive {
            execute_archive(&session_manager, config.archive_timeout_hours).await;
            // Clean up downloaded attachments older than archive timeout
            let max_age_days = config.archive_timeout_hours / 24;
            crate::router::cleanup_old_attachments(&config.workspace, max_age_days.max(1));
            crate::social::cleanup_old_media(&config.workspace, max_age_days.max(1));
            next_archive = now + chrono::Duration::minutes(config.archive_check_interval_mins as i64);
        }

        // ── System: Social polling ──
        if now >= next_ig_poll {
            if let (Some(ref tx), Some(ref sc)) = (&config.social_item_tx, &config.social_config) {
                let ig_cfg = sc.read().unwrap().social.instagram.clone();
                if let Some(cfg) = ig_cfg {
                    let tx = Arc::clone(tx);
                    let db = Arc::clone(&state_db);
                    tokio::spawn(async move {
                        match crate::social::poller::poll_instagram(&cfg, &db).await {
                            Ok(items) => {
                                let count = items.len();
                                for item in items { let _ = tx.send(item); }
                                if count > 0 { info!(count, "social poll instagram: new items"); }
                            }
                            Err(e) => warn!(error = %e, "social poll instagram failed"),
                        }
                    });
                }
                next_ig_poll = now + chrono::Duration::minutes(ig_poll_interval.unwrap_or(5) as i64);
            }
        }
        if now >= next_th_poll {
            if let (Some(ref tx), Some(ref sc)) = (&config.social_item_tx, &config.social_config) {
                let th_cfg = sc.read().unwrap().social.threads.clone();
                if let Some(cfg) = th_cfg {
                    let tx = Arc::clone(tx);
                    let db = Arc::clone(&state_db);
                    tokio::spawn(async move {
                        match crate::social::poller::poll_threads(&cfg, &db).await {
                            Ok(items) => {
                                let count = items.len();
                                for item in items { let _ = tx.send(item); }
                                if count > 0 { info!(count, "social poll threads: new items"); }
                            }
                            Err(e) => warn!(error = %e, "social poll threads failed"),
                        }
                    });
                }
                next_th_poll = now + chrono::Duration::minutes(th_poll_interval.unwrap_or(5) as i64);
            }
        }

        // ── System: Social token refresh ──
        if now >= next_token_check {
            if let Some(ref sc) = config.social_config {
                check_token_refresh(sc, &state_db).await;
            }
            next_token_check = now + chrono::Duration::hours(24);
        }

        // ── System: Diary extraction ──
        check_diary_extraction(&state_db, &agent_registry, &diary_in_flight, &config.embedder).await;

        // ── User tasks from DB ──
        if let Err(e) = tick_user_tasks(&state_db, &agent_registry, &session_manager, &now, &config.embedder).await {
            error!(error = %e, "scheduler: user task tick failed");
        }
    }
}

/// Store token expiry in both DB (persists across restarts) and env var (TUI display).
fn set_token_expiry(state_db: &crate::state::StateDb, platform: &str, value: &str) {
    let _ = state_db.upsert_social_cursor(platform, "token_expires_at", value);
    let env_key = format!("CATCLAW_{}_TOKEN_EXPIRES_AT", platform.to_uppercase());
    // Format for human display: "2026-05-25" or "permanent"
    let display = if value == "permanent" {
        "permanent".to_string()
    } else if let Ok(dt) = value.parse::<chrono::DateTime<Utc>>() {
        let days = (dt - Utc::now()).num_days();
        format!("{} ({}d left)", dt.format("%Y-%m-%d"), days)
    } else {
        value.to_string()
    };
    std::env::set_var(&env_key, &display);
}

/// Called at gateway startup: exchange short-lived tokens for long-lived ones.
/// Does NOT refresh Threads long-lived tokens (that's the 30-day scheduler job).
pub async fn startup_token_check(
    config: &std::sync::RwLock<crate::config::Config>,
    state_db: &crate::state::StateDb,
) {
    // Restore cached expiry env vars from DB (so TUI shows them before first API check)
    for platform in &["instagram", "threads"] {
        if let Ok(Some(val)) = state_db.get_social_cursor(platform, "token_expires_at") {
            set_token_expiry(state_db, platform, &val);
        }
    }

    let (ig_cfg, th_cfg) = {
        let cfg = config.read().unwrap();
        (cfg.social.instagram.clone(), cfg.social.threads.clone())
    };

    // Instagram: check expiry and exchange if short-lived.
    if let Some(ig) = ig_cfg {
        if let Ok(current_token) = std::env::var(&ig.token_env) {
            match crate::social::instagram::InstagramClient::check_token_expiry(&current_token).await {
                Ok(0) => {
                    debug!("instagram token: permanent, no action needed");
                    set_token_expiry(state_db, "instagram", "permanent");
                }
                Ok(expires_in) if expires_in < 86400 => {
                    // Short-lived — exchange for long-lived
                    if let (Some(app_id), Some(app_secret_env)) = (&ig.app_id, &ig.app_secret_env) {
                        if let Ok(app_secret) = std::env::var(app_secret_env) {
                            match crate::social::instagram::InstagramClient::exchange_token(app_id, &app_secret, &current_token).await {
                                Ok(new_token) => {
                                    crate::config::write_env_var(&ig.token_env, &new_token);
                                    // Re-check expiry of the new long-lived token
                                    if let Ok(new_exp) = crate::social::instagram::InstagramClient::check_token_expiry(&new_token).await {
                                        let expires_at = Utc::now() + chrono::Duration::seconds(new_exp);
                                        set_token_expiry(state_db, "instagram", &expires_at.to_rfc3339());
                                    }
                                    info!("instagram token: exchanged short-lived for long-lived");
                                }
                                Err(e) => warn!(error = %e, "instagram token exchange failed"),
                            }
                        } else {
                            warn!("instagram token: short-lived but {} not set in env", app_secret_env);
                        }
                    } else {
                        warn!("instagram token: short-lived but app_id/app_secret_env not configured, cannot exchange");
                    }
                }
                Ok(expires_in) => {
                    let expires_at = Utc::now() + chrono::Duration::seconds(expires_in);
                    set_token_expiry(state_db, "instagram", &expires_at.to_rfc3339());
                    debug!("instagram token: long-lived, expires {}", expires_at.format("%Y-%m-%d"));
                }
                Err(e) => warn!(error = %e, "instagram token expiry check failed"),
            }
        }
    }

    // Threads: try exchange short→long, then estimate expiry.
    // Threads API has no debug_token — expiry is estimated from last refresh (60-day lifetime).
    if let Some(th) = th_cfg {
        if let Ok(current_token) = std::env::var(&th.token_env) {
            let mut expiry_set = false;
            if let (Some(app_id), Some(app_secret_env)) = (&th.app_id, &th.app_secret_env) {
                if let Ok(app_secret) = std::env::var(app_secret_env) {
                    match crate::social::threads::ThreadsClient::exchange_token(app_id, &app_secret, &current_token).await {
                        Ok(new_token) if new_token != current_token => {
                            crate::config::write_env_var(&th.token_env, &new_token);
                            let _ = state_db.upsert_social_cursor("threads", "token_refresh_at", &Utc::now().to_rfc3339());
                            let expires_at = Utc::now() + chrono::Duration::days(60);
                            set_token_expiry(state_db, "threads", &expires_at.to_rfc3339());
                            expiry_set = true;
                            info!("threads token: exchanged short-lived for long-lived");
                        }
                        Ok(_) => {
                            // Already long-lived — estimate expiry from last refresh if known
                            if let Ok(Some(last_refresh)) = state_db.get_social_cursor("threads", "token_refresh_at") {
                                if let Ok(dt) = last_refresh.parse::<chrono::DateTime<Utc>>() {
                                    let expires_at = dt + chrono::Duration::days(60);
                                    set_token_expiry(state_db, "threads", &expires_at.to_rfc3339());
                                    expiry_set = true;
                                }
                            }
                            debug!("threads token: exchange returned same token, already long-lived");
                        }
                        Err(e) => debug!(error = %e, "threads token exchange skipped (likely already long-lived)"),
                    }
                }
            }
            // Fallback: if no expiry was set (no app_id, no refresh record, etc.),
            // verify token is valid via /me and assume 60-day lifetime from now.
            if !expiry_set {
                let http = reqwest::Client::new();
                let resp = http.get("https://graph.threads.net/v1.0/me")
                    .query(&[("fields", "id"), ("access_token", &current_token)])
                    .send().await;
                if let Ok(r) = resp {
                    if r.status().is_success() {
                        let expires_at = Utc::now() + chrono::Duration::days(60);
                        set_token_expiry(state_db, "threads", &expires_at.to_rfc3339());
                        debug!("threads token: valid, estimated 60-day expiry");
                    } else {
                        warn!("threads token: /me returned {}, token may be invalid", r.status());
                    }
                }
            }
        }
    }
}

/// Periodic token refresh (called every 24h by the scheduler).
/// - Instagram: check expiry via debug_token API, refresh if < 30 days remaining.
/// - Threads: refresh every 30 days (tracked in social_cursors).
pub async fn check_token_refresh(
    config: &std::sync::RwLock<crate::config::Config>,
    state_db: &crate::state::StateDb,
) {
    let (ig_cfg, th_cfg) = {
        let cfg = config.read().unwrap();
        (cfg.social.instagram.clone(), cfg.social.threads.clone())
    };

    // Instagram: use debug_token to check exact expiry.
    if let Some(ig) = ig_cfg {
        if let Ok(current_token) = std::env::var(&ig.token_env) {
            match crate::social::instagram::InstagramClient::check_token_expiry(&current_token).await {
                Ok(0) => {
                    set_token_expiry(state_db, "instagram", "permanent");
                    debug!("instagram token: permanent, no refresh needed");
                }
                Ok(expires_in) if expires_in < 86400 => {
                    // Still short-lived (exchange hadn't happened yet or failed at startup)
                    if let (Some(app_id), Some(app_secret_env)) = (&ig.app_id, &ig.app_secret_env) {
                        if let Ok(app_secret) = std::env::var(app_secret_env) {
                            match crate::social::instagram::InstagramClient::exchange_token(app_id, &app_secret, &current_token).await {
                                Ok(new_token) => {
                                    crate::config::write_env_var(&ig.token_env, &new_token);
                                    if let Ok(new_exp) = crate::social::instagram::InstagramClient::check_token_expiry(&new_token).await {
                                        let expires_at = Utc::now() + chrono::Duration::seconds(new_exp);
                                        set_token_expiry(state_db, "instagram", &expires_at.to_rfc3339());
                                    }
                                    info!("instagram token: exchanged short-lived for long-lived");
                                }
                                Err(e) => warn!(error = %e, "instagram token exchange failed"),
                            }
                        }
                    }
                }
                Ok(expires_in) if expires_in < 30 * 86400 => {
                    // Long-lived but < 30 days remaining — refresh now
                    match crate::social::instagram::InstagramClient::refresh_token(&current_token).await {
                        Ok(new_token) => {
                            crate::config::write_env_var(&ig.token_env, &new_token);
                            if let Ok(new_exp) = crate::social::instagram::InstagramClient::check_token_expiry(&new_token).await {
                                let expires_at = Utc::now() + chrono::Duration::seconds(new_exp);
                                set_token_expiry(state_db, "instagram", &expires_at.to_rfc3339());
                            }
                            info!(expires_in, "instagram token: refreshed (expiring soon)");
                        }
                        Err(e) => warn!(error = %e, "instagram token refresh failed"),
                    }
                }
                Ok(expires_in) => {
                    let expires_at = Utc::now() + chrono::Duration::seconds(expires_in);
                    set_token_expiry(state_db, "instagram", &expires_at.to_rfc3339());
                    debug!("instagram token: valid, no refresh needed");
                }
                Err(e) => warn!(error = %e, "instagram token expiry check failed"),
            }
        }
    }

    // Threads: no expiry API — refresh every 30 days, tracked in social_cursors.
    if let Some(th) = th_cfg {
        if let Ok(current_token) = std::env::var(&th.token_env) {
            let should_refresh = match state_db.get_social_cursor("threads", "token_refresh_at") {
                Ok(Some(last_refresh_str)) => {
                    match last_refresh_str.parse::<chrono::DateTime<Utc>>() {
                        Ok(last_refresh) => Utc::now() - last_refresh >= chrono::Duration::days(30),
                        Err(_) => true, // Corrupt timestamp — refresh to be safe
                    }
                }
                Ok(None) => true, // Never refreshed — do it now
                Err(e) => {
                    warn!(error = %e, "threads token: could not read last refresh time");
                    false
                }
            };

            if should_refresh {
                match crate::social::threads::ThreadsClient::refresh_token(&current_token).await {
                    Ok(new_token) => {
                        crate::config::write_env_var(&th.token_env, &new_token);
                        let _ = state_db.upsert_social_cursor("threads", "token_refresh_at", &Utc::now().to_rfc3339());
                        let expires_at = Utc::now() + chrono::Duration::days(60);
                        set_token_expiry(state_db, "threads", &expires_at.to_rfc3339());
                        info!("threads token: refreshed (30-day renewal)");
                    }
                    Err(e) => warn!(error = %e, "threads token refresh failed"),
                }
            } else {
                debug!("threads token: not yet due for refresh");
            }
        }
    }
}

/// Process due user tasks from the database
async fn tick_user_tasks(
    state_db: &StateDb,
    agent_registry: &std::sync::RwLock<AgentRegistry>,
    session_manager: &SessionManager,
    now: &DateTime<Utc>,
    embedder: &Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>>,
) -> crate::error::Result<()> {
    let now_str = now.to_rfc3339();
    let due_tasks = state_db.list_due_tasks(&now_str)?;

    for task in due_tasks {
        let agent = match agent_registry.read().unwrap().get(&task.agent_id).cloned() {
            Some(a) => a,
            None => {
                warn!(task_id = task.id, agent_id = %task.agent_id, "agent not found for task, skipping");
                continue;
            }
        };

        info!(
            task_id = task.id,
            name = %task.name,
            agent = %task.agent_id,
            "executing user task"
        );

        let prompt = task.payload.as_deref().unwrap_or("(no prompt specified)");
        execute_prompt(&agent, session_manager, state_db, prompt, task.id, task.keep_context, task.remember, embedder).await;

        // Update schedule: calculate next run or disable if one-shot
        let last_run = now.to_rfc3339();

        if let Some(ref cron_expr) = task.cron_expr {
            match compute_next_cron(cron_expr, now) {
                Some(next) => {
                    state_db.update_task_schedule(task.id, &next.to_rfc3339(), &last_run)?;
                }
                None => {
                    warn!(task_id = task.id, cron = %cron_expr, "invalid cron, disabling task");
                    state_db.disable_task(task.id)?;
                }
            }
        } else if let Some(interval_mins) = task.interval_mins {
            let next = *now + chrono::Duration::minutes(interval_mins);
            state_db.update_task_schedule(task.id, &next.to_rfc3339(), &last_run)?;
        } else {
            // One-shot: delete after execution
            info!(task_id = task.id, name = %task.name, "one-shot task completed, deleting");
            state_db.delete_task(task.id)?;
        }
    }

    Ok(())
}

/// Send a heartbeat poll to the agent.
async fn execute_heartbeat(agent: &crate::agent::Agent, session_manager: &SessionManager, log_dir: &Path) {
    let key = SessionKey::new(&agent.id, "system", "heartbeat");
    let sender = SenderInfo::default();

    // Scan logs for new ERROR/WARN since last heartbeat, merge into issues.json
    let issues_path = agent.workspace.join("memory").join("issues.json");
    let last_ts_path = agent.workspace.join("memory").join(".last_heartbeat_log_ts");
    scan_log_issues(log_dir, &issues_path, &last_ts_path).await;

    let mut message = "Heartbeat poll. Read HEARTBEAT.md and follow its instructions.".to_string();

    // Append open issues if any
    if let Some(issues_text) = open_issues_summary(&issues_path).await {
        info!(agent = %agent.id, "heartbeat: open issues found, appending to message");
        message.push_str(&issues_text);
    }

    match session_manager
        .send_and_wait(
            &key,
            agent,
            &message,
            Priority::Heartbeat,
            &sender,
            None,
            None,
        )
        .await
    {
        Ok(response) => {
            let trimmed = response.trim();
            if trimmed == "HEARTBEAT_OK" {
                info!(agent = %agent.id, "heartbeat OK");
            } else if trimmed == "NO_REPLY" {
                info!(agent = %agent.id, "heartbeat: no action needed");
            } else {
                info!(agent = %agent.id, response_len = trimmed.len(), "heartbeat returned action");
            }
        }
        Err(e) => {
            error!(agent = %agent.id, error = %e, "heartbeat failed");
        }
    }
}

/// Send a user-scheduled prompt to the agent
#[allow(clippy::too_many_arguments)]
async fn execute_prompt(
    agent: &crate::agent::Agent,
    session_manager: &SessionManager,
    state_db: &StateDb,
    prompt: &str,
    task_id: i64,
    keep_context: bool,
    remember: bool,
    embedder: &Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>>,
) {
    let context_id = format!("task-{}", task_id);
    let key = SessionKey::new(&agent.id, "system", &context_id);
    let sender = SenderInfo::default();

    // Default: archive old session so each run starts fresh.
    // With keep_context, the session persists across runs.
    if !keep_context {
        let session_key = key.to_key_string();
        let _ = session_manager.state_db().update_session_state(&session_key, "archived");
    }

    match session_manager
        .send_and_wait(&key, agent, prompt, Priority::Cron, &sender, None, None)
        .await
    {
        Ok(response) => {
            info!(
                agent = %agent.id,
                task_id = task_id,
                response_len = response.trim().len(),
                "user task completed"
            );

            // If remember=true, run diary extraction on this system session.
            // Note: if diary extraction fails here, it won't be retried by the periodic
            // check (which skips origin="system" sessions). The task will produce new
            // content on its next scheduled run.
            if remember {
                let session_key = key.to_key_string();
                if let Ok(Some(session_row)) = state_db.get_session(&session_key) {
                    info!(agent = %agent.id, task_id, "task: running diary extraction (remember=true)");
                    // Convert &StateDb to Arc for extract_diary_for_session
                    let db_arc = session_manager.state_db_arc();
                    extract_diary_for_session(agent, &session_row, &db_arc, embedder).await;
                }
            }
        }
        Err(e) => {
            error!(agent = %agent.id, task_id = task_id, error = %e, "user task failed");
        }
    }
}

/// Find and archive stale sessions
async fn execute_archive(
    session_manager: &SessionManager,
    archive_timeout_hours: u64,
) {
    match session_manager.find_stale_sessions(archive_timeout_hours) {
        Ok(stale) => {
            if stale.is_empty() {
                return;
            }
            info!(count = stale.len(), "archiving stale sessions");
            for session in stale {
                // Diary extraction already happened via check_diary_extraction
                // (idle 30 min trigger), so just archive — no new content to extract.
                if let Err(e) = session_manager.archive(&session.session_key).await {
                    error!(
                        session = %session.session_key,
                        error = %e,
                        "failed to archive session"
                    );
                }
            }
        }
        Err(e) => {
            error!(error = %e, "failed to find stale sessions");
        }
    }
}

/// Compute the next occurrence after `after` for a cron expression
fn compute_next_cron(cron_expr: &str, after: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    let cron = Cron::new(cron_expr).parse().ok()?;
    cron.find_next_occurrence(after, false).ok()
}

// ---------------------------------------------------------------------------
// Layer 1: Diary extraction — transcript → memory/YYYY-MM-DD.md
// ---------------------------------------------------------------------------

/// Minimum idle time (minutes) before a session is eligible for diary extraction
const DIARY_IDLE_MINS: i64 = 30;

/// Minimum number of user turns required for diary extraction
const DIARY_MIN_USER_TURNS: usize = 2;

const DIARY_PROMPT: &str = r#"You are writing a diary entry for an AI agent. Read the following context
to understand who you are, who you talk to, and your existing memories:

# IDENTITY.md
{identity}

# SOUL.md
{soul}

# USER.md
{user}

# Existing Memories (reference only — do not repeat)
{memory_context}

---

Below is a transcript of a recent conversation. Write a diary entry in your
own voice and personality, in the same language used in the conversation.
This is YOUR diary — write from your perspective.

Focus on:
- What the user said that matters (preserve exact words for important statements)
- Their intentions, emotions, priorities
- Decisions made, preferences revealed
- What you did, learned, or found surprising
- Anything to remember for next time

Do NOT write a cold summary or bullet-point log. Write like reflecting on
your day in a personal journal. Let your personality show.

If the conversation has no meaningful content worth recording, reply exactly: NO_DIARY

## Transcript
{transcript}"#;

/// Result of a diary generation attempt
enum DiaryResult {
    /// Diary entry text to append
    Entry(String),
    /// LLM decided nothing worth recording
    NoDiary,
    /// An error occurred
    Error(String),
}

/// Check all idle sessions for diary extraction eligibility and process them.
async fn check_diary_extraction(
    state_db: &Arc<StateDb>,
    agent_registry: &std::sync::RwLock<AgentRegistry>,
    in_flight: &DiaryInFlight,
    embedder: &Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>>,
) {
    let sessions = match state_db.list_sessions() {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "diary: failed to list sessions");
            return;
        }
    };

    let now = Utc::now();
    let cutoff = now - chrono::Duration::minutes(DIARY_IDLE_MINS);
    let cutoff_str = cutoff.to_rfc3339();

    for session in sessions {
        // Only process idle sessions
        if session.state != "idle" {
            continue;
        }

        // Skip system-originated sessions (heartbeat, cron tasks)
        if session.origin == "system" {
            continue;
        }

        // Skip sessions that haven't been idle long enough
        if session.last_activity_at > cutoff_str {
            continue;
        }

        // Skip if already being processed by a previous tick
        {
            let guard = in_flight.lock().unwrap();
            if guard.contains(&session.session_id) {
                continue;
            }
        }

        // Need the agent to get workspace path
        let agent = match agent_registry.read().unwrap().get(&session.agent_id).cloned() {
            Some(a) => a,
            None => continue,
        };

        // Mark as in-flight to prevent duplicate extraction on next tick
        in_flight.lock().unwrap().insert(session.session_id.clone());

        // Spawn background task so diary extraction does not block the scheduler loop
        let agent_clone = agent.clone();
        let session_clone = session.clone();
        let db_clone = state_db.clone();
        let emb_clone = embedder.clone();
        let in_flight_clone = Arc::clone(in_flight);
        let session_id_clone = session.session_id.clone();
        tokio::spawn(async move {
            extract_diary_for_session(&agent_clone, &session_clone, &db_clone, &emb_clone).await;
            // Remove from in-flight set (allows retry on error, or re-check on next tick)
            in_flight_clone.lock().unwrap().remove(&session_id_clone);
        });
    }
}

/// Extract a diary entry from a single session's transcript and store it in the
/// palace DB as a memory node.
///
/// Reads transcript entries since the last diary marker, validates content quality
/// (minimum user turns, meaningful responses), generates a diary via `claude -p`,
/// and writes a `diary_extracted` or `diary_skipped` marker to the transcript.
///
/// Safe to call on sessions in any state (idle, active, archived).
/// Does NOT change session state — caller is responsible for lifecycle management.
pub async fn extract_diary_for_session(
    agent: &Agent,
    session: &SessionRow,
    state_db: &Arc<StateDb>,
    embedder: &Option<Arc<tokio::sync::OnceCell<crate::memory::embed::Embedder>>>,
) {
    let transcript = match TranscriptLog::open_existing(&agent.workspace, &session.session_id).await {
        Some(t) => t,
        None => {
            // No transcript file — nothing to extract
            return;
        }
    };

    let (entries, marker_found) = transcript.read_since_last_marker().await;

    // If a marker already exists and there are no new entries since, skip silently
    if marker_found && entries.is_empty() {
        return;
    }

    // Count user turns
    let user_turns = entries.iter().filter(|e| e.role == "user").count();
    if user_turns < DIARY_MIN_USER_TURNS {
        if !marker_found || !entries.is_empty() {
            transcript
                .log_system(&format!(
                    "diary_skipped (reason: insufficient content, {} user turn{})",
                    user_turns,
                    if user_turns == 1 { "" } else { "s" }
                ))
                .await;
        }
        debug!(session = %session.session_key, user_turns, "diary: skipped, too few user turns");
        return;
    }

    // Check if all assistant responses are NO_REPLY or HEARTBEAT_OK
    let has_meaningful_response = entries.iter().any(|e| {
        e.role == "assistant" && {
            let t = e.content.trim();
            t != "NO_REPLY" && t != "HEARTBEAT_OK"
        }
    });
    if !has_meaningful_response {
        if !marker_found || !entries.is_empty() {
            transcript
                .log_system("diary_skipped (reason: no meaningful assistant responses)")
                .await;
        }
        debug!(session = %session.session_key, "diary: skipped, no meaningful responses");
        return;
    }

    // Build readable transcript and extract channel info for the diary header
    let readable = TranscriptLog::format_readable(&entries);
    let channel_label = build_channel_label(&session.session_key);
    // Format time label in the agent's configured timezone
    let _time_label = entries
        .first()
        .and_then(|e| {
            chrono::DateTime::parse_from_rfc3339(&e.timestamp)
                .ok()
                .map(|dt| {
                    let utc_dt = dt.with_timezone(&Utc);
                    if let Some(ref tz_name) = agent.timezone {
                        if let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() {
                            return utc_dt.with_timezone(&tz).format("%H:%M").to_string();
                        }
                    }
                    utc_dt.format("%H:%M").to_string()
                })
        })
        .unwrap_or_else(|| "??:??".to_string());

    info!(session = %session.session_key, user_turns, "diary: generating diary entry");

    match generate_diary(agent, &readable, state_db).await {
        DiaryResult::Entry(diary_text) => {
            // Derive room from channel label (e.g. "discord dm" → "discord")
            let room = channel_label
                .split_whitespace()
                .next()
                .unwrap_or("general")
                .to_string();

            let req = crate::memory::WriteRequest {
                wing: agent.id.clone(),
                room,
                hall: "events".to_string(),
                content: diary_text,
                summary: None,
                source: "diary".to_string(),
                importance: Some(5),
            };

            let diary_node_id = match state_db.memory_write(&req) {
                Ok(id) => id,
                Err(e) => {
                    error!(error = %e, "diary: failed to write to palace DB");
                    return;
                }
            };

            transcript.log_system("diary_extracted").await;
            info!(
                agent = %agent.id,
                session = %session.session_key,
                diary_node_id,
                "diary: entry written to palace DB"
            );

            // Haiku post-processing: extract summary, room, facts, KG triples (background)
            let wing = agent.id.clone();
            let text = req.content.clone();
            let emb = embedder.clone();
            let db = state_db.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::memory::analyze::analyze_diary(
                    &db, emb.as_ref(), &wing, diary_node_id, &text,
                ).await {
                    warn!(error = %e, "diary: haiku analysis failed (non-fatal)");
                }
            });
        }
        DiaryResult::NoDiary => {
            transcript
                .log_system("diary_skipped (reason: LLM returned NO_DIARY)")
                .await;
            debug!(session = %session.session_key, "diary: LLM said NO_DIARY");
        }
        DiaryResult::Error(e) => {
            error!(session = %session.session_key, error = %e, "diary: generation failed");
            // Don't write a marker on error — will retry next tick
        }
    }
}

/// Generate a diary entry by calling claude -p with the agent's personality context.
async fn generate_diary(agent: &Agent, transcript_text: &str, state_db: &StateDb) -> DiaryResult {
    // Read personality files
    let identity = read_file_or_empty(&agent.workspace.join("IDENTITY.md")).await;
    let soul = read_file_or_empty(&agent.workspace.join("SOUL.md")).await;
    let user_md = read_file_or_empty(&agent.workspace.join("USER.md")).await;

    // Load L1 context from palace DB (replaces MEMORY.md)
    let memory_context = crate::memory::context::build_l1_context(state_db, &agent.id, 2000)
        .unwrap_or_default();

    // Build the prompt from template
    let prompt = DIARY_PROMPT
        .replace("{identity}", &identity)
        .replace("{soul}", &soul)
        .replace("{user}", &user_md)
        .replace("{memory_context}", &memory_context)
        .replace("{transcript}", transcript_text);

    // Call Haiku for diary generation — doesn't need deep reasoning,
    // just personality-aware journaling. Personality context is in the prompt.
    let result = Command::new("claude")
        .args([
            "-p",
            &prompt,
            "--model",
            "claude-haiku-4-5-20251001",
            "--max-turns",
            "1",
            "--output-format",
            "text",
            "--dangerously-skip-permissions",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CLAUDECODE")
        .output()
        .await;

    match result {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return DiaryResult::Error(format!("claude exited with {}: {}", output.status, stderr));
            }
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text == "NO_DIARY" {
                DiaryResult::NoDiary
            } else if text.is_empty() {
                DiaryResult::Error("claude returned empty output".to_string())
            } else {
                DiaryResult::Entry(text)
            }
        }
        Err(e) => DiaryResult::Error(format!("failed to spawn claude: {}", e)),
    }
}

/// Build a human-readable channel label from a session key.
/// `catclaw:agent:discord:dm-123` → `discord dm`
/// `catclaw:agent:telegram:group-456` → `telegram group`
fn build_channel_label(session_key: &str) -> String {
    let parts: Vec<&str> = session_key.splitn(4, ':').collect();
    if parts.len() >= 4 {
        let origin = parts[2];
        let context = parts[3];
        // Extract the context type (before the first '-' or the whole thing)
        let context_type = context.split('-').next().unwrap_or(context);
        format!("{} {}", origin, context_type)
    } else {
        "unknown".to_string()
    }
}

/// Read a file to string, returning empty string on any error
async fn read_file_or_empty(path: &Path) -> String {
    tokio::fs::read_to_string(path)
        .await
        .unwrap_or_default()
}


// Memory distillation (Layer 2) has been replaced by the palace DB.
// Diary entries are now stored directly in state.db via memory_write().
// L1 context is generated dynamically from high-importance memories.
// The old check_distillation_due() and .last_distill tracking have been removed.

// ---------------------------------------------------------------------------
// Log issue tracking — scan ERROR/WARN logs, persist to issues.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogIssue {
    /// Unique ID: hash of (level, target, msg) truncated to 8 hex chars
    pub id: String,
    pub level: String,
    pub target: String,
    pub msg: String,
    pub first_seen: String,
    pub last_seen: String,
    pub count: u32,
    /// "open" | "ignored"
    pub status: String,
}

impl LogIssue {
    fn key(level: &str, target: &str, msg: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // All warn!/error! calls use structured fields for dynamic values, so msg
        // should be fully static. As a safety net, strip everything after ": " in
        // case any call site still embeds a dynamic value directly in the message.
        let msg_prefix = msg.split_once(": ").map(|(p, _)| p).unwrap_or(msg);
        let mut h = DefaultHasher::new();
        level.hash(&mut h);
        target.hash(&mut h);
        msg_prefix.hash(&mut h);
        format!("{:016x}", h.finish())[..8].to_string()
    }
}

async fn load_issues(path: &Path) -> Vec<LogIssue> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

async fn save_issues(path: &Path, issues: &[LogIssue]) {
    if let Ok(s) = serde_json::to_string_pretty(issues) {
        if let Err(e) = tokio::fs::write(path, s).await {
            warn!(error = %e, "failed to write issues.json");
        }
    }
}

/// Scan today's (and yesterday's, if ts straddles midnight) log file for ERROR/WARN
/// entries newer than `.last_heartbeat_log_ts`. Merge into issues.json (dedup by key).
/// Updates `.last_heartbeat_log_ts` to now after scanning.
async fn scan_log_issues(log_dir: &Path, issues_path: &Path, last_ts_path: &Path) {
    let last_ts = tokio::fs::read_to_string(last_ts_path).await.unwrap_or_default();
    let last_ts = last_ts.trim().to_string();

    // Collect candidate log files: today + yesterday (handles midnight boundary)
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();

    let mut new_entries: Vec<crate::logging::LogRecord> = Vec::new();
    for date in &[&yesterday, &today] {
        let path = log_dir.join(format!("catclaw-{}.jsonl", date));
        let records = crate::logging::read_log_file(&path);
        for r in records {
            if matches!(r.level.as_str(), "ERROR" | "WARN")
                && (last_ts.is_empty() || r.ts.as_str() > last_ts.as_str()) {
                new_entries.push(r);
            }
        }
    }

    let mut issues = load_issues(issues_path).await;
    let now_ts = chrono::Utc::now().to_rfc3339();

    // Build set of issue keys seen in this scan
    let seen_ids: std::collections::HashSet<String> = new_entries
        .iter()
        .map(|r| LogIssue::key(&r.level, &r.target, &r.msg))
        .collect();

    // Auto-remove open issues that did NOT appear in this scan period
    issues.retain(|i| i.status == "ignored" || seen_ids.contains(&i.id));

    for record in &new_entries {
        let id = LogIssue::key(&record.level, &record.target, &record.msg);
        if let Some(existing) = issues.iter_mut().find(|i| i.id == id && i.status == "open") {
            existing.last_seen = record.ts.clone();
            existing.count += 1;
        } else if !issues.iter().any(|i| i.id == id && i.status == "ignored") {
            // Only add if not already ignored; ignored = suppress forever
            issues.push(LogIssue {
                id,
                level: record.level.clone(),
                target: record.target.clone(),
                msg: record.msg.clone(),
                first_seen: record.ts.clone(),
                last_seen: record.ts.clone(),
                count: 1,
                status: "open".to_string(),
            });
        }
    }

    save_issues(issues_path, &issues).await;
    let _ = tokio::fs::write(last_ts_path, &now_ts).await;
}

/// Build a summary of open issues to append to the heartbeat message.
/// Returns None if there are no open issues.
async fn open_issues_summary(issues_path: &Path) -> Option<String> {
    let issues = load_issues(issues_path).await;
    let open: Vec<&LogIssue> = issues.iter().filter(|i| i.status == "open").collect();
    if open.is_empty() {
        return None;
    }

    let mut lines = vec![
        "\n\nOPEN ISSUES (from system logs — do NOT reply HEARTBEAT_OK until addressed):".to_string(),
        "Each issue is in memory/issues.json. To resolve or ignore, use Bash to update the status field.".to_string(),
        "".to_string(),
    ];
    for issue in &open {
        lines.push(format!(
            "[{}] {} | {} | {} (seen {} time{}, last: {})",
            issue.id,
            issue.level,
            issue.target,
            issue.msg,
            issue.count,
            if issue.count == 1 { "" } else { "s" },
            &issue.last_seen[..19.min(issue.last_seen.len())],
        ));
    }
    lines.push("".to_string());
    lines.push("To resolve: delete the entry from memory/issues.json.".to_string());
    lines.push("To ignore forever: set status to 'ignored' in memory/issues.json.".to_string());
    Some(lines.join("\n"))
}
