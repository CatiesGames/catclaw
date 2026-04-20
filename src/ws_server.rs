use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
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
        let line_webhook_router: Router<Arc<GatewayHandle>> = if gw.line_adapter.is_some() {
            crate::channel::line::build_webhook_router()
        } else {
            Router::new()
        };
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .route("/ws/backend", get(ws_backend_handler))
            .route("/health", get(|| async { "ok" }))
            .route("/media/{filename}", get(serve_media))
            .merge(mcp_server::router())
            .merge(webhook_router)
            .merge(line_webhook_router)
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

/// WebSocket handler for backend channel connections.
async fn ws_backend_handler(
    ws: WebSocketUpgrade,
    State(gw): State<Arc<GatewayHandle>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let adapter = match &gw.backend_adapter {
            Some(a) => a.clone(),
            None => {
                warn!("backend WS connection rejected: no backend adapter configured");
                return;
            }
        };
        crate::channel::backend::handle_backend_ws(
            socket,
            adapter,
            gw.session_manager.clone(),
            gw.agent_registry.clone(),
        )
        .await;
    })
}

/// Serve a file from `{workspace}/media_tmp/{filename}`.
/// Filename must match strict UUID+extension pattern to prevent path traversal.
///
/// **Security note**: This endpoint has no authentication. The Meta Graph API
/// must be able to fetch the URL directly to create media containers.
/// Access control relies on UUID v4 entropy (122 bits) making filenames unguessable.
/// URLs appear in MCP tool responses and session transcripts — treat as sensitive.
async fn serve_media(
    Path(filename): Path<String>,
    State(gw): State<Arc<GatewayHandle>>,
) -> impl IntoResponse {
    use axum::http::{header, StatusCode};
    use axum::response::Response;
    use axum::body::Body;

    // Validate filename: strict UUID + extension pattern (e.g. "a1b2c3d4-...-1234.png")
    // Must have exactly one dot, no ".." sequences, no path separators.
    let valid = filename.len() < 100
        && !filename.contains("..")
        && !filename.contains('/')
        && !filename.contains('\\')
        && filename.matches('.').count() == 1
        && filename.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !valid {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::empty())
            .unwrap();
    }

    let workspace = gw.config.read().unwrap().general.workspace.clone();
    let path = workspace.join("media_tmp").join(&filename);

    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let content_type = match filename.rsplit('.').next().unwrap_or("") {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "application/octet-stream",
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
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
        "env.list" => handle_env_list(req, gw),
        "env.get" => handle_env_get(req, gw),
        "env.set" => handle_env_set(req, gw),
        "env.remove" => handle_env_remove(req, gw),
        "mcp.tools" => handle_mcp_tools(req, gw),
        "social.inbox.list" => handle_social_inbox_list(req, gw),
        "social.inbox.get" => handle_social_inbox_get(req, gw),
        "social.inbox.approve" => handle_social_inbox_approve(req, gw).await,
        "social.inbox.discard" => handle_social_inbox_discard(req, gw),
        "social.inbox.reprocess" => handle_social_inbox_reprocess(req, gw),
        "social.poll" => handle_social_poll(req, gw),
        "social.mode" => handle_social_mode(req, gw),
        "social.draft.list" => handle_social_draft_list(req, gw),
        "social.draft.approve" => handle_social_draft_approve(req, gw).await,
        "social.draft.discard" => handle_social_draft_discard(req, gw).await,
        "social.draft.submit_for_approval" => handle_social_draft_submit(req, gw).await,
        "contact.list" => handle_contact_list(req, gw),
        "contact.get" => handle_contact_get(req, gw),
        "contact.update" => handle_contact_update(req, gw).await,
        "contact.delete" => handle_contact_delete(req, gw),
        "contact.bind" => handle_contact_bind(req, gw),
        "contact.unbind" => handle_contact_unbind(req, gw),
        "contact.draft.list" => handle_contact_draft_list(req, gw),
        "contact.draft.approve" => handle_contact_draft_approve(req, gw).await,
        "contact.draft.discard" => handle_contact_draft_discard(req, gw).await,
        "contact.draft.request_revision" => handle_contact_draft_revision(req, gw).await,
        "contact.ai_pause" => handle_contact_ai_pause(req, gw),
        "contact.ai_resume" => handle_contact_ai_resume(req, gw),
        "issues.list" => handle_issues_list(req, gw),
        "issues.ignore" => handle_issues_ignore(req, gw).await,
        "issues.resolve" => handle_issues_resolve(req, gw).await,
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
            let broadcast_forward_failed = |reason: String| {
                let event = crate::ws_protocol::WsEvent {
                    event: "approval.forward_failed".to_string(),
                    data: json!({
                        "request_id": rid,
                        "session_key": skey,
                        "reason": reason,
                    }),
                };
                let _ = gw_fwd.event_bus.send(serde_json::to_string(&event).unwrap_or_default());
            };

            let session_row = match gw_fwd.state_db.get_session(&skey) {
                Ok(Some(row)) => row,
                Ok(None) => {
                    warn!(session_key = %skey, "approval forward: session not found in DB");
                    broadcast_forward_failed("session not found in DB".to_string());
                    return;
                }
                Err(e) => {
                    warn!(session_key = %skey, error = %e, "approval forward: DB error");
                    broadcast_forward_failed(format!("DB error: {e}"));
                    return;
                }
            };
            let origin = &session_row.origin;
            if origin == "tui" || origin == "system" {
                return; // TUI/system sessions don't need channel forwarding
            }
            let adapter = match gw_fwd.adapters.get(origin) {
                Some(a) => a,
                None => {
                    warn!(origin = %origin, session_key = %skey, "approval forward: no adapter for origin");
                    broadcast_forward_failed(format!("no adapter for origin '{origin}'"));
                    return;
                }
            };
            match (session_row.platform_channel_id(), session_row.platform_sender_id()) {
                (Some(channel_id), Some(sender_id)) => {
                    let thread_id = session_row.platform_thread_id();
                    if let Err(e) = adapter.send_approval(&channel_id, &sender_id, thread_id.as_deref(), &rid, &tname, &tinput).await {
                        warn!(error = %e, origin = %origin, "approval forward: failed to send to channel");
                        broadcast_forward_failed(format!("failed to send to {origin}: {e}"));
                    }
                }
                (channel_id, sender_id) => {
                    warn!(
                        session_key = %skey,
                        origin = %origin,
                        has_channel_id = channel_id.is_some(),
                        has_sender_id = sender_id.is_some(),
                        "approval forward: missing platform IDs in session metadata"
                    );
                    broadcast_forward_failed(format!(
                        "missing platform IDs (channel={}, sender={})",
                        channel_id.is_some(), sender_id.is_some()
                    ));
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
                        "keep_context": t.keep_context,
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

    // Handle secret value keys: write actual token to ~/.catclaw/.env, not TOML
    let is_secret_value = matches!(key,
        "social.instagram.token_value" | "social.instagram.app_secret_value" | "social.instagram.webhook_verify_token_value" |
        "social.threads.token_value" | "social.threads.app_secret_value" | "social.threads.webhook_verify_token_value"
    );
    if is_secret_value {
        // Derive the env var name from config
        let env_var_name = {
            let config = gw.config.read().unwrap();
            let result: Option<String> = match key {
                "social.instagram.token_value" => config.social.instagram.as_ref().map(|c| c.token_env.clone()),
                "social.instagram.app_secret_value" => config.social.instagram.as_ref().and_then(|c| c.app_secret_env.clone()),
                "social.instagram.webhook_verify_token_value" => config.social.instagram.as_ref().and_then(|c| c.webhook_verify_token_env.clone()),
                "social.threads.token_value" => config.social.threads.as_ref().map(|c| c.token_env.clone()),
                "social.threads.app_secret_value" => config.social.threads.as_ref().and_then(|c| c.app_secret_env.clone()),
                "social.threads.webhook_verify_token_value" => config.social.threads.as_ref().and_then(|c| c.webhook_verify_token_env.clone()),
                _ => None,
            };
            result
        };
        let env_var_name = match env_var_name {
            Some(n) if !n.is_empty() => n,
            _ => return WsResponse::err(req.id, -1, "env var name not configured — set token_env first"),
        };
        let env_path = {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".catclaw").join(".env")
        };
        // Read-modify-write .env
        let mut lines: Vec<String> = if env_path.exists() {
            std::fs::read_to_string(&env_path).unwrap_or_default().lines().map(String::from).collect()
        } else {
            Vec::new()
        };
        let prefix = format!("{}=", env_var_name);
        if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
            lines[pos] = format!("{}={}", env_var_name, value);
        } else {
            lines.push(format!("{}={}", env_var_name, value));
        }
        if let Some(parent) = env_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&env_path, lines.join("\n") + "\n") {
            return WsResponse::err(req.id, -1, format!("failed to write .env: {}", e));
        }
        std::env::set_var(&env_var_name, value);
        info!(key = %key, env_var = %env_var_name, "social secret updated in .env");

        // Auto-exchange short-lived token → long-lived after token update
        if matches!(key, "social.instagram.token_value" | "social.threads.token_value") {
            let config = gw.config.clone();
            let state_db = gw.state_db.clone();
            tokio::spawn(async move {
                crate::scheduler::startup_token_check(&config, &state_db).await;
            });
        }

        return WsResponse::ok(req.id, json!({"needs_restart": false, "key": key, "value": "***"}));
    }

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

