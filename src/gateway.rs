use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::approval::PendingApproval;
use dashmap::DashMap;

use crate::agent::{AgentLoader, AgentRegistry};
use crate::channel::discord::DiscordAdapter;
use crate::channel::slack::SlackAdapter;
use crate::channel::telegram::TelegramAdapter;
use crate::channel::{AdapterFilter, ChannelAdapter, MsgContext};
use crate::config::Config;
use crate::error::Result;
use crate::pidfile;
use crate::router::MessageRouter;
use crate::scheduler;
use crate::session::manager::SessionManager;
use crate::state::StateDb;
use crate::mcp_discovery;
use crate::ws_server;
use tokio::sync::mpsc as tokio_mpsc;

/// Shared gateway services that TUI (or other in-process consumers) can use.
#[derive(Clone)]
pub struct GatewayHandle {
    pub state_db: Arc<StateDb>,
    pub session_manager: Arc<SessionManager>,
    pub agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    pub adapters: Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    /// Ordered list of all active adapters (used by social forward cards and auto_reply).
    #[allow(dead_code)]
    pub adapters_list: Arc<Vec<Arc<dyn ChannelAdapter>>>,
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
    /// Discovered MCP tools per server (populated at startup).
    /// In-memory only; re-discovered on each gateway restart.
    pub mcp_tools: Arc<std::sync::RwLock<HashMap<String, Vec<String>>>>,
    /// Channel to inject social SocialItems into the ingest pipeline (webhook + manual poll).
    pub social_item_tx: Arc<tokio::sync::mpsc::UnboundedSender<crate::social::SocialItem>>,
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
        config.general.timezone.as_deref(),
    )?));

    let default_agent_id = config
        .default_agent_id()
        .unwrap_or("main")
        .to_string();

    // 4. Create session manager (MCP shares the same port as WS)
    let gw_config = Arc::new(std::sync::RwLock::new(config.clone()));
    let session_manager = Arc::new(
        SessionManager::new(state_db.clone(), config.general.max_concurrent_sessions)
            .with_mcp_port(config.general.port)
            .with_config_path(config_path.clone())
            .with_config(gw_config.clone()),
    );

    // 5. Load bindings from config
    let router = Arc::new(MessageRouter::new(
        session_manager.clone(),
        agent_registry.clone(),
        &config.bindings,
        default_agent_id,
        config.general.workspace.clone(),
    ));

    // 6. Create message channel
    let (msg_tx, mut msg_rx) = mpsc::channel::<MsgContext>(256);

    // Create pending_approvals early so we can wire adapter approval channels into it
    let pending_approvals: Arc<DashMap<String, PendingApproval>> = Arc::new(DashMap::new());

    // Collect approval_rx receivers from adapters to wire later
    let mut approval_receivers: Vec<tokio_mpsc::UnboundedReceiver<(String, bool)>> = Vec::new();
    // Collect social_action_rx receivers from adapters
    let mut social_action_receivers: Vec<tokio_mpsc::UnboundedReceiver<(i64, String, Option<String>)>> = Vec::new();

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
                if let Some(rx) = adapter.take_social_action_rx().await {
                    social_action_receivers.push(rx);
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
                if let Some(rx) = adapter.take_social_action_rx().await {
                    social_action_receivers.push(rx);
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
            "slack" => {
                let (sa, filter) = SlackAdapter::from_config(channel_config)?;
                adapter_filters.push(filter);
                let adapter = Arc::new(sa);

                // Take approval_rx before moving adapter into the start task
                if let Some(rx) = adapter.take_approval_rx().await {
                    approval_receivers.push(rx);
                }
                if let Some(rx) = adapter.take_social_action_rx().await {
                    social_action_receivers.push(rx);
                }

                adapters.push(adapter.clone());

                let tx = msg_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = adapter.start(tx).await {
                        error!(error = %e, "slack adapter error");
                    }
                });

                info!("slack adapter started");
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

    // Create social ingest channel early (used by both scheduler and ingest task).
    let (social_item_tx_raw, social_item_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::social::SocialItem>();
    let social_item_tx = Arc::new(social_item_tx_raw);

    // 8. Start scheduler
    {
        let sched_config = scheduler::SchedulerConfig {
            heartbeat_enabled: config.heartbeat.as_ref().is_some_and(|h| h.enabled),
            heartbeat_interval_mins: config.heartbeat.as_ref().map_or(30, |h| h.interval_mins),
            archive_timeout_hours: config.general.session_archive_timeout_hours,
            archive_check_interval_mins: 360, // every 6 hours
            workspace: config.general.workspace.clone(),
            social_item_tx: Some(social_item_tx.clone()),
            social_config: Some(gw_config.clone()),
            log_dir: config.logging.resolve_log_dir(&config.general.workspace),
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
    let adapters_list = Arc::new(adapters.clone());
    let adapter_map: HashMap<String, Arc<dyn ChannelAdapter>> = adapters
        .iter()
        .map(|a| (a.name().to_string(), a.clone()))
        .collect();
    let adapter_map = Arc::new(adapter_map);

    let (event_bus_tx, _) = tokio::sync::broadcast::channel::<String>(256);
    let event_bus = Arc::new(event_bus_tx);

    let router_adapters = adapter_map.clone();
    let router_event_bus = event_bus.clone();
    tokio::spawn(async move {
        info!("gateway message router ready");
        while let Some(msg) = msg_rx.recv().await {
            let router = router.clone();
            let adapter = router_adapters.get(msg.channel_type.as_str()).cloned();
            let bus = router_event_bus.clone();

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
                    // Notify TUI that sessions may have changed
                    let _ = bus.send(
                        serde_json::to_string(&crate::ws_protocol::WsEvent {
                            event: "session.updated".to_string(),
                            data: serde_json::json!({}),
                        }).unwrap_or_default()
                    );
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

    // Check for pending update notification (written by `catclaw update --notify`)
    if let Some(notify) = crate::dist::read_and_clear_pending_notify() {
        let notify_adapters = adapter_map.clone();
        tokio::spawn(async move {
            // Wait for adapters to fully connect (WS handshake, auth, etc.)
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            if let Some(adapter) = notify_adapters.get(&notify.channel_type) {
                let msg = crate::channel::OutboundMessage {
                    channel_type: crate::channel::ChannelType::Tui,
                    channel_id: notify.channel_id.clone(),
                    peer_id: notify.channel_id,
                    text: notify.message,
                    thread_id: None,
                    reply_to_message_id: None,
                };
                if let Err(e) = adapter.send(msg).await {
                    error!(error = %e, "failed to send pending update notification");
                } else {
                    info!("sent pending update notification");
                }
            } else {
                warn!(channel_type = %notify.channel_type, "no adapter found for pending notification");
            }
        });
    }

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

    // Start social ingest pipeline (receives SocialItems from webhook + polling, deduplicates,
    // resolves action, dispatches forward/auto_reply/template).
    {
        let ingest_db = state_db.clone();
        let ingest_config = gw_config.clone();
        let ingest_adapters = adapters_list.clone();
        let ingest_sm = session_manager.clone();
        let ingest_ar = agent_registry.clone();
        tokio::spawn(crate::social::run_ingest(
            social_item_rx,
            ingest_db,
            ingest_config,
            ingest_adapters,
            ingest_sm,
            ingest_ar,
        ));
        info!("social ingest pipeline started");
    }

    // Startup: clear media_tmp dir (temp files don't survive restarts).
    {
        let media_dir = config.general.workspace.join("media_tmp");
        if media_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&media_dir) {
                let mut count = 0usize;
                for entry in entries.flatten() {
                    if std::fs::remove_file(entry.path()).is_ok() {
                        count += 1;
                    }
                }
                if count > 0 {
                    info!(count, "startup: cleared media_tmp files");
                }
            }
        }
    }

    // Startup token check: exchange short-lived tokens for long-lived ones.
    {
        let token_config = gw_config.clone();
        let token_db = state_db.clone();
        tokio::spawn(async move {
            crate::scheduler::startup_token_check(&token_config, &token_db).await;
        });
    }

    // Startup catchup poll: regardless of mode, run one poll on launch to recover
    // any events that arrived while the gateway was offline (webhook gap recovery).
    {
        let startup_config = gw_config.read().unwrap().clone();
        let startup_tx = social_item_tx.clone();
        let startup_db = state_db.clone();
        tokio::spawn(async move {
            let ig_cfg = startup_config.social.instagram.clone();
            let th_cfg = startup_config.social.threads.clone();

            if let Some(cfg) = ig_cfg.filter(|c| c.mode == "webhook" || c.mode == "polling") {
                match crate::social::poller::poll_instagram(&cfg, &startup_db).await {
                    Ok(items) => {
                        let count = items.len();
                        for item in items { let _ = startup_tx.send(item); }
                        if count > 0 { info!(count, "startup catchup: instagram"); }
                    }
                    Err(e) => warn!(error = %e, "startup catchup: instagram poll failed"),
                }
            }
            if let Some(cfg) = th_cfg.filter(|c| c.mode == "webhook" || c.mode == "polling") {
                match crate::social::poller::poll_threads(&cfg, &startup_db).await {
                    Ok(items) => {
                        let count = items.len();
                        for item in items { let _ = startup_tx.send(item); }
                        if count > 0 { info!(count, "startup catchup: threads"); }
                    }
                    Err(e) => warn!(error = %e, "startup catchup: threads poll failed"),
                }
            }
        });
    }

    // Wire social action receivers (button presses from adapters → approve/ignore/auto_reply handlers).
    for mut rx in social_action_receivers {
        let sa_db = state_db.clone();
        let sa_config = gw_config.clone();
        let sa_adapters = adapters_list.clone();
        let sa_sm = session_manager.clone();
        let sa_ar = agent_registry.clone();
        tokio::spawn(async move {
            while let Some((inbox_id, action, hint)) = rx.recv().await {
                handle_social_button_action(
                    inbox_id, &action, hint.as_deref(), &sa_db, &sa_config, &sa_adapters, &sa_sm, &sa_ar,
                ).await;
            }
        });
    }

    // Discover MCP tools from user .mcp.json servers (non-blocking, best-effort)
    let mcp_tools: Arc<std::sync::RwLock<HashMap<String, Vec<String>>>> =
        Arc::new(std::sync::RwLock::new(HashMap::new()));
    {
        let mcp_json_path = config.general.workspace.join(".mcp.json");
        let mcp_env = config.mcp_env.clone();
        let tools_ref = mcp_tools.clone();
        tokio::spawn(async move {
            let results = mcp_discovery::discover_all(&mcp_json_path, &mcp_env).await;
            let count = results.len();
            let mut map = tools_ref.write().unwrap();
            for entry in results {
                info!(server = %entry.server_name, tools = entry.tools.len(), "MCP tools discovered");
                map.insert(entry.server_name, entry.tools);
            }
            if count == 0 {
                info!("MCP discovery: no tools discovered (0 servers or all failed)");
            } else {
                info!(total_servers = count, "MCP discovery complete");
            }
        });
    }

    let handle = GatewayHandle {
        state_db,
        session_manager,
        agent_registry,
        adapters: adapter_map,
        adapters_list,
        config_path,
        config: gw_config,
        adapter_filters,
        pending_approvals,
        event_bus,
        mcp_tools,
        social_item_tx,
    };

    // 10. Start gateway server (WS + MCP on single port)
    let server_addr = format!("{}:{}", config.general.bind_addr, config.general.port);
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

// ── Social button action dispatcher ──────────────────────────────────────────

/// Handle a social button press from any adapter.
/// For inbox actions, `card_id` is the social_inbox.id.
/// For draft actions (prefix "draft_"), `card_id` is the social_drafts.id.
/// `hint` is an optional user-provided hint string for ai_reply_hint flows.
#[allow(clippy::too_many_arguments)]
async fn handle_social_button_action(
    card_id: i64,
    action: &str,
    hint: Option<&str>,
    db: &Arc<StateDb>,
    config: &Arc<std::sync::RwLock<Config>>,
    adapters: &Arc<Vec<Arc<dyn ChannelAdapter>>>,
    session_manager: &Arc<SessionManager>,
    agent_registry: &Arc<std::sync::RwLock<crate::agent::AgentRegistry>>,
) {
    use crate::social::{dispatch_action, forward, ResolvedAction, SocialItem, SocialPlatform};

    // ── Draft button actions (social_draft: prefix → "draft_approve" / "draft_discard") ──
    if action == "draft_approve" || action == "draft_discard" {
        let draft = match db.get_social_draft(card_id) {
            Ok(Some(d)) => d,
            Ok(None) => {
                warn!(card_id, action, "social draft button: draft not found");
                return;
            }
            Err(e) => {
                error!(card_id, error = %e, "social draft button: db error");
                return;
            }
        };

        // Resolve admin_channel
        let admin_channel = {
            let cfg = config.read().unwrap();
            match draft.platform.as_str() {
                "instagram" => cfg.social.instagram.as_ref().map(|c| c.admin_channel.clone()),
                "threads" => cfg.social.threads.as_ref().map(|c| c.admin_channel.clone()),
                _ => None,
            }.unwrap_or_default()
        };

        let try_update_draft_card = |card: forward::ForwardCard| {
            let fwd_ref = draft.forward_ref.clone();
            let ch = admin_channel.clone();
            let ads = adapters.clone();
            async move {
                if let (Some(msg_ref), false) = (fwd_ref, ch.is_empty()) {
                    forward::update_forward_card(card, &msg_ref, &ch, &ads).await;
                }
            }
        };

        if action == "draft_discard" {
            if draft.status == "sent" {
                warn!(card_id, status = %draft.status, "social draft_discard: already sent, cannot discard");
                return;
            }
            info!(card_id, platform = %draft.platform, "social draft_discard: deleted");
            let workspace = config.read().unwrap().general.workspace.clone();
            crate::social::cleanup_draft_media(&workspace, draft.media_url.as_deref());
            let base = forward::build_social_draft_card(&draft);
            let resolved = forward::build_resolved_card(&base, "已捨棄");
            try_update_draft_card(resolved).await;
            let _ = db.delete_social_draft(card_id);
            return;
        }

        // draft_approve: idempotency guard — allow awaiting_approval/draft/failed (retry)
        if draft.status != "awaiting_approval" && draft.status != "draft" && draft.status != "failed" {
            warn!(card_id, status = %draft.status, "social draft_approve: already resolved");
            return;
        }

        // Show "publishing..." state immediately, then spawn background task for API call.
        // This prevents blocking the button handler loop while waiting for Meta API.
        let cfg = config.read().unwrap().clone();
        let base = forward::build_social_draft_card(&draft);
        let publishing = forward::build_publishing_card(&base);
        try_update_draft_card(publishing).await;

        let db = db.clone();
        let adapters = adapters.clone();
        tokio::spawn(async move {
            let try_update = |card: forward::ForwardCard| {
                let fwd_ref = draft.forward_ref.clone();
                let ch = admin_channel.clone();
                let ads = adapters.clone();
                async move {
                    if let (Some(msg_ref), false) = (fwd_ref, ch.is_empty()) {
                        forward::update_forward_card(card, &msg_ref, &ch, &ads).await;
                    }
                }
            };
            match crate::social::execute_draft_publish(&draft, &cfg).await {
                Ok(reply_id) => {
                    info!(card_id, reply_id = %reply_id, platform = %draft.platform, "social draft_approve: published successfully");
                    let resolved = forward::build_resolved_card(&base, "已發送");
                    try_update(resolved).await;
                    let _ = db.update_social_draft_sent(card_id, &reply_id);
                }
                Err(e) => {
                    error!(card_id, error = %e, platform = %draft.platform, "social draft_approve: send failed");
                    let failed = forward::build_failed_card(&base, "發送失敗，點擊重試");
                    try_update(failed).await;
                    let _ = db.update_social_draft_status(card_id, "failed");
                }
            }
        });
        return;
    }

    // ── Inbox button actions ──────────────────────────────────────────────────

    let row = match db.get_social_inbox(card_id) {
        Ok(Some(r)) => r,
        Ok(None) => {
            warn!(card_id, "social button action: inbox item not found");
            return;
        }
        Err(e) => {
            error!(card_id, error = %e, "social button action: db error");
            return;
        }
    };

    // Resolve admin_channel for card updates (used by multiple branches below).
    let admin_channel = {
        let cfg = config.read().unwrap();
        match row.platform.as_str() {
            "instagram" => cfg.social.instagram.as_ref().map(|c| c.admin_channel.clone()),
            "threads" => cfg.social.threads.as_ref().map(|c| c.admin_channel.clone()),
            _ => None,
        }.unwrap_or_default()
    };

    // Helper: update the forward card if we have a forward_ref and admin_channel.
    let try_update_card = |card: forward::ForwardCard| {
        let fwd_ref = row.forward_ref.clone();
        let ch = admin_channel.clone();
        let ads = adapters.clone();
        async move {
            if let (Some(msg_ref), false) = (fwd_ref, ch.is_empty()) {
                forward::update_forward_card(card, &msg_ref, &ch, &ads).await;
            }
        }
    };

    match action {
        "ignore" => {
            let _ = db.update_social_inbox_status(card_id, "ignored");
            let base = forward::build_forward_card(&row);
            let resolved = forward::build_resolved_card(&base, "已忽略");
            try_update_card(resolved).await;
        }
        "discard" | "discard_draft" => {
            let _ = db.update_social_inbox_status(card_id, "ignored");
            if let Some(ref draft) = row.draft {
                let base = forward::build_draft_card(&row, draft);
                let resolved = forward::build_resolved_card(&base, "已捨棄");
                try_update_card(resolved).await;
            }
        }
        "approve" | "approve_draft" => {
            // Send draft via Meta API (inbox-based legacy approve path).
            let draft = match &row.draft {
                Some(d) => d.clone(),
                None => {
                    warn!(card_id, "social approve: no draft");
                    return;
                }
            };
            let cfg = config.read().unwrap().clone();
            let result: crate::error::Result<String> = match row.platform.as_str() {
                "instagram" => {
                    async {
                        use crate::social::instagram::InstagramClient;
                        let ig = cfg.social.instagram.as_ref()
                            .ok_or_else(|| crate::error::CatClawError::Social("no instagram config".into()))?;
                        let token = std::env::var(&ig.token_env)
                            .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
                        let resp = InstagramClient::new(token, ig.user_id.clone())
                            .reply_comment(&row.platform_id, &draft)
                            .await?;
                        Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
                    }.await
                }
                "threads" => {
                    async {
                        use crate::social::threads::ThreadsClient;
                        let th = cfg.social.threads.as_ref()
                            .ok_or_else(|| crate::error::CatClawError::Social("no threads config".into()))?;
                        let token = std::env::var(&th.token_env)
                            .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", th.token_env)))?;
                        let resp = ThreadsClient::new(token, th.user_id.clone())
                            .reply(&row.platform_id, &draft)
                            .await?;
                        Ok(resp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string())
                    }.await
                }
                p => Err(crate::error::CatClawError::Social(format!("unknown platform '{}'", p))),
            };
            let status_label = match &result {
                Ok(_) => "已發送",
                Err(_) => "發送失敗",
            };
            if let Some(ref draft) = row.draft {
                let base = forward::build_draft_card(&row, draft);
                let resolved = forward::build_resolved_card(&base, status_label);
                try_update_card(resolved).await;
            }
            match result {
                Ok(reply_id) => { let _ = db.update_social_inbox_sent(card_id, &reply_id); }
                Err(e) => {
                    error!(card_id, error = %e, "social approve: send failed");
                    let _ = db.update_social_inbox_status(card_id, "failed");
                }
            }
        }
        "manual_reply" => {
            // Update card to "等待手動回覆" state and remove buttons — admin will reply manually.
            let base = forward::build_forward_card(&row);
            let resolved = forward::build_resolved_card(&base, "等待手動回覆");
            try_update_card(resolved).await;
            let _ = db.update_social_inbox_status(card_id, "manual");
        }
        "ai_reply" => {
            // Update card to "AI 回覆中…" processing state, then dispatch.
            let base = forward::build_forward_card(&row);
            let processing = forward::build_resolved_card(&base, "AI 回覆中…");
            try_update_card(processing).await;

            if admin_channel.is_empty() { return; }

            let agent_id = {
                let cfg = config.read().unwrap();
                match row.platform.as_str() {
                    "instagram" => cfg.social.instagram.as_ref().map(|c| c.agent.clone()),
                    "threads" => cfg.social.threads.as_ref().map(|c| c.agent.clone()),
                    _ => None,
                }.unwrap_or_else(|| "main".to_string())
            };

            // Fetch parent post text for AI context
            let parent_context = if let Some(ref mid) = row.media_id {
                fetch_parent_text(&row.platform, mid, db, config).await
                    .map(|(text, _)| text)
            } else {
                None
            };

            let platform = match row.platform.as_str() {
                "instagram" => SocialPlatform::Instagram,
                "threads" => SocialPlatform::Threads,
                _ => return,
            };
            let mut text_with_context = if let Some(h) = hint {
                let base = row.text.as_deref().unwrap_or("");
                format!("{}\n\n[Admin hint: {}]", base, h)
            } else {
                row.text.as_deref().unwrap_or("").to_string()
            };
            if let Some(ref parent) = parent_context {
                text_with_context = format!("[Original post they are replying to: {}]\n\n{}", parent, text_with_context);
            }
            let item = SocialItem {
                platform,
                platform_id: row.platform_id.clone(),
                event_type: row.event_type.clone(),
                author_id: row.author_id.clone(),
                author_name: row.author_name.clone(),
                media_id: row.media_id.clone(),
                text: Some(text_with_context),
                metadata: row.metadata.as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(serde_json::json!({})),
            };

            dispatch_action(
                ResolvedAction::AutoReply { agent: agent_id },
                item, db, config, adapters, session_manager, agent_registry, &admin_channel,
            ).await;
        }
        "view_original" => {
            let media_id = match &row.media_id {
                Some(id) => id.clone(),
                None => {
                    warn!(card_id, "view_original: no media_id");
                    return;
                }
            };
            let original = fetch_parent_text(&row.platform, &media_id, db, config).await;
            if let Some((text, _permalink)) = original {
                let mut card = forward::build_forward_card(&row);
                card.original_text = Some(text);
                try_update_card(card).await;
            } else {
                warn!(card_id, platform = %row.platform, media_id = %media_id, "view_original: failed to fetch parent text");
            }
        }
        unknown => {
            warn!(card_id, action = unknown, "social button action: unknown action");
        }
    }
}

/// Fetch parent post text, using cache if available. Returns (text, permalink).
async fn fetch_parent_text(
    platform: &str,
    media_id: &str,
    db: &Arc<StateDb>,
    config: &Arc<std::sync::RwLock<Config>>,
) -> Option<(String, Option<String>)> {
    // Check cache first
    if let Ok(Some(cached)) = db.get_parent_cache(platform, media_id) {
        return Some(cached);
    }
    // Fetch from API
    let cfg = config.read().unwrap().clone();
    let result = match platform {
        "instagram" => {
            let ig = cfg.social.instagram.as_ref()?;
            let token = std::env::var(&ig.token_env).ok()?;
            let resp = crate::social::instagram::InstagramClient::new(token, ig.user_id.clone())
                .get_media_by_id(media_id).await.ok()?;
            let text = resp.get("caption").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let permalink = resp.get("permalink").and_then(|v| v.as_str()).map(str::to_string);
            Some((text, permalink))
        }
        "threads" => {
            let th = cfg.social.threads.as_ref()?;
            let token = std::env::var(&th.token_env).ok()?;
            let resp = crate::social::threads::ThreadsClient::new(token, th.user_id.clone())
                .get_post_by_id(media_id).await.ok()?;
            let text = resp.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let permalink = resp.get("permalink").and_then(|v| v.as_str()).map(str::to_string);
            Some((text, permalink))
        }
        _ => None,
    };
    // Cache the result
    if let Some((ref text, ref permalink)) = result {
        let _ = db.upsert_parent_cache(platform, media_id, text, permalink.as_deref());
    }
    result
}
