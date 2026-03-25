use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::approval::{ApprovalPendingEvent, ApprovalResultEvent, PendingApproval};
use crate::gateway::GatewayHandle;
use crate::mcp_server;
use crate::session::manager::SenderInfo;
use crate::session::{Priority, SessionKey};
use crate::state::StateDb;
use crate::ws_protocol::{WsEvent, WsRequest, WsResponse};

/// Start the combined server (WS + MCP) on the given address.
/// - `GET /ws` — WebSocket upgrade (TUI/WebUI)
/// - `POST /mcp` — MCP JSON-RPC (Claude CLI tool calls)
pub fn spawn(addr: String, gw: GatewayHandle) -> tokio::task::JoinHandle<()> {
    let gw = Arc::new(gw);

    tokio::spawn(async move {
        let webhook_router = crate::social::webhook::build_router(gw.clone());
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .merge(mcp_server::router())
            .merge(webhook_router)
            .with_state(gw);

        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, addr = %addr, "failed to bind server");
                return;
            }
        };

        info!(addr = %addr, "gateway server listening (WS + MCP)");

        if let Err(e) = axum::serve(listener, app).await {
            error!(error = %e, "gateway server error");
        }
    })
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(gw): State<Arc<GatewayHandle>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, gw))
}

async fn handle_connection(socket: WebSocket, gw: Arc<GatewayHandle>) {
    let (mut ws_sink, mut ws_read) = socket.split();

    // ── Auth: first message must be {"auth": "<token>"} ──────────────────────
    let expected_token = gw.config.read().unwrap().general.ws_token.clone();
    if !expected_token.is_empty() {
        let auth_ok = match ws_read.next().await {
            Some(Ok(Message::Text(t))) => {
                serde_json::from_str::<serde_json::Value>(&t)
                    .ok()
                    .and_then(|v| v.get("auth").and_then(|a| a.as_str()).map(String::from))
                    .as_deref() == Some(expected_token.as_str())
            }
            _ => false,
        };
        if !auth_ok {
            let _ = ws_sink.send(Message::Text(
                r#"{"error":"unauthorized"}"#.to_string().into(),
            )).await;
            let _ = ws_sink.close().await;
            warn!("WS client rejected: invalid or missing auth token");
            return;
        }
    }

    // Channel for sending messages back to this client (responses + events)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    // Subscribe to the gateway event bus (approval.pending, approval.result, etc.)
    let mut bus_rx = gw.event_bus.subscribe();
    let out_tx_bus = out_tx.clone();
    tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(text) => { if out_tx_bus.send(text).is_err() { break; } }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // Writer task: drain out_rx → ws_sink
    let write_handle = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Read loop: process incoming requests
    while let Some(msg) = ws_read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };

        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => continue,
        };

        let req: WsRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let resp = WsResponse::err(0, -32700, format!("parse error: {}", e));
                let _ = out_tx.send(serde_json::to_string(&resp).unwrap());
                continue;
            }
        };

        let resp = dispatch(&req, &gw, &out_tx).await;
        let _ = out_tx.send(serde_json::to_string(&resp).unwrap());
    }

    drop(out_tx);
    let _ = write_handle.await;
    info!("WS client disconnected");
}

async fn dispatch(
    req: &WsRequest,
    gw: &Arc<GatewayHandle>,
    event_tx: &mpsc::UnboundedSender<String>,
) -> WsResponse {
    match req.method.as_str() {
        "gateway.status" => handle_gateway_status(req, gw),
        "sessions.list" => handle_sessions_list(req, &gw.state_db),
        "sessions.delete" => handle_sessions_delete(req, &gw.state_db),
        "sessions.send" => handle_sessions_send(req, gw, event_tx).await,
        "sessions.stop" => handle_sessions_stop(req, gw),
        "sessions.transcript" => handle_sessions_transcript(req, gw),
        "sessions.set_model" => handle_sessions_set_model(req, gw),
        "agents.list" => handle_agents_list(req, gw),
        "agents.get" => handle_agents_get(req, gw),
        "agents.default" => handle_agents_default(req, gw),
        "agents.reload_tools" => handle_agents_reload_tools(req, gw),
        "tasks.list" => handle_tasks_list(req, &gw.state_db),
        "tasks.enable" => handle_tasks_enable(req, &gw.state_db),
        "tasks.disable" => handle_tasks_disable(req, &gw.state_db),
        "tasks.delete" => handle_tasks_delete(req, &gw.state_db),
        "config.get" => handle_config_get(req, gw),
        "config.set" => handle_config_set(req, gw),
        "approval.request" => handle_approval_request(req, gw).await,
        "approval.respond" => handle_approval_respond(req, gw).await,
        "approval.list" => handle_approval_list(req, gw),
        "mcp_env.list" => handle_mcp_env_list(req, gw),
        "mcp_env.get" => handle_mcp_env_get(req, gw),
        "mcp_env.set" => handle_mcp_env_set(req, gw),
        "mcp_env.remove" => handle_mcp_env_remove(req, gw),
        "mcp.tools" => handle_mcp_tools(req, gw),
        "social.inbox.list" => handle_social_inbox_list(req, gw),
        "social.inbox.get" => handle_social_inbox_get(req, gw),
        "social.inbox.approve" => handle_social_inbox_approve(req, gw).await,
        "social.inbox.discard" => handle_social_inbox_discard(req, gw),
        "social.inbox.reprocess" => handle_social_inbox_reprocess(req, gw),
        "social.poll" => handle_social_poll(req, gw),
        "social.mode" => handle_social_mode(req, gw),
        _ => WsResponse::err(req.id, -32601, format!("unknown method: {}", req.method)),
    }
}

