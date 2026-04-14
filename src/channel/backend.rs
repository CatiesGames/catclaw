//! Backend channel adapter — allows external backend servers to connect via WebSocket
//! and route messages from multiple end-users to CatClaw agents.
//!
//! Protocol: single WS connection per backend, multiplexing users via tenant_id + user_id.
//! Endpoint: `/ws/backend` (separate from the TUI `/ws` endpoint).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use super::{
    ActionInfo, ChannelAdapter, ChannelCapabilities, ChannelType, MsgContext, OutboundMessage,
    TypingGuard,
};
use crate::error::{CatClawError, Result};

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

/// Inbound messages from the backend server to CatClaw.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendInbound {
    /// First message after WS connect — authenticate with shared secret.
    Auth {
        secret: String,
    },
    /// User connected: carries context (metadata + conversation history).
    /// Archives any existing session for this user and prepares context for
    /// the next message.
    SessionStart {
        tenant_id: String,
        user_id: String,
        user_name: String,
        #[serde(default)]
        user_role: Option<String>,
        #[serde(default)]
        metadata: Option<Value>,
        #[serde(default)]
        history: Option<Vec<HistoryEntry>>,
    },
    /// User sent a chat message.
    Message {
        tenant_id: String,
        user_id: String,
        text: String,
        #[serde(default)]
        metadata: Option<Value>,
    },
    /// Behavioural event (not an explicit user message).
    /// Example events: "course_hesitation", "reading_article".
    ContextEvent {
        tenant_id: String,
        user_id: String,
        user_name: String,
        event: String,
        data: Value,
        #[serde(default)]
        metadata: Option<Value>,
    },
    /// User disconnected.
    Disconnect {
        tenant_id: String,
        user_id: String,
    },
}

/// A single message in the conversation history provided by the backend.
#[derive(Debug, Deserialize)]
pub struct HistoryEntry {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// Outbound messages from CatClaw to the backend server.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendOutbound {
    Response {
        tenant_id: String,
        user_id: String,
        text: String,
    },
    Typing {
        tenant_id: String,
        user_id: String,
        active: bool,
    },
    #[allow(dead_code)]
    SessionArchived {
        tenant_id: String,
        user_id: String,
    },
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Metadata about a connected user, stored after session_start.
#[derive(Debug, Clone)]
pub(crate) struct UserMeta {
    tenant_id: String,
    user_id: String,
    user_name: String,
    /// Stored for future use (e.g. context re-injection on reconnect).
    #[allow(dead_code)]
    user_role: Option<String>,
}

/// Parse a user key ("tenant.user.uid") into (tenant_id, user_id).
fn parse_user_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("")?; // no-op, just for clarity
    let dot_user = rest.find(".user.")?;
    let tenant = &rest[..dot_user];
    let uid = &rest[dot_user + ".user.".len()..];
    if tenant.is_empty() || uid.is_empty() {
        return None;
    }
    Some((tenant, uid))
}

/// Build user key from tenant + user_id.
fn user_key(tenant_id: &str, user_id: &str) -> String {
    format!("{}.user.{}", tenant_id, user_id)
}

// ---------------------------------------------------------------------------
// BackendAdapter
// ---------------------------------------------------------------------------

/// Channel adapter for external backend servers.
///
/// Unlike Discord/Telegram/Slack adapters, the BackendAdapter does not run its
/// own event loop in `start()`. Instead, WebSocket connections are accepted by
/// the axum route `/ws/backend` in ws_server.rs, which calls
/// [`handle_backend_ws`] with a reference to this adapter.
pub struct BackendAdapter {
    /// Shared secret for authenticating backend connections.
    secret: String,
    /// Connected WS clients: connection_id → sender for writing outbound messages.
    connections: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<String>>>>,
    /// Maps user_key → (connection_id, UserMeta) for outbound routing.
    user_sessions: Arc<RwLock<HashMap<String, (String, UserMeta)>>>,
    /// Router's message channel, stored on start().
    msg_tx: Arc<RwLock<Option<mpsc::Sender<MsgContext>>>>,
    /// Pending context from session_start, consumed by the next message.
    /// user_key → formatted context string.
    pending_context: Arc<RwLock<HashMap<String, String>>>,
}