// ── Subprocess env handlers ──

fn handle_env_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let config = gw.config.read().unwrap();
    let masked: serde_json::Map<String, Value> = config.env
        .iter()
        .map(|(k, v)| (k.clone(), json!(mask_value(v))))
        .collect();
    WsResponse::ok(req.id, json!(masked))
}

fn handle_env_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k,
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let config = gw.config.read().unwrap();
    let value = config.env.get(key).map(|v| mask_value(v)).unwrap_or_default();
    WsResponse::ok(req.id, json!({"key": key, "value": value}))
}

fn handle_env_set(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };
    let value = match req.params.get("value").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: value"),
    };

    // 1. Write to catclaw.toml [env] section (for subprocess injection)
    let serialized = {
        let config = gw.config.read().unwrap();
        let mut full = config.clone();
        full.env.insert(key.clone(), value.clone());
        match toml::to_string_pretty(&full) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("serialize error: {}", e)),
        }
    };

    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    {
        let mut config = gw.config.write().unwrap();
        if let Ok(new_config) = toml::from_str::<crate::config::Config>(&serialized) {
            config.env = new_config.env;
        }
    }

    // 2. Write to ~/.catclaw/.env (so gateway process can read via std::env::var)
    write_dotenv(&key, &value);

    // 3. Set in current process env (immediate effect without restart)
    std::env::set_var(&key, &value);

    info!(key = %key, "env set (toml + .env + process)");
    WsResponse::ok(req.id, json!({"key": key}))
}