// ── Approval handlers ──

async fn handle_approval_request(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let request_id = req.params.get("request_id")
        .and_then(|v| v.as_str()).unwrap_or("").to_string();
    let session_key = match req.params.get("session_key").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: session_key"),
    };
    let tool_name = req.params.get("tool_name")
        .and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tool_input = req.params.get("tool_input").cloned().unwrap_or(json!({}));

    let timeout_secs = {
        let config = gw.config.read().unwrap();
        // Use the agent's timeout if available; default 120, treat 0 as 120
        let agent_id = session_key.split(':').nth(1).unwrap_or("");
        let t = config.agents.iter()
            .find(|a| a.id == agent_id)
            .map(|a| a.approval.timeout_secs)
            .unwrap_or(120);
        if t == 0 { 120 } else { t }
    };

    let (response_tx, response_rx) = tokio::sync::oneshot::channel::<bool>();

    gw.pending_approvals.insert(request_id.clone(), PendingApproval {
        request_id: request_id.clone(),
        session_key: session_key.clone(),
        tool_name: tool_name.clone(),
        tool_input: tool_input.clone(),
        created_at: std::time::Instant::now(),
        response_tx,
    });

    // Broadcast approval.pending to all connected TUI clients
    let event = crate::ws_protocol::WsEvent {
        event: "approval.pending".to_string(),
        data: serde_json::to_value(ApprovalPendingEvent {
            request_id: request_id.clone(),
            session_key: session_key.clone(),
            tool_name: tool_name.clone(),
            tool_input: tool_input.clone(),
            expires_secs: timeout_secs,
        }).unwrap_or(json!({})),
    };
    let _ = gw.event_bus.send(serde_json::to_string(&event).unwrap_or_default());

    // Forward approval request to the origin channel (Discord/Telegram)
    {
        let gw_fwd = gw.clone();
        let rid = request_id.clone();
        let tname = tool_name.clone();
        let tinput = tool_input.clone();
        let skey = session_key.clone();
        tokio::spawn(async move {
            if let Ok(Some(session_row)) = gw_fwd.state_db.get_session(&skey) {
                let origin = &session_row.origin;
                if origin == "tui" || origin == "system" {
                    return; // TUI/system sessions don't need channel forwarding
                }
                if let Some(adapter) = gw_fwd.adapters.get(origin) {
                    if let (Some(channel_id), Some(sender_id)) = (
                        session_row.platform_channel_id(),
                        session_row.platform_sender_id(),
                    ) {
                        let thread_id = session_row.platform_thread_id();
                        if let Err(e) = adapter.send_approval(&channel_id, &sender_id, thread_id.as_deref(), &rid, &tname, &tinput).await {
                            warn!(error = %e, origin = %origin, "failed to forward approval to channel");
                        }
                    }
                }
            }
        });
    }

    // Wait for response in background, then broadcast result
    let gw2 = gw.clone();
    let rid = request_id.clone();
    tokio::spawn(async move {
        let approved = tokio::time::timeout(
            tokio::time::Duration::from_secs(timeout_secs),
            response_rx,
        ).await.unwrap_or(Ok(false)).unwrap_or(false);

        let result_event = crate::ws_protocol::WsEvent {
            event: "approval.result".to_string(),
            data: serde_json::to_value(ApprovalResultEvent {
                request_id: rid.clone(),
                approved,
                reason: if approved { None } else { Some("denied by user".to_string()) },
            }).unwrap_or(json!({})),
        };
        let _ = gw2.event_bus.send(serde_json::to_string(&result_event).unwrap_or_default());
    });

    WsResponse::ok(req.id, json!({"request_id": request_id}))
}

