//! LINE Messaging API adapter (Stage 4 MVP — text only; rich features in Stage 5).
//!
//! Webhook-driven (LINE has no polling mode). Gateway mounts the webhook handler
//! via `GatewayHandle.line_adapter` (same pattern as backend adapter).
//!
//! Outbound:
//! - Reply token (5-min validity, free) when responding to a recent inbound event.
//! - Push API fallback when no reply token (or expired).
//!
//! Inbound:
//! - HMAC-SHA256 signature verification using `secret_env`.
//! - Events parsed and forwarded to msg_tx; sender_id = LINE userId,
//!   channel_id = groupId / roomId / userId (DM).

#![allow(dead_code)]

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, OnceCell, RwLock};
use tracing::{debug, info, warn};

use super::{
    ActionInfo, AdapterFilter, ChannelAdapter, ChannelCapabilities, ChannelType, MsgContext,
    OutboundMessage, TypingGuard,
};
use crate::error::{CatClawError, Result};

const LINE_API: &str = "https://api.line.me/v2/bot";
const LINE_DATA_API: &str = "https://api-data.line.me/v2/bot";

pub struct LineAdapter {
    token: String,
    secret: String,
    filter: Arc<std::sync::RwLock<AdapterFilter>>,
    msg_tx: OnceCell<mpsc::Sender<MsgContext>>,
    /// reply_token cache: keyed by LINE userId, value = (reply_token, expires_at_unix).
    /// Reply tokens are free for 5 minutes after the inbound event.
    reply_tokens: RwLock<std::collections::HashMap<String, (String, i64)>>,
    /// Bot user id (set after first webhook event delivers it).
    bot_user_id: Mutex<Option<String>>,
    http: reqwest::Client,
}

impl LineAdapter {
    pub fn new(token: String, secret: String, filter: Arc<std::sync::RwLock<AdapterFilter>>) -> Self {
        LineAdapter {
            token,
            secret,
            filter,
            msg_tx: OnceCell::new(),
            reply_tokens: RwLock::new(std::collections::HashMap::new()),
            bot_user_id: Mutex::new(None),
            http: reqwest::Client::new(),
        }
    }

    pub fn from_config(
        config: &crate::config::ChannelConfig,
    ) -> Result<(Self, Arc<std::sync::RwLock<AdapterFilter>>)> {
        let token = std::env::var(&config.token_env).map_err(|_| {
            CatClawError::Config(format!("environment variable {} not set", config.token_env))
        })?;
        let secret_env = config.secret_env.as_ref().ok_or_else(|| {
            CatClawError::Config("LINE channel requires secret_env (channel signing secret)".into())
        })?;
        let secret = std::env::var(secret_env).map_err(|_| {
            CatClawError::Config(format!("environment variable {} not set", secret_env))
        })?;
        let filter = Arc::new(std::sync::RwLock::new(AdapterFilter::from_config(config)));
        Ok((LineAdapter::new(token, secret, filter.clone()), filter))
    }

    /// HMAC-SHA256 signature verification. Returns true if `x-line-signature`
    /// header matches `base64(HMAC-SHA256(channel_secret, body))`.
    pub fn verify_signature(&self, signature_header: &str, body: &[u8]) -> bool {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(body);
        let computed = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        // Constant-time compare on bytes.
        let a = computed.as_bytes();
        let b = signature_header.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Inject a parsed LINE webhook payload. Spawns parsing + dispatch.
    /// Called by the axum webhook handler after signature verification.
    pub async fn handle_webhook_payload(&self, payload: Value) {
        let Some(events) = payload.get("events").and_then(|v| v.as_array()) else {
            debug!("line webhook: no events array");
            return;
        };
        for event in events {
            if let Some(ctx) = self.parse_event(event).await {
                if let Some(tx) = self.msg_tx.get() {
                    let _ = tx.send(ctx).await;
                }
            }
        }
    }

    /// Parse a single LINE webhook event into a MsgContext (for "message" events).
    /// Other event types (follow, unfollow, postback) are logged but skipped in MVP.
    async fn parse_event(&self, event: &Value) -> Option<MsgContext> {
        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let source = event.get("source")?;
        let user_id = source.get("userId").and_then(|v| v.as_str()).unwrap_or("");

        // Cache reply token (5-min validity).
        if let Some(rt) = event.get("replyToken").and_then(|v| v.as_str()) {
            let expires = chrono::Utc::now().timestamp() + 295; // 5 min minus buffer
            self.reply_tokens
                .write()
                .await
                .insert(user_id.to_string(), (rt.to_string(), expires));
        }

        match event_type {
            "message" => {
                let message = event.get("message")?;
                let msg_type = message.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if msg_type != "text" {
                    debug!(msg_type, "line webhook: non-text message (Stage 5)");
                    return None;
                }
                let text = message.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let message_id = message
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("user");
                let is_dm = source_type == "user";
                let channel_id = match source_type {
                    "user" => user_id.to_string(),
                    "group" => source.get("groupId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    "room" => source.get("roomId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    _ => user_id.to_string(),
                };

                // Fetch sender display name (best-effort).
                let sender_name = self
                    .get_display_name(user_id, source_type, &channel_id)
                    .await
                    .unwrap_or_else(|| user_id.to_string());

                Some(MsgContext {
                    channel_type: ChannelType::Tui, // placeholder — see channel_type_for_line
                    channel_id: channel_id.clone(),
                    peer_id: user_id.to_string(),
                    sender_id: user_id.to_string(),
                    sender_name,
                    text,
                    attachments: vec![],
                    reply_to: None,
                    thread_id: None,
                    is_direct_message: is_dm,
                    raw_event: event.clone(),
                    channel_name: Some(format!("line:{}", &channel_id)),
                    guild_id: None,
                    message_id,
                })
                .map(|mut c| {
                    c.channel_type = channel_type_for_line();
                    c
                })
            }
            "follow" => {
                info!(user_id, "line follow event");
                None
            }
            "unfollow" => {
                info!(user_id, "line unfollow event");
                None
            }
            "postback" => {
                debug!(data = ?event.get("postback"), "line postback (Stage 5)");
                None
            }
            other => {
                debug!(event_type = other, "line: unhandled event type");
                None
            }
        }
    }

    async fn get_display_name(
        &self,
        user_id: &str,
        source_type: &str,
        channel_id: &str,
    ) -> Option<String> {
        let url = match source_type {
            "user" => format!("{}/profile/{}", LINE_API, user_id),
            "group" => format!("{}/group/{}/member/{}", LINE_API, channel_id, user_id),
            "room" => format!("{}/room/{}/member/{}", LINE_API, channel_id, user_id),
            _ => return None,
        };
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        json.get("displayName")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Try to take a cached reply_token for a user. Returns None if expired or absent.
    async fn take_reply_token(&self, user_id: &str) -> Option<String> {
        let mut map = self.reply_tokens.write().await;
        if let Some((token, expires)) = map.get(user_id).cloned() {
            if chrono::Utc::now().timestamp() < expires {
                map.remove(user_id);
                return Some(token);
            }
            map.remove(user_id);
        }
        None
    }

    async fn reply_text(&self, reply_token: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "replyToken": reply_token,
            "messages": [{"type": "text", "text": truncate_line_text(text)}],
        });
        let resp = self
            .http
            .post(format!("{}/message/reply", LINE_API))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line reply: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CatClawError::Channel(format!(
                "line reply failed: {} {}",
                status, body
            )));
        }
        Ok(())
    }

    async fn push_text(&self, user_id: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "to": user_id,
            "messages": [{"type": "text", "text": truncate_line_text(text)}],
        });
        let resp = self
            .http
            .post(format!("{}/message/push", LINE_API))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line push: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CatClawError::Channel(format!(
                "line push failed: {} {}",
                status, body
            )));
        }
        Ok(())
    }
}

