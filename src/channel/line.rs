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
    ActionInfo, AdapterFilter, Attachment, ChannelAdapter, ChannelCapabilities, ChannelType,
    MsgContext, OutboundMessage, TypingGuard,
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
        // Reject empty secret: HMAC accepts any key length, so an empty secret
        // would let anyone who knows the secret is empty forge a valid signature.
        if secret.is_empty() {
            return Err(CatClawError::Config(format!(
                "environment variable {} is set but empty — LINE channel secret must not be blank",
                secret_env
            )));
        }
        if token.is_empty() {
            return Err(CatClawError::Config(format!(
                "environment variable {} is set but empty — LINE access token must not be blank",
                config.token_env
            )));
        }
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
    ///
    /// When `contacts.enabled=true`, every inbound LINE sender is auto-registered
    /// as a `role=unknown` contact (bound to LINE userId) BEFORE the message is
    /// dispatched to the router. The router then sees role=unknown and skips
    /// agent dispatch (per design — unknown contacts are storage-only until the
    /// admin promotes them to client/admin via `contacts_update`).
    pub async fn handle_webhook_payload(
        &self,
        payload: Value,
        db: &crate::state::StateDb,
        default_agent_id: &str,
        contacts_enabled: bool,
    ) {
        let Some(events) = payload.get("events").and_then(|v| v.as_array()) else {
            debug!("line webhook: no events array");
            return;
        };
        for event in events {
            // Auto-register unknown contact (no LLM): cheap DB operation that
            // ensures we have a record of every LINE sender, even if they're
            // never promoted to a client. Fail-soft — log and continue on error.
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if contacts_enabled {
                if let Some(source) = event.get("source") {
                    let user_id = source
                        .get("userId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !user_id.is_empty() {
                        self.ensure_unknown_contact(db, default_agent_id, user_id, source).await;
                        // Unfollow: pause AI for the contact (if any) and tag
                        // it. The contact row is preserved so historical data
                        // isn't lost.
                        if event_type == "unfollow" {
                            if let Ok(Some(mut c)) = db.get_contact_by_platform_user("line", user_id) {
                                c.ai_paused = true;
                                if !c.tags.iter().any(|t| t == "unfollowed") {
                                    c.tags.push("unfollowed".to_string());
                                }
                                let _ = db.update_contact(&c);
                            }
                        }
                    }
                }
            }

            if let Some(ctx) = self.parse_event(event).await {
                if let Some(tx) = self.msg_tx.get() {
                    let _ = tx.send(ctx).await;
                }
            }
        }
    }

    /// If a LINE userId has no contact binding yet, create a `role=unknown`
    /// contact and bind the LINE userId to it. Idempotent — does nothing when
    /// already bound. No LLM call; pure DB writes.
    async fn ensure_unknown_contact(
        &self,
        db: &crate::state::StateDb,
        default_agent_id: &str,
        user_id: &str,
        source: &Value,
    ) {
        match db.get_contact_by_platform_user("line", user_id) {
            Ok(Some(_)) => return, // already bound
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, user_id, "ensure_unknown_contact: lookup failed");
                return;
            }
        }
        // Try to fetch displayName for nicer naming; fall back to userId prefix.
        let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("user");
        let channel_id = match source_type {
            "user" => user_id.to_string(),
            "group" => source.get("groupId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "room" => source.get("roomId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            _ => user_id.to_string(),
        };
        let display_name = self
            .get_display_name(user_id, source_type, &channel_id)
            .await
            .unwrap_or_else(|| format!("LINE:{}", &user_id[..user_id.len().min(8)]));

        let contact = crate::contacts::Contact::new(default_agent_id, display_name);
        let contact_id = contact.id.clone();
        if let Err(e) = db.insert_contact(&contact) {
            warn!(error = %e, user_id, "ensure_unknown_contact: insert failed");
            return;
        }
        let mut ch = crate::contacts::ContactChannel::new(&contact_id, "line", user_id);
        ch.is_primary = true;
        if let Err(e) = db.upsert_contact_channel(&ch) {
            warn!(error = %e, user_id, contact_id = %contact_id, "ensure_unknown_contact: bind failed");
            return;
        }
        info!(
            user_id,
            contact_id = %contact_id,
            "auto-registered unknown LINE contact"
        );
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

        let source_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("user");
        let is_dm = source_type == "user";
        let channel_id = match source_type {
            "user" => user_id.to_string(),
            "group" => source.get("groupId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "room" => source.get("roomId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            _ => user_id.to_string(),
        };

        let sender_name = self
            .get_display_name(user_id, source_type, &channel_id)
            .await
            .unwrap_or_else(|| user_id.to_string());

        let mk_ctx = |text: String, attachments: Vec<Attachment>, message_id: Option<String>| MsgContext {
            channel_type: channel_type_for_line(),
            channel_id: channel_id.clone(),
            peer_id: user_id.to_string(),
            sender_id: user_id.to_string(),
            sender_name: sender_name.clone(),
            text,
            attachments,
            reply_to: None,
            thread_id: None,
            is_direct_message: is_dm,
            raw_event: event.clone(),
            channel_name: Some(format!("line:{}", &channel_id)),
            guild_id: None,
            message_id,
        };

        match event_type {
            "message" => {
                let message = event.get("message")?;
                let msg_type = message.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let message_id = message.get("id").and_then(|v| v.as_str()).map(String::from);
                match msg_type {
                    "text" => {
                        let text = message
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(mk_ctx(text, vec![], message_id))
                    }
                    "image" | "video" | "audio" | "file" => {
                        let mid = message_id.clone().unwrap_or_default();
                        let url = format!("{}/message/{}/content", LINE_DATA_API, mid);
                        let filename = match msg_type {
                            "image" => format!("{}.jpg", mid),
                            "video" => format!("{}.mp4", mid),
                            "audio" => format!("{}.m4a", mid),
                            "file" => message
                                .get("fileName")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                                .unwrap_or_else(|| mid.clone()),
                            _ => mid.clone(),
                        };
                        let size = message.get("fileSize").and_then(|v| v.as_u64());
                        let auth = format!("Bearer {}", self.token);
                        let att = Attachment {
                            filename,
                            url,
                            content_type: Some(msg_type.to_string()),
                            size,
                            auth_header: Some(auth),
                        };
                        Some(mk_ctx(format!("[{}]", msg_type), vec![att], message_id))
                    }
                    other => {
                        debug!(msg_type = other, "line: unsupported message subtype");
                        None
                    }
                }
            }
            "follow" => {
                // No agent dispatch. The auto-bind path in handle_webhook_payload
                // already registered the user as an unknown contact for later
                // promotion via TUI / `contacts_update`.
                info!(user_id, "line follow event — silent (auto-registered as unknown contact)");
                None
            }
            "unfollow" => {
                // Mark the contact (if any) as ai_paused + tag 'unfollowed' so
                // later attempts to message them are explicitly halted.
                info!(user_id, "line unfollow event");
                None
            }
            "postback" => {
                let data = event
                    .get("postback")
                    .and_then(|p| p.get("data"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(mk_ctx(format!("[LINE postback] {}", data), vec![], None))
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

    /// Send a Flex message either via reply token or push.
    async fn send_flex(&self, target: &str, alt_text: &str, contents: Value) -> Result<Value> {
        let body = serde_json::json!({
            "to": target,
            "messages": [{
                "type": "flex",
                "altText": alt_text,
                "contents": contents,
            }]
        });
        let resp = self
            .http
            .post(format!("{}/message/push", LINE_API))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line flex push: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(CatClawError::Channel(format!(
                "line flex push failed: {} {}", status, txt
            )));
        }
        Ok(serde_json::json!({"sent": true, "target": target}))
    }

    async fn show_loading(&self, user_id: &str, seconds: u32) -> Result<Value> {
        let body = serde_json::json!({
            "chatId": user_id,
            "loadingSeconds": seconds.min(60),
        });
        let resp = self
            .http
            .post(format!("{}/chat/loading/start", LINE_API))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line loading: {e}")))?;
        if !resp.status().is_success() {
            return Err(CatClawError::Channel(format!(
                "line loading failed: {}", resp.status()
            )));
        }
        Ok(serde_json::json!({"started": true}))
    }

    async fn line_get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(format!("{}{}", LINE_API, path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line GET {path}: {e}")))?;
        let status = resp.status();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(CatClawError::Channel(format!(
                "line GET {path} failed: {} {}", status, json
            )));
        }
        Ok(json)
    }

    async fn line_post_json(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}{}", LINE_API, path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line POST {path}: {e}")))?;
        let status = resp.status();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(CatClawError::Channel(format!(
                "line POST {path} failed: {} {}", status, json
            )));
        }
        Ok(json)
    }

    async fn line_delete(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .delete(format!("{}{}", LINE_API, path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line DELETE {path}: {e}")))?;
        if !resp.status().is_success() {
            return Err(CatClawError::Channel(format!(
                "line DELETE {path} failed: {}", resp.status()
            )));
        }
        Ok(serde_json::json!({"deleted": true}))
    }

    async fn rich_menu_upload_image(&self, menu_id: &str, image_path: &str) -> Result<Value> {
        let bytes = std::fs::read(image_path)
            .map_err(|e| CatClawError::Channel(format!("read image '{image_path}': {e}")))?;
        let content_type = if image_path.to_lowercase().ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        let url = format!("https://api-data.line.me/v2/bot/richmenu/{menu_id}/content");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| CatClawError::Channel(format!("line rich menu upload: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(CatClawError::Channel(format!(
                "line rich menu upload failed: {} {}", status, txt
            )));
        }
        Ok(serde_json::json!({"uploaded": true, "menu_id": menu_id}))
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
        line_actions()
    }

    async fn execute(&self, action: &str, params: Value) -> Result<Value> {
        execute_line_action(self, action, params).await
    }

    fn platform_name(&self) -> &str {
        "line"
    }
}