async fn handle_approval_respond(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let request_id = match req.params.get("request_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing param: request_id"),
    };
    let approved = req.params.get("approved")
        .and_then(|v| v.as_bool()).unwrap_or(false);

    match gw.pending_approvals.remove(request_id) {
        Some((_, pa)) => {
            let _ = pa.response_tx.send(approved);
            WsResponse::ok(req.id, json!({"approved": approved}))
        }
        None => WsResponse::err(req.id, -1, format!("approval request '{}' not found or already resolved", request_id)),
    }
}

fn handle_approval_list(req: &WsRequest, gw: &GatewayHandle) -> WsResponse {
    let items: Vec<serde_json::Value> = gw.pending_approvals.iter().map(|e| {
        json!({
            "request_id": e.request_id,
            "session_key": e.session_key,
            "tool_name": e.tool_name,
            "tool_input": e.tool_input,
            "age_secs": e.created_at.elapsed().as_secs(),
        })
    }).collect();
    WsResponse::ok(req.id, json!(items))
}

// ── Handlers ──

fn handle_gateway_status(req: &WsRequest, gw: &GatewayHandle) -> WsResponse {
    let agents = gw.agent_registry.read().unwrap().list().len();
    let sessions = gw.state_db.list_sessions().unwrap_or_default();
    let active = sessions.iter().filter(|s| s.state == "active").count();
    WsResponse::ok(
        req.id,
        json!({
            "agents": agents,
            "active_sessions": active,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}

fn handle_sessions_list(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    match db.list_sessions() {
        Ok(sessions) => {
            let rows: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    json!({
                        "session_key": s.session_key,
                        "session_id": s.session_id,
                        "agent_id": s.agent_id,
                        "origin": s.origin,
                        "context_id": s.context_id,
                        "state": s.state,
                        "last_activity_at": s.last_activity_at,
                        "created_at": s.created_at,
                        "model": s.model(),
                    })
                })
                .collect();
            WsResponse::ok(req.id, json!(rows))
        }
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_sessions_delete(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    let key = req.params.get("key").and_then(|v| v.as_str());
    match key {
        Some(k) => match db.delete_session(k) {
            Ok(_) => WsResponse::ok(req.id, json!({})),
            Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
        },
        None => WsResponse::err(req.id, -32602, "missing param: key"),
    }
}

fn handle_sessions_stop(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let stopped = gw.session_manager.stop_session(key);
    WsResponse::ok(req.id, json!({ "stopped": stopped }))
}

async fn handle_sessions_send(
    req: &WsRequest,
    gw: &Arc<GatewayHandle>,
    event_tx: &mpsc::UnboundedSender<String>,
) -> WsResponse {
    let params = &req.params;
    let key_str = match params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: agent_id"),
    };
    let message = match params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: message"),
    };

    let agent = match gw.agent_registry.read().unwrap().get(&agent_id).cloned() {
        Some(a) => a,
        None => return WsResponse::err(req.id, -1, format!("agent not found: {}", agent_id)),
    };

    // Parse session key: catclaw:{agent}:{origin}:{context_id}
    let session_key = match SessionKey::from_key_string(&key_str) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "invalid session key format"),
    };

    // Optional model override for new sessions (from pending session)
    let model_override = params.get("model").and_then(|v| v.as_str()).map(String::from);

    // stream param: default true for TUI/WebUI
    let stream = params
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let request_id = req.id;
    let sm = gw.session_manager.clone();
    let tx = event_tx.clone();

    if stream {
        // Streaming mode: push delta/tool_use/response events
        tokio::spawn(async move {
            let sender = SenderInfo {
                sender_id: Some("tui-user".to_string()),
                sender_name: Some("You".to_string()),
                channel_id: None,
                thread_id: None,
            };
            match sm
                .send_streaming(&session_key, &agent, &message, Priority::Direct, &sender, model_override.as_deref())
                .await
            {
                Ok(mut rx) => {
                    use crate::session::SessionEvent;
                    info!(request_id = request_id, "streaming: waiting for events from session");
                    while let Some(event) = rx.recv().await {
                        let ws_event = match event {
                            SessionEvent::TextDelta { text } => WsEvent {
                                event: "session.delta".to_string(),
                                data: json!({ "request_id": request_id, "text": text }),
                            },
                            SessionEvent::ToolUse { name, input } => WsEvent {
                                event: "session.tool_use".to_string(),
                                data: json!({ "request_id": request_id, "tool": name, "input": input }),
                            },
                            SessionEvent::Complete { text } => {
                                info!(request_id = request_id, len = text.len(), "session.send completed (streaming)");
                                let evt = WsEvent {
                                    event: "session.response".to_string(),
                                    data: json!({ "request_id": request_id, "text": text }),
                                };
                                let _ = tx.send(serde_json::to_string(&evt).unwrap());
                                break;
                            }
                            SessionEvent::Error { message } => {
                                error!(request_id = request_id, error = %message, "session.send failed (streaming)");
                                let evt = WsEvent {
                                    event: "session.error".to_string(),
                                    data: json!({ "request_id": request_id, "error": message }),
                                };
                                let _ = tx.send(serde_json::to_string(&evt).unwrap());
                                break;
                            }
                        };
                        if tx.send(serde_json::to_string(&ws_event).unwrap()).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    error!(request_id = request_id, error = %e, "session.send_streaming failed");
                    let evt = WsEvent {
                        event: "session.error".to_string(),
                        data: json!({ "request_id": request_id, "error": format!("{}", e) }),
                    };
                    let _ = tx.send(serde_json::to_string(&evt).unwrap());
                }
            }
        });
    } else {
        // Non-streaming mode: wait for complete response
        tokio::spawn(async move {
            let sender = SenderInfo {
                sender_id: Some("tui-user".to_string()),
                sender_name: Some("You".to_string()),
                channel_id: None,
                thread_id: None,
            };
            let event = match sm
                .send_and_wait(&session_key, &agent, &message, Priority::Direct, &sender, model_override.as_deref(), None)
                .await
            {
                Ok(response) => {
                    info!(request_id = request_id, len = response.len(), "session.send completed");
                    WsEvent {
                        event: "session.response".to_string(),
                        data: json!({ "request_id": request_id, "text": response }),
                    }
                }
                Err(e) => {
                    error!(request_id = request_id, error = %e, "session.send failed");
                    WsEvent {
                        event: "session.error".to_string(),
                        data: json!({ "request_id": request_id, "error": format!("{}", e) }),
                    }
                }
            };
            let _ = tx.send(serde_json::to_string(&event).unwrap());
        });
    }

    // Immediate ack
    WsResponse::ok(req.id, json!({ "request_id": request_id }))
}