fn handle_env_remove(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let key = match req.params.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: key"),
    };

    let serialized = {
        let config = gw.config.read().unwrap();
        let mut full = config.clone();
        full.env.remove(&key);
        match toml::to_string_pretty(&full) {
            Ok(s) => s,
            Err(e) => return WsResponse::err(req.id, -1, format!("serialize error: {}", e)),
        }
    };

    if let Err(e) = std::fs::write(&gw.config_path, &serialized) {
        return WsResponse::err(req.id, -1, format!("failed to save config: {}", e));
    }

    {
        let mut config = gw.config.write().unwrap();
        if let Ok(new_config) = toml::from_str::<crate::config::Config>(&serialized) {
            config.env = new_config.env;
        }
    }

    // Remove from .env and process env
    remove_dotenv(&key);
    std::env::remove_var(&key);

    info!(key = %key, "env removed (toml + .env + process)");
    WsResponse::ok(req.id, json!({"key": key}))
}

/// Write a key=value pair to ~/.catclaw/.env (create or update).
fn write_dotenv(key: &str, value: &str) {
    let env_path = dotenv_path();
    let mut lines: Vec<String> = if env_path.exists() {
        std::fs::read_to_string(&env_path)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };
    let prefix = format!("{}=", key);
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = format!("{}={}", key, value);
    } else {
        lines.push(format!("{}={}", key, value));
    }
    if let Some(parent) = env_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&env_path, lines.join("\n") + "\n") {
        warn!(error = %e, "failed to write .env");
    }
}

/// Remove a key from ~/.catclaw/.env.
fn remove_dotenv(key: &str) {
    let env_path = dotenv_path();
    if !env_path.exists() {
        return;
    }
    let prefix = format!("{}=", key);
    let lines: Vec<String> = std::fs::read_to_string(&env_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with(&prefix))
        .map(String::from)
        .collect();
    if let Err(e) = std::fs::write(&env_path, lines.join("\n") + "\n") {
        warn!(error = %e, "failed to write .env");
    }
}

/// Path to ~/.catclaw/.env
fn dotenv_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".catclaw").join(".env")
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
        Ok(rows) => {
            // Attach poll cursors so TUI can display them.
            let mut cursors = serde_json::Map::new();
            for (p, f) in &[("instagram", "comments"), ("instagram", "mentions"),
                            ("threads", "replies"), ("threads", "mentions")] {
                if let Ok(Some(val)) = gw.state_db.get_social_cursor(p, f) {
                    cursors.insert(format!("{}.{}", p, f), json!(val));
                }
            }
            WsResponse::ok(req.id, json!({ "items": rows, "cursors": cursors }))
        }
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
                        Err(e) => warn!(error = %e, "manual poll instagram failed"),
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
                        Err(e) => warn!(error = %e, "manual poll threads failed"),
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

// ── Social Draft handlers ──

fn handle_social_draft_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let platform = req.params.get("platform").and_then(|v| v.as_str());
    let status = req.params.get("status").and_then(|v| v.as_str());
    let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
    match gw.state_db.list_social_drafts(platform, status, limit) {
        Ok(rows) => WsResponse::ok(req.id, serde_json::to_value(&rows).unwrap_or(json!([]))),
        Err(e) => WsResponse::err(req.id, -1, format!("db error: {}", e)),
    }
}