impl BackendAdapter {
    /// Create a BackendAdapter from channel config.
    /// `token_env` is treated as either an env var name or a direct secret value.
    /// If the value matches an existing env var, that env var's value is used;
    /// otherwise the value itself is used as the secret.
    pub fn from_config(config: &crate::config::ChannelConfig) -> Result<Self> {
        if config.token_env.is_empty() {
            return Err(CatClawError::Config(
                "backend adapter: token_env (shared secret) not set".into(),
            ));
        }
        // Try as env var name first, fall back to using the value directly as secret
        let secret = std::env::var(&config.token_env)
            .unwrap_or_else(|_| config.token_env.clone());
        Ok(Self {
            secret,
            connections: Arc::new(RwLock::new(HashMap::new())),
            user_sessions: Arc::new(RwLock::new(HashMap::new())),
            msg_tx: Arc::new(RwLock::new(None)),
            pending_context: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Verify the shared secret.
    pub fn verify_secret(&self, candidate: &str) -> bool {
        // Constant-time comparison would be ideal but for a single shared secret
        // behind a WS connection this is acceptable.
        self.secret == candidate
    }

    /// Register a WS connection.
    pub async fn register_connection(
        &self,
        conn_id: &str,
        tx: mpsc::UnboundedSender<String>,
    ) {
        self.connections
            .write()
            .await
            .insert(conn_id.to_string(), tx);
    }

    /// Unregister a WS connection and clean up all user sessions on that connection.
    pub async fn unregister_connection(&self, conn_id: &str) {
        self.connections.write().await.remove(conn_id);
        // Remove all user_sessions that belong to this connection
        self.user_sessions
            .write()
            .await
            .retain(|_, (cid, _)| cid != conn_id);
    }

    /// Register a user session (called on session_start).
    pub async fn register_user(
        &self,
        conn_id: &str,
        meta: UserMeta,
    ) {
        let key = user_key(&meta.tenant_id, &meta.user_id);
        self.user_sessions
            .write()
            .await
            .insert(key, (conn_id.to_string(), meta));
    }

    /// Unregister a user session (called on disconnect).
    pub async fn unregister_user(&self, tenant_id: &str, uid: &str) {
        let key = user_key(tenant_id, uid);
        self.user_sessions.write().await.remove(&key);
        self.pending_context.write().await.remove(&key);
    }

    /// Store pending context for a user (consumed by next message).
    pub async fn set_pending_context(&self, tenant_id: &str, uid: &str, context: String) {
        let key = user_key(tenant_id, uid);
        self.pending_context.write().await.insert(key, context);
    }

    /// Take (consume) pending context for a user.
    pub async fn take_pending_context(&self, tenant_id: &str, uid: &str) -> Option<String> {
        let key = user_key(tenant_id, uid);
        self.pending_context.write().await.remove(&key)
    }

    /// Look up user meta for building MsgContext.
    pub async fn get_user_meta(&self, tenant_id: &str, uid: &str) -> Option<UserMeta> {
        let key = user_key(tenant_id, uid);
        self.user_sessions
            .read()
            .await
            .get(&key)
            .map(|(_, meta)| meta.clone())
    }

    /// Send a serialized outbound message to the backend connection for a given user.
    async fn send_to_user(&self, ukey: &str, msg: &BackendOutbound) -> Result<()> {
        let sessions = self.user_sessions.read().await;
        let (conn_id, _) = sessions.get(ukey).ok_or_else(|| {
            CatClawError::Channel(format!("backend: no session for user key '{}'", ukey))
        })?;
        let connections = self.connections.read().await;
        let tx = connections.get(conn_id).ok_or_else(|| {
            CatClawError::Channel(format!("backend: connection '{}' not found", conn_id))
        })?;
        let json = serde_json::to_string(msg)
            .map_err(|e| CatClawError::Channel(format!("backend: serialize error: {}", e)))?;
        tx.send(json).map_err(|_| {
            CatClawError::Channel("backend: WS connection closed".into())
        })?;
        Ok(())
    }

    /// Get a clone of the msg_tx sender.
    pub async fn msg_tx(&self) -> Option<mpsc::Sender<MsgContext>> {
        self.msg_tx.read().await.clone()
    }
}

// ---------------------------------------------------------------------------
// ChannelAdapter trait
// ---------------------------------------------------------------------------

#[async_trait]
impl ChannelAdapter for BackendAdapter {
    async fn start(&self, msg_tx: mpsc::Sender<MsgContext>) -> Result<()> {
        // Store the msg_tx for use by the WS handler.
        // Unlike other adapters, we don't run an event loop here —
        // connections are accepted by ws_server.rs.
        *self.msg_tx.write().await = Some(msg_tx);
        info!("backend adapter ready (WS connections accepted on /ws/backend)");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        // peer_id is "{tenant_id}.user.{user_id}"
        let (tenant_id, uid) = parse_user_key(&msg.peer_id).ok_or_else(|| {
            CatClawError::Channel(format!(
                "backend: invalid peer_id format: '{}'",
                msg.peer_id
            ))
        })?;
        let outbound = BackendOutbound::Response {
            tenant_id: tenant_id.to_string(),
            user_id: uid.to_string(),
            text: msg.text,
        };
        self.send_to_user(&msg.peer_id, &outbound).await
    }

    async fn start_typing(&self, _channel_id: &str, peer_id: &str) -> Result<TypingGuard> {
        let (tenant_id, uid) = match parse_user_key(peer_id) {
            Some(pair) => pair,
            None => return Ok(TypingGuard::noop()),
        };
        // Send typing start
        let start_msg = BackendOutbound::Typing {
            tenant_id: tenant_id.to_string(),
            user_id: uid.to_string(),
            active: true,
        };
        let _ = self.send_to_user(peer_id, &start_msg).await;

        // Set up guard to send typing stop on drop
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        let adapter = Arc::new((
            self.user_sessions.clone(),
            self.connections.clone(),
        ));
        let peer_id_owned = peer_id.to_string();
        tokio::spawn(async move {
            // Wait for the guard to be dropped
            let _ = cancel_rx.try_recv();
            // We just need to wait for drop — the oneshot fires on drop
            tokio::select! {
                _ = &mut cancel_rx => {}
            }
            // Send typing stop
            if let Some((tenant, uid)) = parse_user_key(&peer_id_owned) {
                let stop_msg = BackendOutbound::Typing {
                    tenant_id: tenant.to_string(),
                    user_id: uid.to_string(),
                    active: false,
                };
                if let Ok(json) = serde_json::to_string(&stop_msg) {
                    let sessions = adapter.0.read().await;
                    if let Some((conn_id, _)) = sessions.get(&peer_id_owned) {
                        let connections = adapter.1.read().await;
                        if let Some(tx) = connections.get(conn_id) {
                            let _ = tx.send(json);
                        }
                    }
                }
            }
        });

        Ok(TypingGuard::new(cancel_tx))
    }

    async fn create_thread(&self, _channel_id: &str, _title: &str) -> Result<String> {
        Err(CatClawError::Channel(
            "backend adapter does not support threads".into(),
        ))
    }

    fn name(&self) -> &str {
        "backend"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            threading: false,
            typing_indicator: true,
            message_editing: false,
            max_message_length: 100_000,
            attachments: false,
            streaming: false,
        }
    }

    fn supported_actions(&self) -> Vec<ActionInfo> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Context formatting
// ---------------------------------------------------------------------------

/// Build a context string from session_start metadata + history.
pub fn format_session_context(
    user_name: &str,
    user_role: Option<&str>,
    metadata: Option<&Value>,
    history: Option<&[HistoryEntry]>,
) -> String {
    let mut ctx = String::new();

    // User info block
    ctx.push_str("[User Info]\n");
    ctx.push_str(&format!("Name: {}\n", user_name));
    if let Some(role) = user_role {
        ctx.push_str(&format!("Role: {}\n", role));
    }

    // Metadata (backend-defined, flat key-value)
    if let Some(Value::Object(map)) = metadata {
        for (k, v) in map {
            match v {
                Value::String(s) => ctx.push_str(&format!("{}: {}\n", k, s)),
                Value::Array(arr) => {
                    let items: Vec<String> = arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    ctx.push_str(&format!("{}: {}\n", k, items.join(", ")));
                }
                other => ctx.push_str(&format!("{}: {}\n", k, other)),
            }
        }
    }

    // Conversation history
    if let Some(entries) = history {
        if !entries.is_empty() {
            ctx.push('\n');
            ctx.push_str("[Recent Conversation]\n");
            for entry in entries {
                let role_label = match entry.role.as_str() {
                    "user" => "User",
                    "assistant" => "Assistant",
                    other => other,
                };
                if let Some(ts) = &entry.timestamp {
                    ctx.push_str(&format!("[{}] {}: {}\n", ts, role_label, entry.content));
                } else {
                    ctx.push_str(&format!("{}: {}\n", role_label, entry.content));
                }
            }
            ctx.push_str("[End of conversation history]\n");
        }
    }

    ctx
}

/// Format a context event as agent-readable text.
pub fn format_context_event(event: &str, data: &Value) -> String {
    let data_str = if let Value::Object(map) = data {
        map.iter()
            .map(|(k, v)| {
                let val = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{}: {}", k, val)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        data.to_string()
    };
    format!("[Context Event: {}]\n{}", event, data_str)
}

// ---------------------------------------------------------------------------
// WebSocket handler
// ---------------------------------------------------------------------------

/// Handle a backend WS connection.
///
/// Called by `ws_backend_handler` in ws_server.rs after the HTTP upgrade.
/// This function owns the connection lifetime: auth → read loop → cleanup.
pub async fn handle_backend_ws(
    socket: axum::extract::ws::WebSocket,
    adapter: Arc<BackendAdapter>,
    session_manager: Arc<crate::session::manager::SessionManager>,
    agent_registry: Arc<std::sync::RwLock<crate::agent::AgentRegistry>>,
) {
    use axum::extract::ws::Message;
    use futures::{SinkExt, StreamExt};

    let (mut ws_tx, mut ws_rx) = socket.split();

    // --- Auth handshake ---
    let authed = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        ws_rx.next(),
    )
    .await
    {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<BackendInbound>(&text) {
                Ok(BackendInbound::Auth { secret }) => adapter.verify_secret(&secret),
                _ => false,
            }
        }
        _ => false,
    };

    if !authed {
        let err = serde_json::to_string(&BackendOutbound::Error {
            message: "authentication failed".into(),
        })
        .unwrap_or_default();
        let _ = ws_tx.send(Message::Text(err.into())).await;
        return;
    }

    // --- Connection registered ---
    let conn_id = uuid::Uuid::new_v4().to_string();
    info!(conn_id = %conn_id, "backend: connection authenticated");

    // Outbound channel: adapter writes here, we forward to WS
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    adapter.register_connection(&conn_id, out_tx).await;

    // Spawn outbound forwarder
    let conn_id_out = conn_id.clone();
    let outbound_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                debug!(conn_id = %conn_id_out, "backend: outbound WS closed");
                break;
            }
        }
    });

    // --- Inbound read loop ---
    while let Some(frame) = ws_rx.next().await {
        let text = match frame {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => continue, // axum auto-responds with Pong
            Ok(_) => continue,
            Err(e) => {
                warn!(conn_id = %conn_id, error = %e, "backend: WS read error");
                break;
            }
        };

        let inbound = match serde_json::from_str::<BackendInbound>(&text) {
            Ok(msg) => msg,
            Err(e) => {
                warn!(conn_id = %conn_id, error = %e, "backend: invalid message");
                continue;
            }
        };

        match inbound {
            BackendInbound::Auth { .. } => {
                // Already authenticated, ignore duplicate auth
            }
            BackendInbound::SessionStart {
                tenant_id,
                user_id,
                user_name,
                user_role,
                metadata,
                history,
            } => {
                handle_session_start(
                    &adapter,
                    &session_manager,
                    &agent_registry,
                    &conn_id,
                    &tenant_id,
                    &user_id,
                    &user_name,
                    user_role.as_deref(),
                    metadata.as_ref(),
                    history.as_deref(),
                )
                .await;
            }
            BackendInbound::Message {
                tenant_id,
                user_id,
                text: msg_text,
                metadata,
            } => {
                handle_message(
                    &adapter,
                    &tenant_id,
                    &user_id,
                    &msg_text,
                    metadata.as_ref(),
                )
                .await;
            }
            BackendInbound::ContextEvent {
                tenant_id,
                user_id,
                user_name,
                event,
                data,
                metadata,
            } => {
                handle_context_event(
                    &adapter,
                    &tenant_id,
                    &user_id,
                    &user_name,
                    &event,
                    &data,
                    metadata.as_ref(),
                )
                .await;
            }
            BackendInbound::Disconnect {
                tenant_id,
                user_id,
            } => {
                debug!(
                    conn_id = %conn_id,
                    tenant_id = %tenant_id,
                    user_id = %user_id,
                    "backend: user disconnected"
                );
                adapter.unregister_user(&tenant_id, &user_id).await;
            }
        }
    }

    // --- Cleanup ---
    info!(conn_id = %conn_id, "backend: connection closed");
    adapter.unregister_connection(&conn_id).await;
    outbound_task.abort();
}