fn handle_sessions_set_model(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    // model: string to set, null/absent to clear
    let model = req.params.get("model").and_then(|v| v.as_str());

    match gw.session_manager.set_session_model(key, model) {
        Ok(()) => WsResponse::ok(req.id, json!({ "model": model })),
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_sessions_transcript(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let registry = gw.agent_registry.read().unwrap();
    let db = &gw.state_db;
    let agent_id = match req.params.get("agent_id").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => return WsResponse::err(req.id, -32602, "missing param: agent_id"),
    };
    let session_id = match req.params.get("session_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing param: session_id"),
    };

    // Validate: cross-check agent_id against DB to prevent reading arbitrary agent workspaces
    let db_agent_id = db.list_sessions()
        .unwrap_or_default()
        .into_iter()
        .find(|row| row.session_id == session_id)
        .map(|row| row.agent_id);

    if let Some(ref actual_agent_id) = db_agent_id {
        if actual_agent_id != agent_id {
            return WsResponse::err(req.id, -1, "session does not belong to the specified agent");
        }
    }

    // Find transcript file
    let agent = match registry.get(agent_id) {
        Some(a) => a,
        None => return WsResponse::err(req.id, -1, format!("agent not found: {}", agent_id)),
    };

    // Find transcript file: try plain {session_id}.jsonl first, then *_{session_id}.jsonl
    let transcripts_dir = agent.workspace.join("transcripts");
    let plain = transcripts_dir.join(format!("{}.jsonl", session_id));
    let path = if plain.exists() {
        plain
    } else {
        // Search for labeled transcript: {label}_{session_id}.jsonl
        let suffix = format!("_{}.jsonl", session_id);
        match std::fs::read_dir(&transcripts_dir) {
            Ok(entries) => {
                let mut found = None;
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.ends_with(&suffix) {
                            found = Some(entry.path());
                            break;
                        }
                    }
                }
                match found {
                    Some(p) => p,
                    None => return WsResponse::ok(req.id, json!([])),
                }
            }
            Err(_) => return WsResponse::ok(req.id, json!([])),
        }
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return WsResponse::err(req.id, -1, format!("read transcript: {}", e)),
    };

    let entries: Vec<Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    WsResponse::ok(req.id, json!(entries))
}

fn handle_agents_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let registry = gw.agent_registry.read().unwrap();
    let agents: Vec<Value> = registry
        .list()
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "workspace": a.workspace.display().to_string(),
                "is_default": a.is_default,
            })
        })
        .collect();
    WsResponse::ok(req.id, json!(agents))
}