async fn handle_social_draft_approve(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    let draft = match gw.state_db.get_social_draft(id) {
        Ok(Some(d)) => d,
        Ok(None) => return WsResponse::err(req.id, -32602, format!("draft {} not found", id)),
        Err(e) => return WsResponse::err(req.id, -1, format!("db error: {}", e)),
    };

    // Idempotency guard: allow awaiting_approval/draft/failed (retry)
    if draft.status != "awaiting_approval" && draft.status != "draft" && draft.status != "failed" {
        return WsResponse::err(
            req.id, -32602,
            format!("draft {} cannot be approved (status={})", id, draft.status),
        );
    }

    let cfg = gw.config.read().unwrap().clone();
    let admin_channel = match draft.platform.as_str() {
        "instagram" => cfg.social.instagram.as_ref().map(|c| c.admin_channel.clone()),
        "threads" => cfg.social.threads.as_ref().map(|c| c.admin_channel.clone()),
        _ => None,
    }.unwrap_or_default();

    // Show "publishing..." state immediately
    if let Some(ref fwd_ref) = draft.forward_ref {
        if !admin_channel.is_empty() {
            let base = crate::social::forward::build_social_draft_card(&draft);
            let publishing = crate::social::forward::build_publishing_card(&base);
            if let Err(e) = crate::social::forward::update_forward_card(
                publishing, fwd_ref, &admin_channel, &gw.adapters_list,
            ).await {
                warn!(id, msg_ref = %fwd_ref, error = %e,
                    "ws social.draft.approve: publishing card update failed");
            }
        }
    }

    match crate::social::execute_draft_publish(&draft, &cfg).await {
        Ok(reply_id) => {
            info!(id, reply_id = %reply_id, platform = %draft.platform, "social.draft.approve: published successfully");
            let _ = gw.state_db.update_social_draft_sent(id, &reply_id);
            // Keep media_tmp file so the approval card image stays visible
            if let Some(ref fwd_ref) = draft.forward_ref {
                if !admin_channel.is_empty() {
                    let base = crate::social::forward::build_social_draft_card(&draft);
                    let resolved = crate::social::forward::build_resolved_card(&base, "已發送");
                    if let Err(e) = crate::social::forward::update_forward_card(
                        resolved, fwd_ref, &admin_channel, &gw.adapters_list,
                    ).await {
                        warn!(id, msg_ref = %fwd_ref, error = %e,
                            "ws social.draft.approve: resolved card update failed");
                    }
                }
            }
            WsResponse::ok(req.id, json!({ "status": "sent", "reply_id": reply_id }))
        }
        Err(e) => {
            let _ = gw.state_db.update_social_draft_status(id, "failed");
            // Update card to failed state (with retry button)
            if let Some(ref fwd_ref) = draft.forward_ref {
                if !admin_channel.is_empty() {
                    let base = crate::social::forward::build_social_draft_card(&draft);
                    let failed = crate::social::forward::build_failed_card(&base, "發送失敗，點擊重試");
                    if let Err(uerr) = crate::social::forward::update_forward_card(
                        failed, fwd_ref, &admin_channel, &gw.adapters_list,
                    ).await {
                        warn!(id, msg_ref = %fwd_ref, error = %uerr,
                            "ws social.draft.approve: failed-card update failed");
                    }
                }
            }
            WsResponse::err(req.id, -1, format!("publish failed: {}", e))
        }
    }
}

async fn handle_social_draft_discard(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => return WsResponse::err(req.id, -32602, "missing param: id"),
    };
    let draft = match gw.state_db.get_social_draft(id) {
        Ok(Some(d)) => d,
        Ok(None) => return WsResponse::err(req.id, -32602, format!("draft {} not found", id)),
        Err(e) => return WsResponse::err(req.id, -1, format!("db error: {}", e)),
    };

    // Only unsent drafts can be discarded
    if draft.status == "sent" {
        return WsResponse::err(req.id, -32602, format!("draft {} already sent, cannot discard", id));
    }

    // Clean up media file if present
    let workspace = gw.config.read().unwrap().general.workspace.clone();
    crate::social::cleanup_draft_media(&workspace, &draft.media_urls);

    // Update forward card in admin channel (remove buttons, show "已捨棄")
    let cfg = gw.config.read().unwrap().clone();
    let admin_channel = match draft.platform.as_str() {
        "instagram" => cfg.social.instagram.as_ref().map(|c| c.admin_channel.clone()),
        "threads" => cfg.social.threads.as_ref().map(|c| c.admin_channel.clone()),
        _ => None,
    }.unwrap_or_default();
    if let Some(ref fwd_ref) = draft.forward_ref {
        if !admin_channel.is_empty() {
            let base = crate::social::forward::build_social_draft_card(&draft);
            let resolved = crate::social::forward::build_resolved_card(&base, "已捨棄");
            if let Err(e) = crate::social::forward::update_forward_card(
                resolved, fwd_ref, &admin_channel, &gw.adapters_list,
            ).await {
                warn!(id, msg_ref = %fwd_ref, error = %e,
                    "ws social.draft.discard: card update failed");
            }
        }
    }

    // Delete from DB
    match gw.state_db.delete_social_draft(id) {
        Ok(_) => {
            info!(id, platform = %draft.platform, "social.draft.discard: deleted");
            WsResponse::ok(req.id, json!({ "status": "deleted" }))
        }
        Err(e) => WsResponse::err(req.id, -1, format!("db error: {}", e)),
    }
}

