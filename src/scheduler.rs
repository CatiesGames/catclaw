use std::sync::Arc;

use chrono::{DateTime, Utc};
use croner::Cron;
use tracing::{error, info, warn};

use crate::agent::AgentRegistry;
use crate::session::manager::{SenderInfo, SessionManager};
use crate::session::{Priority, SessionKey};
use crate::state::StateDb;

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
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            heartbeat_enabled: true,
            heartbeat_interval_mins: 30,
            archive_timeout_hours: 168, // 7 days
            archive_check_interval_mins: 360, // 6 hours
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

    loop {
        interval.tick().await;
        let now = Utc::now();

        // ── System: Heartbeat ──
        if config.heartbeat_enabled && now >= next_heartbeat {
            let agents: Vec<_> = agent_registry.read().unwrap().list().into_iter().cloned().collect();
            for agent in &agents {
                execute_heartbeat(agent, &session_manager).await;
            }
            next_heartbeat = now + chrono::Duration::minutes(config.heartbeat_interval_mins as i64);
        }

        // ── System: Archive stale sessions ──
        if now >= next_archive {
            execute_archive(&session_manager, &agent_registry, config.archive_timeout_hours).await;
            next_archive = now + chrono::Duration::minutes(config.archive_check_interval_mins as i64);
        }

        // ── User tasks from DB ──
        if let Err(e) = tick_user_tasks(&state_db, &agent_registry, &session_manager, &now).await {
            error!(error = %e, "scheduler: user task tick failed");
        }
    }
}

/// Process due user tasks from the database
async fn tick_user_tasks(
    state_db: &StateDb,
    agent_registry: &std::sync::RwLock<AgentRegistry>,
    session_manager: &SessionManager,
    now: &DateTime<Utc>,
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
        execute_prompt(&agent, session_manager, prompt, task.id).await;

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
            // One-shot: disable after execution
            info!(task_id = task.id, name = %task.name, "one-shot task completed, disabling");
            state_db.update_task_schedule(task.id, &last_run, &last_run)?;
            state_db.disable_task(task.id)?;
        }
    }

    Ok(())
}

/// Send a heartbeat poll to the agent
async fn execute_heartbeat(agent: &crate::agent::Agent, session_manager: &SessionManager) {
    let key = SessionKey::new(&agent.id, "system", "heartbeat");
    let sender = SenderInfo::default();

    match session_manager
        .send_and_wait(
            &key,
            agent,
            "Heartbeat poll. Read HEARTBEAT.md and follow its instructions.",
            Priority::Heartbeat,
            &sender,
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
async fn execute_prompt(
    agent: &crate::agent::Agent,
    session_manager: &SessionManager,
    prompt: &str,
    task_id: i64,
) {
    // Each task gets its own session keyed by task ID, so context persists across runs
    let context_id = format!("task-{}", task_id);
    let key = SessionKey::new(&agent.id, "system", &context_id);
    let sender = SenderInfo::default();

    match session_manager
        .send_and_wait(&key, agent, prompt, Priority::Cron, &sender, None)
        .await
    {
        Ok(response) => {
            info!(
                agent = %agent.id,
                task_id = task_id,
                response_len = response.trim().len(),
                "user task completed"
            );
        }
        Err(e) => {
            error!(agent = %agent.id, task_id = task_id, error = %e, "user task failed");
        }
    }
}

/// Find and archive stale sessions
async fn execute_archive(
    session_manager: &SessionManager,
    agent_registry: &std::sync::RwLock<AgentRegistry>,
    archive_timeout_hours: u64,
) {
    match session_manager.find_stale_sessions(archive_timeout_hours) {
        Ok(stale) => {
            if stale.is_empty() {
                return;
            }
            info!(count = stale.len(), "archiving stale sessions");
            for session in stale {
                let agent = agent_registry.read().unwrap().get(&session.agent_id).cloned();
                if let Some(agent) = agent {
                    if let Err(e) = session_manager
                        .archive_with_summary(&session.session_key, &agent)
                        .await
                    {
                        error!(
                            session = %session.session_key,
                            error = %e,
                            "failed to archive session"
                        );
                    }
                } else {
                    let _ = session_manager.archive(&session.session_key).await;
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