// ---------------------------------------------------------------------------
// Inbound message handlers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn handle_session_start(
    adapter: &BackendAdapter,
    session_manager: &crate::session::manager::SessionManager,
    agent_registry: &std::sync::RwLock<crate::agent::AgentRegistry>,
    conn_id: &str,
    tenant_id: &str,
    uid: &str,
    user_name: &str,
    user_role: Option<&str>,
    metadata: Option<&Value>,
    history: Option<&[HistoryEntry]>,
) {
    let ukey = user_key(tenant_id, uid);
    debug!(tenant_id, user_id = uid, "backend: session_start");

    // Register user
    adapter
        .register_user(
            conn_id,
            UserMeta {
                tenant_id: tenant_id.to_string(),
                user_id: uid.to_string(),
                user_name: user_name.to_string(),
                user_role: user_role.map(String::from),
            },
        )
        .await;

    // Resolve agent for this tenant to build session key.
    // The router will do the actual binding resolution — here we just need the
    // agent_id to construct the session key for archive lookup.
    // Use the registry's default agent; the router may resolve differently.
    let agent_id = {
        let registry = agent_registry.read().unwrap();
        match registry.default_agent_id() {
            Some(id) => id.to_string(),
            None => {
                warn!(tenant_id, "backend: no default agent configured, cannot archive old session");
                return;
            }
        }
    };

    // Archive existing session for this user (if any)
    let session_key_str = format!("catclaw:{}:backend:{}", agent_id, ukey);
    if let Ok(Some(row)) = session_manager.state_db().get_session(&session_key_str) {
        if row.state != "archived" {
            // Stop running subprocess if active
            if session_manager.stop_session(&session_key_str) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            if let Err(e) = session_manager.archive(&session_key_str).await {
                warn!(error = %e, session_key = %session_key_str, "backend: failed to archive old session");
            } else {
                debug!(session_key = %session_key_str, "backend: archived old session");
            }
        }
    }

    // Build context string from metadata + history
    let context = format_session_context(user_name, user_role, metadata, history);
    if !context.trim().is_empty() {
        adapter.set_pending_context(tenant_id, uid, context).await;
    }
}