/// Fetch original comment context from Instagram API. Returns (username, text).
async fn fetch_ig_comment_context(comment_id: &str, cfg: &crate::config::Config) -> Option<(String, String)> {
    let ig = cfg.social.instagram.as_ref()?;
    let token = std::env::var(&ig.token_env).ok()?;
    let client = crate::social::instagram::InstagramClient::new(token, ig.user_id.clone());
    let val = client.get_comment_by_id(comment_id).await.ok()?;
    let username = val.get("username").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Some((username, text))
}

/// Fetch original post context from Threads API. Returns (username, text).
async fn fetch_th_post_context(post_id: &str, cfg: &crate::config::Config) -> Option<(String, String)> {
    let th = cfg.social.threads.as_ref()?;
    let token = std::env::var(&th.token_env).ok()?;
    let client = crate::social::threads::ThreadsClient::new(token, th.user_id.clone());
    let val = client.get_post_by_id(post_id).await.ok()?;
    let username = val.get("username").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Some((username, text))
}

/// Parse a JSON value that may be a real array or a stringified JSON array.
/// Agents sometimes pass `"[\"url1\",\"url2\"]"` instead of `["url1","url2"]`.
fn parse_string_or_array(v: &serde_json::Value) -> Vec<String> {
    if let Some(arr) = v.as_array() {
        return arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    }
    if let Some(s) = v.as_str() {
        if s.starts_with('[') {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(s) {
                return parsed;
            }
        }
        // Single URL string — wrap in vec.
        if !s.is_empty() {
            return vec![s.to_string()];
        }
    }
    vec![]
}