// ── LINE-specific MCP actions ─────────────────────────────────────────────────

fn line_actions() -> Vec<ActionInfo> {
    vec![
        ActionInfo {
            name: "rich_menu_create".into(),
            description: "Create a LINE rich menu. Returns menu_id. Use rich_menu_upload_image next \
                          to attach the background image, then rich_menu_link_user to apply to a user."
                .into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{
                    "name":{"type":"string"},
                    "chat_bar_text":{"type":"string","description":"Text shown on the chat bar (max 14 chars)"},
                    "size":{"type":"object","properties":{"width":{"type":"integer"},"height":{"type":"integer"}},"description":"Standard sizes: 2500x1686 (full) or 2500x843 (compact)"},
                    "areas":{"type":"array","description":"Tap area definitions: [{bounds:{x,y,width,height}, action:{type,...}}]"},
                    "selected":{"type":"boolean","description":"Default visibility (default true)"}
                },
                "required":["name","chat_bar_text","size","areas"]
            }),
        },
        ActionInfo {
            name: "rich_menu_upload_image".into(),
            description: "Upload background image (JPEG/PNG) for a rich menu. image_path must be an absolute local path.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"menu_id":{"type":"string"},"image_path":{"type":"string"}},
                "required":["menu_id","image_path"]
            }),
        },
        ActionInfo {
            name: "rich_menu_list".into(),
            description: "List all rich menus on this LINE OA.".into(),
            params_schema: serde_json::json!({"type":"object","properties":{},"required":[]}),
        },
        ActionInfo {
            name: "rich_menu_delete".into(),
            description: "Delete a rich menu.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"menu_id":{"type":"string"}},
                "required":["menu_id"]
            }),
        },
        ActionInfo {
            name: "rich_menu_set_default".into(),
            description: "Set a rich menu as the OA-wide default (shown to users without per-user override).".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"menu_id":{"type":"string"}},
                "required":["menu_id"]
            }),
        },
        ActionInfo {
            name: "rich_menu_link_user".into(),
            description: "Apply a rich menu to a specific user (overrides default). Use this to \
                          differentiate admin vs client menus.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"menu_id":{"type":"string"},"line_user_id":{"type":"string"}},
                "required":["menu_id","line_user_id"]
            }),
        },
        ActionInfo {
            name: "rich_menu_unlink_user".into(),
            description: "Remove the per-user rich menu override (user falls back to default).".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"line_user_id":{"type":"string"}},
                "required":["line_user_id"]
            }),
        },
        ActionInfo {
            name: "get_quota".into(),
            description: "Get LINE message quota (push API monthly limit + usage).".into(),
            params_schema: serde_json::json!({"type":"object","properties":{},"required":[]}),
        },
        ActionInfo {
            name: "get_profile".into(),
            description: "Get a LINE user's display name + picture URL.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{"line_user_id":{"type":"string"}},
                "required":["line_user_id"]
            }),
        },
        ActionInfo {
            name: "send_flex".into(),
            description: "Send a Flex message (rich UI) to a user/group/room. `contents` follows LINE Flex Message JSON schema.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{
                    "target":{"type":"string","description":"LINE userId / groupId / roomId"},
                    "alt_text":{"type":"string","description":"Fallback text shown in notifications"},
                    "contents":{"type":"object","description":"Flex Message contents (Bubble or Carousel)"}
                },
                "required":["target","alt_text","contents"]
            }),
        },
        ActionInfo {
            name: "show_loading".into(),
            description: "Show a loading animation in a 1:1 chat for up to 60 seconds.".into(),
            params_schema: serde_json::json!({
                "type":"object",
                "properties":{
                    "line_user_id":{"type":"string"},
                    "seconds":{"type":"integer","description":"5-60, rounded to nearest 5 by LINE"}
                },
                "required":["line_user_id"]
            }),
        },
    ]
}

