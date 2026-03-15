use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::approval::PendingApproval;
use dashmap::DashMap;

use crate::agent::{AgentLoader, AgentRegistry};
use crate::channel::discord::DiscordAdapter;
use crate::channel::telegram::TelegramAdapter;
use crate::channel::{AdapterFilter, ChannelAdapter, MsgContext};
use crate::config::Config;
use crate::error::Result;
use crate::pidfile;
use crate::router::MessageRouter;
use crate::scheduler;
use crate::session::manager::SessionManager;
use crate::state::StateDb;
use crate::ws_server;
use tokio::sync::mpsc as tokio_mpsc;

/// Shared gateway services that TUI (or other in-process consumers) can use.
#[derive(Clone)]
pub struct GatewayHandle {
    pub state_db: Arc<StateDb>,
    pub session_manager: Arc<SessionManager>,
    pub agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    pub adapters: Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    /// Path to the config file (for saving changes).
    pub config_path: PathBuf,
    /// Shared mutable config (for hot-reload via config.set).
    pub config: Arc<std::sync::RwLock<Config>>,
    /// Per-channel adapter filters (indexed by channel config index).
    pub adapter_filters: Vec<Arc<std::sync::RwLock<AdapterFilter>>>,
    /// Pending tool approval requests waiting for user response.
    pub pending_approvals: Arc<DashMap<String, PendingApproval>>,
    /// Broadcast bus for pushing events to all connected WS clients.
    pub event_bus: Arc<tokio::sync::broadcast::Sender<String>>,
}