/// Create a draft from tool_input args. Called by the hook path — the MCP handler
/// hasn't executed yet, so we must build the draft here from the raw arguments.
fn stage_draft_from_tool(
    tool_name: &str,
    tool_input: &serde_json::Value,
    db: &crate::state::StateDb,
) -> std::result::Result<crate::state::SocialDraftRow, String> {
    use crate::state::SocialDraftRow;

    let (platform, draft_type, content_key, reply_to_key, media_key) = match tool_name {
        "mcp__catclaw__instagram_reply_comment" => ("instagram", "reply", "message", Some("comment_id"), None),
        "mcp__catclaw__instagram_create_post"   => ("instagram", "post",  "caption", None,               Some("image_urls")),
        "mcp__catclaw__instagram_send_dm"       => ("instagram", "dm",    "text",    Some("recipient_id"), None),
        "mcp__catclaw__threads_reply"            => ("threads",   "reply", "text",    Some("reply_to_id"),  None),
        "mcp__catclaw__threads_create_post"      => ("threads",   "post",  "text",    None,               Some("media_urls")),
        other => return Err(format!("unrecognized social tool '{}'", other)),
    };

    let content = tool_input.get(content_key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing '{}' in tool_input", content_key))?;

    let reply_to_id = reply_to_key
        .and_then(|k| tool_input.get(k))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let media_urls: Vec<String> = media_key
        .and_then(|k| tool_input.get(k))
        .map(parse_string_or_array)
        .unwrap_or_default();

    // Reuse any non-terminal draft for the same target so retries don't leave
    // zombie rows behind. Refresh the content + media with the agent's new output,
    // and reset status to 'draft' so the submit handler can transition it cleanly.
    if let Ok(Some(mut existing)) = db.find_latest_draft_for_tool(platform, draft_type, reply_to_id.as_deref()) {
        let _ = db.update_social_draft_content(existing.id, content, &media_urls);
        let _ = db.update_social_draft_status(existing.id, "draft");
        existing.content = content.to_string();
        existing.media_urls = media_urls;
        existing.status = "draft".to_string();
        return Ok(existing);
    }

    let mut row = SocialDraftRow::new(platform, draft_type, content);
    row.reply_to_id = reply_to_id.clone();
    row.media_urls = media_urls;

    // For reply/dm drafts, try to fill original_author and original_text from inbox.
    if draft_type == "reply" || draft_type == "dm" {
        if let Some(ref rid) = reply_to_id {
            if let Ok(Some(inbox_row)) = db.get_social_inbox_by_platform_id(platform, rid) {
                row.original_author = inbox_row.author_name.clone();
                row.original_text = inbox_row.text.clone();
            }
        }
    }

    let id = db.insert_social_draft(&row).map_err(|e| format!("db insert error: {}", e))?;
    db.get_social_draft(id)
        .map_err(|e| format!("db read error: {}", e))?
        .ok_or_else(|| "failed to read auto-staged draft".to_string())
}

/// Called by the cmd_hook when a social publish tool hits require_approval.
/// Creates the draft from tool_input (since the MCP handler hasn't run yet),
/// sends approval card, stores forward_ref.
async fn handle_social_draft_submit(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let tool_name = match req.params.get("tool_name").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing param: tool_name"),
    };
    let tool_input = req.params.get("tool_input").cloned().unwrap_or(json!({}));

    // Build draft from tool_input. The hook fires BEFORE the MCP handler, so the
    // draft does not exist yet — we must create it here.
    let mut draft = match stage_draft_from_tool(&tool_name, &tool_input, &gw.state_db) {
        Ok(d) => d,
        Err(e) => return WsResponse::err(req.id, -32602, e),
    };

    // For reply drafts missing original context, fetch from API.
    if draft.original_author.is_none() && draft.draft_type == "reply" {
        if let Some(ref rid) = draft.reply_to_id {
            let cfg = gw.config.read().unwrap().clone();
            let fetched = match draft.platform.as_str() {
                "instagram" => fetch_ig_comment_context(rid, &cfg).await,
                "threads" => fetch_th_post_context(rid, &cfg).await,
                _ => None,
            };
            if let Some((author, text)) = fetched {
                draft.original_author = Some(author.clone());
                draft.original_text = Some(text.clone());
                let _ = gw.state_db.update_social_draft_original(draft.id, &author, &text);
            }
        }
    }

    // Set status to awaiting_approval.
    let _ = gw.state_db.update_social_draft_status(draft.id, "awaiting_approval");

    // Determine admin_channel.
    let platform = draft.platform.as_str();
    let admin_channel = {
        let cfg = gw.config.read().unwrap();
        match platform {
            "instagram" => cfg.social.instagram.as_ref().map(|c| c.admin_channel.clone()),
            "threads" => cfg.social.threads.as_ref().map(|c| c.admin_channel.clone()),
            _ => None,
        }.unwrap_or_default()
    };

    if admin_channel.is_empty() {
        let _ = gw.state_db.update_social_draft_status(draft.id, "failed");
        return WsResponse::err(
            req.id, -1,
            format!(
                "admin_channel not configured for platform '{}' — draft {} set to failed. \
                 Configure social.{}.admin_channel in catclaw.toml.",
                platform, draft.id, platform,
            ),
        );
    }

    // For reply drafts that originated from an inbox item, edit the inbox forward
    // card in place instead of sending a brand new card. The whole lifecycle of
    // one incoming reply (forward → AI processing → draft review → sent) then
    // lives on a single message in the admin channel.
    let mut reused = false;
    if draft.draft_type == "reply" {
        if let Some(ref rid) = draft.reply_to_id {
            if let Ok(Some(inbox)) = gw.state_db.get_social_inbox_by_platform_id(&draft.platform, rid) {
                if let Some(fwd_ref) = inbox.forward_ref.clone() {
                    let card = crate::social::forward::build_social_draft_card(&draft);
                    match crate::social::forward::update_forward_card(
                        card, &fwd_ref, &admin_channel, &gw.adapters_list,
                    ).await {
                        Ok(_) => {
                            let _ = gw.state_db.update_social_draft_forward_ref(draft.id, &fwd_ref);
                            reused = true;
                        }
                        Err(e) => {
                            warn!(draft_id = draft.id, msg_ref = %fwd_ref, error = %e,
                                "submit_for_approval: failed to reuse inbox card, falling back to new card");
                        }
                    }
                }
            }
        }
    }

    if !reused {
        // No inbox source (DM, proactive post, …) — send a new card.
        let card = crate::social::forward::build_social_draft_card(&draft);
        match crate::social::forward::send_forward_card(card, &admin_channel, &gw.adapters_list).await {
            Ok(Some(msg_id)) => {
                let _ = gw.state_db.update_social_draft_forward_ref(draft.id, &msg_id);
            }
            Ok(None) => {
                warn!("social.draft.submit_for_approval: no adapter sent the card (admin_channel={})", admin_channel);
                let _ = gw.state_db.update_social_draft_status(draft.id, "failed");
                return WsResponse::err(
                    req.id, -1,
                    format!(
                        "no adapter found for admin_channel '{}' — draft {} set to failed",
                        admin_channel, draft.id,
                    ),
                );
            }
            Err(e) => {
                warn!(error = %e, "social.draft.submit_for_approval: failed to send card");
                let _ = gw.state_db.update_social_draft_status(draft.id, "failed");
                return WsResponse::err(
                    req.id, -1,
                    format!("failed to send approval card: {} — draft {} set to failed", e, draft.id),
                );
            }
        }
    }

    WsResponse::ok(req.id, json!({ "draft_id": draft.id, "status": "awaiting_approval" }))
}