async fn execute_line_action(adapter: &LineAdapter, action: &str, params: Value) -> Result<Value> {
    let s = |k: &str| -> Result<String> {
        params.get(k).and_then(|v| v.as_str()).map(String::from).ok_or_else(|| {
            CatClawError::Channel(format!("missing arg '{}' for line.{}", k, action))
        })
    };
    match action {
        "rich_menu_create" => {
            let name = s("name")?;
            let chat_bar_text = s("chat_bar_text")?;
            let size = params
                .get("size")
                .cloned()
                .ok_or_else(|| CatClawError::Channel("missing 'size'".into()))?;
            let areas = params
                .get("areas")
                .cloned()
                .ok_or_else(|| CatClawError::Channel("missing 'areas'".into()))?;
            let selected = params
                .get("selected")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let body = serde_json::json!({
                "size": size,
                "selected": selected,
                "name": name,
                "chatBarText": chat_bar_text,
                "areas": areas,
            });
            adapter.line_post_json("/richmenu", body).await
        }
        "rich_menu_upload_image" => {
            let menu_id = s("menu_id")?;
            let image_path = s("image_path")?;
            adapter.rich_menu_upload_image(&menu_id, &image_path).await
        }
        "rich_menu_list" => adapter.line_get("/richmenu/list").await,
        "rich_menu_delete" => {
            let menu_id = s("menu_id")?;
            adapter.line_delete(&format!("/richmenu/{}", menu_id)).await
        }
        "rich_menu_set_default" => {
            let menu_id = s("menu_id")?;
            adapter
                .line_post_json(&format!("/user/all/richmenu/{}", menu_id), serde_json::json!({}))
                .await
        }
        "rich_menu_link_user" => {
            let menu_id = s("menu_id")?;
            let user_id = s("line_user_id")?;
            adapter
                .line_post_json(
                    &format!("/user/{}/richmenu/{}", user_id, menu_id),
                    serde_json::json!({}),
                )
                .await
        }
        "rich_menu_unlink_user" => {
            let user_id = s("line_user_id")?;
            adapter.line_delete(&format!("/user/{}/richmenu", user_id)).await
        }
        "get_quota" => adapter.line_get("/message/quota").await,
        "get_profile" => {
            let user_id = s("line_user_id")?;
            adapter.line_get(&format!("/profile/{}", user_id)).await
        }
        "send_flex" => {
            let target = s("target")?;
            let alt = s("alt_text")?;
            let contents = params
                .get("contents")
                .cloned()
                .ok_or_else(|| CatClawError::Channel("missing 'contents'".into()))?;
            adapter.send_flex(&target, &alt, contents).await
        }
        "show_loading" => {
            let user_id = s("line_user_id")?;
            let seconds = params
                .get("seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as u32;
            adapter.show_loading(&user_id, seconds).await
        }
        // Used by contacts pipeline for work-card rendering. LINE doesn't have
        // an admin-channel UI in this design (admin uses Discord/Slack); we
        // accept the call to silence the not-supported error.
        "contact_work_card" | "contact_work_card_edit" => Ok(serde_json::json!({})),
        other => Err(CatClawError::Channel(format!(
            "action '{}' not supported by line adapter", other
        ))),
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
    let (contacts_enabled, default_agent) = {
        let cfg = gw.config.read().unwrap();
        (
            cfg.contacts.enabled,
            cfg.default_agent_id().unwrap_or("main").to_string(),
        )
    };
    adapter
        .handle_webhook_payload(payload, &gw.state_db, &default_agent, contacts_enabled)
        .await;
    axum::http::StatusCode::OK
}