async fn handle_message(
    adapter: &BackendAdapter,
    tenant_id: &str,
    uid: &str,
    text: &str,
    metadata: Option<&Value>,
) {
    let meta = match adapter.get_user_meta(tenant_id, uid).await {
        Some(m) => m,
        None => {
            warn!(
                tenant_id,
                user_id = uid,
                "backend: message from unknown user (no session_start)"
            );
            return;
        }
    };

    // Prepend pending context (from session_start) to the first message
    let mut full_text = String::new();
    if let Some(ctx) = adapter.take_pending_context(tenant_id, uid).await {
        full_text.push_str(&ctx);
        full_text.push('\n');
    }
    full_text.push_str(text);

    let ukey = user_key(tenant_id, uid);
    let raw_event = metadata.cloned().unwrap_or(Value::Null);

    let msg_ctx = MsgContext {
        channel_type: ChannelType::Backend,
        channel_id: tenant_id.to_string(),
        peer_id: ukey.clone(),
        sender_id: meta.user_id,
        sender_name: meta.user_name,
        text: full_text,
        attachments: vec![],
        reply_to: None,
        thread_id: None,
        is_direct_message: true,
        raw_event,
        channel_name: Some(ukey),
        guild_id: None,
        message_id: None,
    };

    if let Some(tx) = adapter.msg_tx().await {
        if let Err(e) = tx.send(msg_ctx).await {
            error!(error = %e, "backend: failed to send MsgContext to router");
        }
    }
}