// ── Issues handlers ──

fn handle_issues_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let agents: Vec<crate::agent::Agent> = gw.agent_registry.read().unwrap().list().into_iter().cloned().collect();
    let mut all_issues: Vec<serde_json::Value> = Vec::new();
    for agent in &agents {
        let issues_path = agent.workspace.join("memory").join("issues.json");
        let issues: Vec<crate::scheduler::LogIssue> = std::fs::read_to_string(&issues_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for issue in issues {
            let mut v = serde_json::to_value(&issue).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("agent_id".to_string(), json!(agent.id));
            }
            all_issues.push(v);
        }
    }
    WsResponse::ok(req.id, json!({ "issues": all_issues }))
}

async fn handle_issues_ignore(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let agent_id = match req.params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing agent_id"),
    };
    let issue_id = match req.params.get("issue_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing issue_id"),
    };
    let workspace = {
        let registry = gw.agent_registry.read().unwrap();
        registry.get(&agent_id).map(|a| a.workspace.clone())
    };
    let Some(workspace) = workspace else {
        return WsResponse::err(req.id, -32602, format!("agent '{}' not found", agent_id));
    };
    let issues_path = workspace.join("memory").join("issues.json");
    let mut issues: Vec<crate::scheduler::LogIssue> = tokio::fs::read_to_string(&issues_path)
        .await.ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let found = issues.iter_mut().find(|i| i.id == issue_id);
    match found {
        Some(issue) => { issue.status = "ignored".to_string(); }
        None => return WsResponse::err(req.id, -32602, format!("issue '{}' not found", issue_id)),
    }
    if let Ok(s) = serde_json::to_string_pretty(&issues) {
        if let Err(e) = tokio::fs::write(&issues_path, s).await {
            return WsResponse::err(req.id, -1, format!("failed to write issues.json: {}", e));
        }
    }
    WsResponse::ok(req.id, json!({ "ok": true }))
}

async fn handle_issues_resolve(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let agent_id = match req.params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing agent_id"),
    };
    let issue_id = match req.params.get("issue_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return WsResponse::err(req.id, -32602, "missing issue_id"),
    };
    let workspace = {
        let registry = gw.agent_registry.read().unwrap();
        registry.get(&agent_id).map(|a| a.workspace.clone())
    };
    let Some(workspace) = workspace else {
        return WsResponse::err(req.id, -32602, format!("agent '{}' not found", agent_id));
    };
    let issues_path = workspace.join("memory").join("issues.json");
    let mut issues: Vec<crate::scheduler::LogIssue> = tokio::fs::read_to_string(&issues_path)
        .await.ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let before = issues.len();
    issues.retain(|i| i.id != issue_id);
    if issues.len() == before {
        return WsResponse::err(req.id, -32602, format!("issue '{}' not found", issue_id));
    }
    if let Ok(s) = serde_json::to_string_pretty(&issues) {
        if let Err(e) = tokio::fs::write(&issues_path, s).await {
            return WsResponse::err(req.id, -1, format!("failed to write issues.json: {}", e));
        }
    }
    WsResponse::ok(req.id, json!({ "ok": true }))
}

// ── Contacts handlers ──────────────────────────────────────────────────────────