fn handle_agents_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let registry = gw.agent_registry.read().unwrap();
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match registry.get(id) {
        Some(a) => WsResponse::ok(
            req.id,
            json!({
                "id": a.id,
                "workspace": a.workspace.display().to_string(),
                "is_default": a.is_default,
            }),
        ),
        None => WsResponse::err(req.id, -1, format!("agent not found: {}", id)),
    }
}

fn handle_agents_default(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let registry = gw.agent_registry.read().unwrap();
    match registry.default_agent() {
        Some(a) => WsResponse::ok(
            req.id,
            json!({
                "id": a.id,
                "workspace": a.workspace.display().to_string(),
                "is_default": a.is_default,
            }),
        ),
        None => WsResponse::err(req.id, -1, "no default agent configured"),
    }
}

/// Hot-reload an agent's tool permissions and approval config.
/// Hot-reload an agent's tool permissions, approval config, and model from disk.
/// Called by TUI/CLI after saving agent settings.
fn handle_agents_reload_tools(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let agent_id = match req.params.get("agent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: agent_id"),
    };

    // Re-read model from config (catclaw.toml)
    let (timeout_secs, model, fallback_model) = {
        let disk_config = crate::config::Config::load(&gw.config_path).ok();
        if let Some(ref dc) = disk_config {
            let mut config = gw.config.write().unwrap();
            for disk_agent in &dc.agents {
                if let Some(mem_agent) = config.agents.iter_mut().find(|a| a.id == disk_agent.id) {
                    mem_agent.model = disk_agent.model.clone();
                    mem_agent.fallback_model = disk_agent.fallback_model.clone();
                }
            }
        }
        let config = gw.config.read().unwrap();
        let agent_cfg = config.agents.iter().find(|a| a.id == agent_id);
        match agent_cfg {
            Some(ac) => (ac.approval.timeout_secs, ac.model.clone(), ac.fallback_model.clone()),
            None => (120, None, None),
        }
    };

    // Re-read tool permissions from the agent's tools.toml
    let tools = {
        let registry = gw.agent_registry.read().unwrap();
        match registry.get(agent_id) {
            Some(agent) => {
                let content = std::fs::read_to_string(agent.workspace.join("tools.toml")).unwrap_or_default();
                if let Ok(parsed) = toml::from_str::<toml::Value>(&content) {
                    let get_list = |key: &str| -> Vec<String> {
                        parsed.get(key)
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default()
                    };
                    crate::agent::ToolPermissions {
                        allowed: get_list("allowed"),
                        denied: get_list("denied"),
                        require_approval: get_list("require_approval"),
                    }
                } else {
                    crate::agent::ToolPermissions::default()
                }
            }
            None => return WsResponse::err(req.id, -1, format!("agent not found: {}", agent_id)),
        }
    };

    // Build approval from tools.toml data + catclaw.toml timeout
    let approval = crate::config::ApprovalConfig {
        require_approval: tools.require_approval.clone(),
        blocked: tools.denied.clone(),
        timeout_secs,
    };

    // Apply to registry
    {
        let mut registry = gw.agent_registry.write().unwrap();
        registry.reload_agent_config(agent_id, approval, tools, model, fallback_model);
    }

    info!(agent_id = %agent_id, "agent config hot-reloaded");
    WsResponse::ok(req.id, json!({"agent_id": agent_id}))
}