/// Start gateway services (DB, agents, session manager, channel adapters, scheduler)
/// as background tokio tasks. Returns a handle to the shared services.
///
/// Used by both `catclaw` (with TUI) and `catclaw gateway` (headless).
pub async fn start(config: &Config, config_path: PathBuf) -> Result<GatewayHandle> {
    info!("starting CatClaw gateway");

    // 1. Open state DB
    let state_db = Arc::new(StateDb::open(&config.general.state_db)?);

    // 2. Suspend all previously active sessions (subprocess died on restart)
    let suspended = state_db.suspend_all_active_sessions()?;
    if suspended > 0 {
        info!(count = suspended, "suspended stale sessions from previous run");
    }

    // 3. Load agents
    // Migrate old per-agent skills to shared pool (idempotent)
    if let Err(e) = AgentLoader::migrate_to_shared_skills(&config.general.workspace, &config.agents) {
        warn!(error = %e, "skill migration warning (non-fatal)");
    }

    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(
        &config.agents,
        &config.general.workspace,
        config.general.default_model.as_deref(),
        config.general.default_fallback_model.as_deref(),
    )?));

    let default_agent_id = config
        .default_agent_id()
        .unwrap_or("main")
        .to_string();

    // 4. Create session manager (MCP shares the same port as WS)
    let session_manager = Arc::new(
        SessionManager::new(state_db.clone(), config.general.max_concurrent_sessions)
            .with_mcp_port(config.general.port)
            .with_config_path(config_path.clone()),
    );

    // 5. Load bindings from config
    let router = Arc::new(MessageRouter::new(
        session_manager.clone(),
        agent_registry.clone(),
        &config.bindings,
        default_agent_id,
    ));

    // 6. Create message channel
    let (msg_tx, mut msg_rx) = mpsc::channel::<MsgContext>(256);

    // Create pending_approvals early so we can wire adapter approval channels into it
    let pending_approvals: Arc<DashMap<String, PendingApproval>> = Arc::new(DashMap::new());

    // Collect approval_rx receivers from adapters to wire later
    let mut approval_receivers: Vec<tokio_mpsc::UnboundedReceiver<(String, bool)>> = Vec::new();

    // 7. Start channel adapters
    let mut adapters: Vec<Arc<dyn ChannelAdapter>> = Vec::new();
    let mut adapter_filters: Vec<Arc<std::sync::RwLock<AdapterFilter>>> = Vec::new();

    for channel_config in &config.channels {
        match channel_config.channel_type.as_str() {
            "discord" => {
                let (da, filter) = DiscordAdapter::from_config(channel_config)?;
                adapter_filters.push(filter);
                let adapter = Arc::new(da);

                // Take approval_rx before moving adapter into the start task
                if let Some(rx) = adapter.take_approval_rx().await {
                    approval_receivers.push(rx);
                }

                adapters.push(adapter.clone());

                let tx = msg_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = adapter.start(tx).await {
                        error!(error = %e, "discord adapter error");
                    }
                });

                info!("discord adapter started");
            }
            "telegram" => {
                let (ta, filter) = TelegramAdapter::from_config(channel_config)?;
                adapter_filters.push(filter);
                let adapter = Arc::new(ta);

                // Take approval_rx before moving adapter into the start task
                if let Some(rx) = adapter.take_approval_rx().await {
                    approval_receivers.push(rx);
                }

                adapters.push(adapter.clone());

                let tx = msg_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = adapter.start(tx).await {
                        error!(error = %e, "telegram adapter error");
                    }
                });

                info!("telegram adapter started");
            }
            other => {
                warn!(adapter = other, "unknown channel adapter type, skipping");
            }
        }
    }

    if adapters.is_empty() {
        info!("no channel adapters configured — gateway running in headless mode");
    }

    // Wire adapter approval receivers: forward (request_id, approved) → pending_approvals
    for mut rx in approval_receivers {
        let approvals = pending_approvals.clone();
        tokio::spawn(async move {
            while let Some((request_id, approved)) = rx.recv().await {
                if let Some((_, pa)) = approvals.remove(&request_id) {
                    info!(request_id = %request_id, approved = approved, "channel approval received");
                    let _ = pa.response_tx.send(approved);
                } else {
                    warn!(request_id = %request_id, "approval response for unknown/expired request");
                }
            }
        });
    }

    // 8. Start scheduler
    {
        let sched_config = scheduler::SchedulerConfig {
            heartbeat_enabled: config.heartbeat.as_ref().map_or(false, |h| h.enabled),
            heartbeat_interval_mins: config.heartbeat.as_ref().map_or(30, |h| h.interval_mins),
            archive_timeout_hours: config.general.session_archive_timeout_hours,
            archive_check_interval_mins: 360, // every 6 hours
        };
        let sched_db = state_db.clone();
        let sched_agents = agent_registry.clone();
        let sched_sm = session_manager.clone();
        tokio::spawn(async move {
            scheduler::run(sched_db, sched_agents, sched_sm, sched_config).await;
        });
        info!("scheduler started");
    }

    // 9. Start message router as a background task
    let adapter_map: HashMap<String, Arc<dyn ChannelAdapter>> = adapters
        .iter()
        .map(|a| (a.name().to_string(), a.clone()))
        .collect();
    let adapter_map = Arc::new(adapter_map);

    let router_adapters = adapter_map.clone();
    tokio::spawn(async move {
        info!("gateway message router ready");
        while let Some(msg) = msg_rx.recv().await {
            let router = router.clone();
            let adapter = router_adapters.get(msg.channel_type.as_str()).cloned();

            tokio::spawn(async move {
                if let Some(adapter) = adapter {
                    if let Err(e) = router.route(&msg, adapter.as_ref()).await {
                        error!(
                            error = %e,
                            channel = %msg.channel_type,
                            sender = %msg.sender_name,
                            "failed to route message"
                        );
                    }
                } else {
                    warn!(
                        channel = %msg.channel_type,
                        "no adapter found for channel type"
                    );
                }
            });
        }
        info!("all message senders dropped, router stopping");
    });

    info!("gateway ready");

    let (event_bus_tx, _) = tokio::sync::broadcast::channel::<String>(256);
    let event_bus = Arc::new(event_bus_tx);

    // Approval expiry cleanup task
    {
        let approvals = pending_approvals.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                let expired: Vec<String> = approvals.iter()
                    .filter(|e| e.created_at.elapsed().as_secs() > 300)
                    .map(|e| e.key().clone())
                    .collect();
                for key in expired {
                    if let Some((_, pa)) = approvals.remove(&key) {
                        warn!(request_id = %pa.request_id, "approval request expired");
                        let _ = pa.response_tx.send(false);
                    }
                }
            }
        });
    }

    let handle = GatewayHandle {
        state_db,
        session_manager,
        agent_registry,
        adapters: adapter_map,
        config_path,
        config: Arc::new(std::sync::RwLock::new(config.clone())),
        adapter_filters,
        pending_approvals,
        event_bus,
    };

    // 10. Start gateway server (WS + MCP on single port)
    let server_addr = format!("127.0.0.1:{}", config.general.port);
    ws_server::spawn(server_addr, handle.clone());

    Ok(handle)
}

/// Run gateway in headless (daemon) mode — blocks until SIGTERM/SIGINT.
/// Used by `catclaw gateway`.
pub async fn run(config: Config, config_path: PathBuf) -> Result<()> {
    // Write PID file
    let pid_path = pidfile::pid_path(Some(&config));
    let pid = std::process::id();
    if let Err(e) = pidfile::write_pid(&pid_path, pid) {
        warn!(error = %e, "failed to write PID file");
    } else {
        info!(pid = pid, path = %pid_path.display(), "PID file written");
    }

    // Start all services
    let _handle = start(&config, config_path).await?;

    // Block until shutdown signal
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("received SIGTERM, shutting down gracefully");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("received SIGINT, shutting down gracefully");
        }
    }

    // Cleanup PID file
    pidfile::remove_pid(&pid_path);
    info!("gateway stopped");

    Ok(())
}