fn handle_contact_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let filter = crate::contacts::ContactsFilter {
        agent_id: req.params.get("agent_id").and_then(|v| v.as_str()).map(String::from),
        role: req.params.get("role").and_then(|v| v.as_str()).map(crate::contacts::ContactRole::parse),
        tag: req.params.get("tag").and_then(|v| v.as_str()).map(String::from),
    };
    match gw.state_db.list_contacts(&filter) {
        Ok(rows) => WsResponse::ok(req.id, json!(rows)),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_get(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    match gw.state_db.get_contact(id) {
        Ok(Some(c)) => {
            let channels = gw.state_db.list_contact_channels(&c.id).unwrap_or_default();
            WsResponse::ok(req.id, json!({"contact": c, "channels": channels}))
        }
        Ok(None) => WsResponse::err(req.id, -1, "contact not found"),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

async fn handle_contact_update(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let default_agent = gw.config.read().unwrap().default_agent_id().unwrap_or("main").to_string();
    let unknown_inbox = gw.config.read().unwrap().contacts.unknown_inbox_channel.clone();
    match crate::contacts::tools::execute_contacts_tool(
        &gw.state_db, &gw.state_db, &gw.adapters,
        &gw.session_manager, &gw.agent_registry,
        &default_agent, unknown_inbox.as_deref(),
        "contacts_update", req.params.clone(),
    ).await {
        Ok(v) => WsResponse::ok(req.id, v),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_delete(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    match gw.state_db.delete_contact(id) {
        Ok(_) => WsResponse::ok(req.id, json!({"deleted": id})),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_bind(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = req.params.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let platform = req.params.get("platform").and_then(|v| v.as_str()).unwrap_or("");
    let pu = req.params.get("platform_user_id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() || platform.is_empty() || pu.is_empty() {
        return WsResponse::err(req.id, -32602, "missing id/platform/platform_user_id");
    }
    let mut ch = crate::contacts::ContactChannel::new(id, platform, pu);
    ch.is_primary = req.params.get("is_primary").and_then(|v| v.as_bool()).unwrap_or(false);
    match gw.state_db.upsert_contact_channel(&ch) {
        Ok(_) => WsResponse::ok(req.id, json!(ch)),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_unbind(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let platform = req.params.get("platform").and_then(|v| v.as_str()).unwrap_or("");
    let pu = req.params.get("platform_user_id").and_then(|v| v.as_str()).unwrap_or("");
    if platform.is_empty() || pu.is_empty() {
        return WsResponse::err(req.id, -32602, "missing platform/platform_user_id");
    }
    match gw.state_db.delete_contact_channel(platform, pu) {
        Ok(_) => WsResponse::ok(req.id, json!({"unbound": true})),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_draft_list(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let contact_id = req.params.get("contact_id").and_then(|v| v.as_str());
    let status = req.params.get("status").and_then(|v| v.as_str());
    let limit = req.params.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
    match gw.state_db.list_contact_drafts(contact_id, status, limit) {
        Ok(rows) => WsResponse::ok(req.id, json!(rows)),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

async fn handle_contact_draft_approve(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(v) => v,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    let unknown_inbox = gw.config.read().unwrap().contacts.unknown_inbox_channel.clone();
    match crate::contacts::pipeline::approve_draft(&gw.state_db, &gw.adapters, id, unknown_inbox.as_deref()).await {
        Ok(res) => WsResponse::ok(req.id, json!(res)),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

async fn handle_contact_draft_discard(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(v) => v,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    let unknown_inbox = gw.config.read().unwrap().contacts.unknown_inbox_channel.clone();
    match crate::contacts::pipeline::discard_draft(&gw.state_db, &gw.adapters, id, unknown_inbox.as_deref()).await {
        Ok(res) => WsResponse::ok(req.id, json!(res)),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

async fn handle_contact_draft_revision(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_i64()) {
        Some(v) => v,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    let note = req.params.get("note").and_then(|v| v.as_str()).unwrap_or("");
    let unknown_inbox = gw.config.read().unwrap().contacts.unknown_inbox_channel.clone();
    match crate::contacts::pipeline::request_revision(&gw.state_db, &gw.adapters, id, note, unknown_inbox.as_deref()).await {
        Ok(res) => {
            // Fire-and-forget: push the revision instruction back to the agent's session.
            crate::contacts::pipeline::dispatch_revision_to_agent(
                &gw.state_db,
                &gw.session_manager,
                &gw.agent_registry,
                id,
            )
            .await;
            WsResponse::ok(req.id, json!(res))
        }
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_ai_pause(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    let mut c = match gw.state_db.get_contact(id) {
        Ok(Some(c)) => c,
        _ => return WsResponse::err(req.id, -1, "contact not found"),
    };
    c.ai_paused = true;
    match gw.state_db.update_contact(&c) {
        Ok(_) => WsResponse::ok(req.id, json!({"paused": id})),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}

fn handle_contact_ai_resume(req: &WsRequest, gw: &Arc<GatewayHandle>) -> WsResponse {
    let id = match req.params.get("id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return WsResponse::err(req.id, -32602, "missing id"),
    };
    let mut c = match gw.state_db.get_contact(id) {
        Ok(Some(c)) => c,
        _ => return WsResponse::err(req.id, -1, "contact not found"),
    };
    c.ai_paused = false;
    match gw.state_db.update_contact(&c) {
        Ok(_) => WsResponse::ok(req.id, json!({"resumed": id})),
        Err(e) => WsResponse::err(req.id, -1, format!("{e}")),
    }
}