fn handle_tasks_list(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    match db.list_scheduled_tasks() {
        Ok(tasks) => {
            let rows: Vec<Value> = tasks
                .iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "task_type": t.task_type,
                        "agent_id": t.agent_id,
                        "name": t.name,
                        "description": t.description,
                        "cron_expr": t.cron_expr,
                        "interval_mins": t.interval_mins,
                        "next_run_at": t.next_run_at,
                        "last_run_at": t.last_run_at,
                        "enabled": t.enabled,
                        "payload": t.payload,
                    })
                })
                .collect();
            WsResponse::ok(req.id, json!(rows))
        }
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_tasks_enable(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match db.enable_task(id) {
        Ok(_) => WsResponse::ok(req.id, json!({})),
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_tasks_disable(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match db.disable_task(id) {
        Ok(_) => WsResponse::ok(req.id, json!({})),
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_tasks_delete(req: &WsRequest, db: &Arc<StateDb>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match db.delete_task(id) {
        Ok(_) => WsResponse::ok(req.id, json!({})),
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_config_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let config = gw.config.read().unwrap();
    match config.config_get(key) {
        Ok(value) => WsResponse::ok(req.id, json!({ "key": key, "value": value })),
        Err(e) => WsResponse::err(req.id, -1, format!("{}", e)),
    }
}

fn handle_config_set(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let value = match req.params.get("value").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return WsResponse::err(req.id, -32602, "missing param: value"),
    };

    let (needs_restart, serialized, channels_snapshot) = {
        let mut config = gw.config.write().unwrap();
        let needs_restart = match config.apply_config_set(key, value) {
            Ok(nr) => nr,
            Err(e) => return WsResponse::err(req.id, -1, format!("{}", e)),
        };
        // Serialize while holding the lock, then release before file I/O
        let serialized = match toml::to_string_pretty(&*config) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("failed to serialize config: {}", e)),
        };
        let channels_snapshot = config.channels.clone();
        (needs_restart, serialized, channels_snapshot)
        // lock released here
    };

    // File I/O outside the lock
    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    // Hot-reload: apply immediate changes
    if !needs_restart {
        // Update adapter filters
        for (i, ch) in channels_snapshot.iter().enumerate() {
            if let Some(filter_lock) = gw.adapter_filters.get(i) {
                let mut filter = filter_lock.write().unwrap();
                *filter = crate::channel::AdapterFilter::from_config(ch);
            }
        }
        // Reload log level if changed
        if key == "logging.level" {
            if let Err(e) = crate::logging::set_log_level(value) {
                return WsResponse::err(req.id, -1, format!("log level reload failed: {}", e));
            }
        }
        // Reload timezone on all agents
        if key == "timezone" {
            let tz = if value.is_empty() { None } else { Some(value.to_string()) };
            let mut registry = gw.agent_registry.write().unwrap();
            registry.set_all_timezone(tz);
        }
    }

    WsResponse::ok(req.id, json!({
        "needs_restart": needs_restart,
        "key": key,
        "value": value,
    }))
}

// ── MCP Env handlers ──

fn mask_value(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 6 {
        "***".to_string()
    } else {
        let prefix: String = chars[..3].iter().collect();
        let suffix: String = chars[chars.len()-3..].iter().collect();
        format!("{}...{}", prefix, suffix)
    }
}

fn handle_mcp_env_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let config = gw.config.read().unwrap();
    let mut result = json!({});
    for (server, vars) in &config.mcp_env {
        let masked: serde_json::Map<String, Value> = vars
            .iter()
            .map(|(k, v)| (k.clone(), json!(mask_value(v))))
            .collect();
        result[server] = json!(masked);
    }
    WsResponse::ok(req.id, result)
}

fn handle_mcp_env_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let server = match req.params.get("server").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing param: server"),
    };
    let config = gw.config.read().unwrap();
    match config.mcp_env.get(server) {
        Some(vars) => {
            let masked: serde_json::Map<String, Value> = vars
                .iter()
                .map(|(k, v)| (k.clone(), json!(mask_value(v))))
                .collect();
            WsResponse::ok(req.id, json!(masked))
        }
        None => WsResponse::ok(req.id, json!({})),
    }
}

fn handle_mcp_env_set(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let server = match req.params.get("server").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: server"),
    };
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let value = match req.params.get("value").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: value"),
    };

    // Clone → mutate → serialize → write to disk → commit to in-memory config
    let serialized = {
        let config = gw.config.read().unwrap();
        let mut draft = config.mcp_env.clone();
        draft.entry(server.clone()).or_default().insert(key.clone(), value);
        // Build a temporary config with the new mcp_env for serialization
        let mut full = config.clone();
        full.mcp_env = draft;
        match toml::to_string_pretty(&full) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("serialize error: {}", e)),
        }
    };

    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    // File write succeeded — now commit to in-memory config
    {
        let mut config = gw.config.write().unwrap();
        // Re-parse from serialized to ensure consistency
        if let Ok(new_config) = toml::from_str::<crate::config::Config>(&serialized) {
            config.mcp_env = new_config.mcp_env;
        }
    }

    info!(server = %server, key = %key, "mcp_env set");
    WsResponse::ok(req.id, json!({"server": server, "key": key}))
}

fn handle_mcp_env_remove(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let server = match req.params.get("server").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: server"),
    };
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };

    // Clone → mutate → serialize → write to disk → commit to in-memory config
    let serialized = {
        let config = gw.config.read().unwrap();
        let mut draft = config.mcp_env.clone();
        if let Some(vars) = draft.get_mut(&server) {
            vars.remove(&key);
            if vars.is_empty() {
                draft.remove(&server);
            }
        }
        let mut full = config.clone();
        full.mcp_env = draft;
        match toml::to_string_pretty(&full) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("serialize error: {}", e)),
        }
    };

    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    // File write succeeded — commit to in-memory config
    {
        let mut config = gw.config.write().unwrap();
        if let Ok(new_config) = toml::from_str::<crate::config::Config>(&serialized) {
            config.mcp_env = new_config.mcp_env;
        }
    }

    info!(server = %server, key = %key, "mcp_env removed");
    WsResponse::ok(req.id, json!({"server": server, "key": key}))
}