async fn handle_context_event(
    adapter: &BackendAdapter,
    tenant_id: &str,
    uid: &str,
    user_name: &str,
    event: &str,
    data: &Value,
    metadata: Option<&Value>,
) {
    // Ensure user is registered (context_event may arrive without session_start
    // if the user was already connected).
    let meta = adapter.get_user_meta(tenant_id, uid).await;
    let sender_name = meta
        .as_ref()
        .map(|m| m.user_name.clone())
        .unwrap_or_else(|| user_name.to_string());

    let ukey = user_key(tenant_id, uid);
    let text = format_context_event(event, data);
    let raw_event = metadata.cloned().unwrap_or(Value::Null);

    let msg_ctx = MsgContext {
        channel_type: ChannelType::Backend,
        channel_id: tenant_id.to_string(),
        peer_id: ukey.clone(),
        sender_id: uid.to_string(),
        sender_name,
        text,
        attachments: vec![],
        reply_to: None,
        thread_id: None,
        is_direct_message: true,
        raw_event,
        channel_name: Some(ukey),
        guild_id: None,
        message_id: None,
    };

    if let Some(tx) = adapter.msg_tx().await {
        if let Err(e) = tx.send(msg_ctx).await {
            error!(error = %e, "backend: failed to send context event to router");
        }
    }
}