fn truncate_line_text(text: &str) -> String {
    // LINE max text length is 5000 chars per message.
    if text.chars().count() <= 5000 {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(4990).collect();
        format!("{}…", truncated)
    }
}

/// LINE doesn't yet have its own ChannelType variant — tag with Tui as a placeholder.
/// Router uses `channel_type.as_str()` which we intercept to "line" via the adapter name.
/// (Long-term: add `ChannelType::Line` enum variant in a follow-up; for Stage 4 we
/// keep the change small and rely on adapter name lookup.)
fn channel_type_for_line() -> ChannelType {
    ChannelType::Line
}

#[async_trait]
impl ChannelAdapter for LineAdapter {
    async fn start(&self, msg_tx: mpsc::Sender<MsgContext>) -> Result<()> {
        let _ = self.msg_tx.set(msg_tx);
        info!("LINE adapter ready (webhook-driven)");
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        // peer_id = LINE userId for DM; for group/room replies use channel_id.
        // Reply-token first (only valid within ~5 min of an inbound event).
        if let Some(rt) = self.take_reply_token(&msg.peer_id).await {
            return self.reply_text(&rt, &msg.text).await;
        }
        let target = if msg.peer_id.starts_with('U') {
            msg.peer_id.clone()
        } else {
            msg.channel_id.clone()
        };
        self.push_text(&target, &msg.text).await
    }

    async fn start_typing(&self, _channel_id: &str, _peer_id: &str) -> Result<TypingGuard> {
        // LINE has Loading Animation API but it's optional — skip for MVP.
        Ok(TypingGuard::noop())
    }

    async fn create_thread(&self, _channel_id: &str, _title: &str) -> Result<String> {
        Err(CatClawError::Channel("LINE has no native threads".into()))
    }

    fn name(&self) -> &str {
        "line"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            threading: false,
            typing_indicator: false,
            message_editing: false,
            max_message_length: 5000,
            attachments: false, // Stage 5 enables image
            streaming: false,
        }
    }

    fn supported_actions(&self) -> Vec<ActionInfo> {
        // line_* tools added in Stage 5 (rich menu, profile, quota, flex).
        vec![]
    }

    fn platform_name(&self) -> &str {
        "line"
    }
}

// ── Webhook router ────────────────────────────────────────────────────────────

/// Build the LINE webhook router. Mounted by ws_server when a `line` channel
/// is configured (lookup via `GatewayHandle.line_adapter`).
pub fn build_webhook_router(
) -> axum::Router<Arc<crate::gateway::GatewayHandle>> {
    axum::Router::new().route(
        "/webhook/line",
        axum::routing::post(receive_webhook),
    )
}

async fn receive_webhook(
    headers: axum::http::HeaderMap,
    axum::extract::State(gw): axum::extract::State<Arc<crate::gateway::GatewayHandle>>,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    let Some(adapter) = gw.line_adapter.as_ref() else {
        warn!("line webhook: no adapter configured");
        return axum::http::StatusCode::NOT_FOUND;
    };
    let sig = headers
        .get("x-line-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !adapter.verify_signature(sig, &body) {
        warn!("line webhook: invalid signature");
        return axum::http::StatusCode::FORBIDDEN;
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            warn!("line webhook: invalid JSON");
            return axum::http::StatusCode::BAD_REQUEST;
        }
    };
    adapter.handle_webhook_payload(payload).await;
    axum::http::StatusCode::OK
}