// ── MCP Tools handler ──

fn handle_mcp_tools(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let tools = gw.mcp_tools.read().unwrap();
    let result: serde_json::Map<String, Value> = tools
        .iter()
        .map(|(server, tool_list)| (server.clone(), json!(tool_list)))
        .collect();
    WsResponse::ok(req.id, json!(result))
}

// ── Social Inbox handlers ─────────────────────────────────────────────────────

fn handle_social_inbox_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let platform = req.params.get("platform").and_then(|v| v.as_str());
    let status = req.params.get("status").and_then(|v| v.as_str());
    let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
    match gw.state_db.list_social_inbox(platform, status, limit) {
        Ok(rows) => WsResponse::ok(req.id, json!(rows)),
        Err(e) => WsResponse::err(req.id, -32603, format!("db error: {}", e)),
    }
}

fn handle_social_inbox_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match gw.state_db.get_social_inbox(id) {
        Ok(Some(row)) => WsResponse::ok(req.id, json!(row)),
        Ok(None) => WsResponse::err(req.id, -32602, format!("inbox item {} not found", id)),
        Err(e) => WsResponse::err(req.id, -32603, format!("db error: {}", e)),
    }
}

async fn handle_social_inbox_approve(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    let row = match gw.state_db.get_social_inbox(id) {
        Ok(Some(r)) => r,
        Ok(None) => return WsResponse::err(req.id, -32602, format!("inbox item {} not found", id)),
        Err(e) => return WsResponse::err(req.id, -32603, format!("db error: {}", e)),
    };
    let draft = match &row.draft {
        Some(d) => d.clone(),
        None => return WsResponse::err(req.id, -32602, "no draft to approve"),
    };

    // Send via Meta API.
    let cfg = gw.config.read().unwrap().clone();
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

    match result {
        Ok(reply_id) => {
            let _ = gw.state_db.update_social_inbox_sent(id, &reply_id);
            WsResponse::ok(req.id, json!({ "status": "sent", "reply_id": reply_id }))
        }
        Err(e) => {
            let _ = gw.state_db.update_social_inbox_status(id, "failed");
            WsResponse::err(req.id, -32603, format!("send failed: {}", e))
        }
    }
}

fn handle_social_inbox_discard(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    match gw.state_db.update_social_inbox_status(id, "ignored") {
        Ok(()) => WsResponse::ok(req.id, json!({ "status": "ignored" })),
        Err(e) => WsResponse::err(req.id, -32603, format!("db error: {}", e)),
    }
}

fn handle_social_inbox_reprocess(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    // Reset draft/reply/session state, then re-inject as a SocialItem so the
    // ingest pipeline re-runs the action router. We bypass the dedup guard by
    // resetting first (the DB row already exists, INSERT OR IGNORE would skip
    // it), so we send directly into the channel and let run_ingest re-dispatch.
    let row = match gw.state_db.get_social_inbox(id) {
        Ok(Some(r)) => r,
        Ok(None) => return WsResponse::err(req.id, -32602, format!("inbox item {} not found", id)),
        Err(e) => return WsResponse::err(req.id, -32603, format!("db error: {}", e)),
    };
    if let Err(e) = gw.state_db.reset_social_inbox_for_reprocess(id) {
        return WsResponse::err(req.id, -32603, format!("db error: {}", e));
    }
    let platform = match row.platform.as_str() {
        "instagram" => crate::social::SocialPlatform::Instagram,
        "threads" => crate::social::SocialPlatform::Threads,
        p => return WsResponse::err(req.id, -32602, format!("unknown platform '{}'", p)),
    };
    let item = crate::social::SocialItem {
        platform,
        platform_id: row.platform_id.clone(),
        event_type: row.event_type.clone(),
        author_id: row.author_id.clone(),
        author_name: row.author_name.clone(),
        media_id: row.media_id.clone(),
        text: row.text.clone(),
        metadata: row.metadata.as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::json!({})),
    };
    // dedup_insert will see the existing row and return inserted=false, so
    // run_ingest would skip it. We reset the row above and re-send; to bypass
    // the INSERT OR IGNORE guard we delete and re-insert via the channel.
    // Simplest correct approach: call the action router inline here instead.
    let gw2 = gw.clone();
    tokio::spawn(async move {
        let (action, admin_channel_opt) = {
            let cfg = gw2.config.read().unwrap();
            match item.platform {
                crate::social::SocialPlatform::Instagram => {
                    if let Some(ig_cfg) = &cfg.social.instagram {
                        let (rules, templates, default_agent) = crate::social::instagram_rule_set(ig_cfg);
                        let action = crate::social::resolve_action(&item, rules, templates, default_agent);
                        (action, Some(ig_cfg.admin_channel.clone()))
                    } else {
                        (crate::social::ResolvedAction::Ignore, None)
                    }
                }
                crate::social::SocialPlatform::Threads => {
                    if let Some(th_cfg) = &cfg.social.threads {
                        let (rules, templates, default_agent) = crate::social::threads_rule_set(th_cfg);
                        let action = crate::social::resolve_action(&item, rules, templates, default_agent);
                        (action, Some(th_cfg.admin_channel.clone()))
                    } else {
                        (crate::social::ResolvedAction::Ignore, None)
                    }
                }
            }
        };
        let Some(admin_channel) = admin_channel_opt else { return; };
        crate::social::dispatch_action(
            action, item, &gw2.state_db, &gw2.config, &gw2.adapters_list,
            &gw2.session_manager, &gw2.agent_registry, &admin_channel,
        ).await;
    });
    WsResponse::ok(req.id, json!({ "status": "pending" }))
}

fn handle_social_poll(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let platform = req.params.get("platform").and_then(|v| v.as_str());
    let cfg = gw.config.read().unwrap();
    let ig_mode = cfg.social.instagram.as_ref().map(|c| c.mode.clone()).unwrap_or_default();
    let th_mode = cfg.social.threads.as_ref().map(|c| c.mode.clone()).unwrap_or_default();
    let ig_cfg = cfg.social.instagram.clone();
    let th_cfg = cfg.social.threads.clone();
    drop(cfg);

    let tx = gw.social_item_tx.clone();
    let db = gw.state_db.clone();

    match platform.unwrap_or("all") {
        "instagram" | "all" if ig_mode == "polling" => {
            if let Some(ig) = ig_cfg {
                let tx2 = tx.clone();
                let db2 = db.clone();
                tokio::spawn(async move {
                    match crate::social::poller::poll_instagram(&ig, &db2).await {
                        Ok(items) => { for item in items { let _ = tx2.send(item); } }
                        Err(e) => warn!("manual poll instagram failed: {}", e),
                    }
                });
            }
        }
        _ => {}
    }
    match platform.unwrap_or("all") {
        "threads" | "all" if th_mode == "polling" => {
            if let Some(th) = th_cfg {
                tokio::spawn(async move {
                    match crate::social::poller::poll_threads(&th, &db).await {
                        Ok(items) => { for item in items { let _ = tx.send(item); } }
                        Err(e) => warn!("manual poll threads failed: {}", e),
                    }
                });
            }
        }
        _ => {}
    }
    WsResponse::ok(req.id, json!({ "status": "polling" }))
}

fn handle_social_mode(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let platform = match req.params.get("platform").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: platform"),
    };
    let mode = match req.params.get("mode").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: mode"),
    };
    if !matches!(mode.as_str(), "webhook" | "polling" | "off") {
        return WsResponse::err(req.id, -32602, "mode must be webhook, polling, or off");
    }

    let (serialized, webhook_url) = {
        let mut cfg = gw.config.write().unwrap();
        match platform.as_str() {
            "instagram" => {
                if let Some(ref mut ig) = cfg.social.instagram {
                    ig.mode = mode.clone();
                } else {
                    return WsResponse::err(req.id, -32602, "social.instagram is not configured");
                }
            }
            "threads" => {
                if let Some(ref mut th) = cfg.social.threads {
                    th.mode = mode.clone();
                } else {
                    return WsResponse::err(req.id, -32602, "social.threads is not configured");
                }
            }
            _ => return WsResponse::err(req.id, -32602, format!("unknown platform '{}'", platform)),
        }
        let webhook_url = if mode == "webhook" {
            let base = cfg.webhook_base_url();
            let path = match platform.as_str() {
                "instagram" => "/webhook/instagram",
                _ => "/webhook/threads",
            };
            Some(format!("{}{}", base, path))
        } else {
            None
        };
        let serialized = match toml::to_string_pretty(&*cfg) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("failed to serialize config: {}", e)),
        };
        (serialized, webhook_url)
        // lock released here
    };

    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    // webhook mode: immediate (handler reads config on each request).
    // polling / off: requires gateway restart to take effect.
    let requires_restart = mode != "webhook";
    let mut result = json!({ "platform": platform, "mode": mode, "requires_restart": requires_restart });
    if let Some(url) = webhook_url {
        result["webhook_url"] = json!(url);
    }
    WsResponse::ok(req.id, result)
}
