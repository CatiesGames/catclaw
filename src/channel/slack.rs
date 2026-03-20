use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info, warn};

use super::{
    ActionInfo, AdapterFilter, Attachment, ChannelAdapter, ChannelCapabilities, ChannelType,
    MsgContext, OutboundMessage, TypingGuard,
};
use crate::error::{CatClawError, Result};

/// Slack channel adapter using Socket Mode (WebSocket) + Web API.
///
/// Requires two tokens:
/// - Bot Token (`xoxb-`): used for Web API calls (chat.postMessage, etc.)
/// - App-Level Token (`xapp-`): used to open Socket Mode WebSocket connection
pub struct SlackAdapter {
    bot_token: String,
    app_token: String,
    filter: Arc<std::sync::RwLock<AdapterFilter>>,
    http: reqwest::Client,
    /// Bot's own user ID (resolved via auth.test on start), used for mention detection.
    bot_user_id: RwLock<Option<String>>,
    /// Sender half for approval decisions from interactive buttons → gateway.
    approval_tx: mpsc::UnboundedSender<(String, bool)>,
    /// Receiver half — taken once by gateway.
    approval_rx: Mutex<Option<mpsc::UnboundedReceiver<(String, bool)>>>,
    /// User display name cache: user_id → display_name
    user_cache: DashMap<String, String>,
    /// Channel name cache: channel_id → channel_name
    channel_cache: DashMap<String, String>,
}

impl SlackAdapter {
    pub fn new(
        bot_token: String,
        app_token: String,
        filter: Arc<std::sync::RwLock<AdapterFilter>>,
    ) -> Self {
        let (approval_tx, approval_rx) = mpsc::unbounded_channel();
        SlackAdapter {
            bot_token,
            app_token,
            filter,
            http: reqwest::Client::new(),
            bot_user_id: RwLock::new(None),
            approval_tx,
            approval_rx: Mutex::new(Some(approval_rx)),
            user_cache: DashMap::new(),
            channel_cache: DashMap::new(),
        }
    }

    /// Take the approval receiver (called once by gateway to wire into approval handling).
    pub async fn take_approval_rx(&self) -> Option<mpsc::UnboundedReceiver<(String, bool)>> {
        self.approval_rx.lock().await.take()
    }

    pub fn from_config(
        config: &crate::config::ChannelConfig,
    ) -> Result<(Self, Arc<std::sync::RwLock<AdapterFilter>>)> {
        let bot_token = std::env::var(&config.token_env).map_err(|_| {
            CatClawError::Config(format!(
                "environment variable {} not set",
                config.token_env
            ))
        })?;

        // Validate bot token format
        if !bot_token.starts_with("xoxb-") {
            return Err(CatClawError::Config(format!(
                "slack bot token (from {}) should start with 'xoxb-'. \
                 Did you swap the bot token and app token?",
                config.token_env
            )));
        }

        let app_token_env = config.app_token_env.as_deref().ok_or_else(|| {
            CatClawError::Config(
                "slack adapter requires 'app_token_env' for Socket Mode".into(),
            )
        })?;
        let app_token = std::env::var(app_token_env).map_err(|_| {
            CatClawError::Config(format!(
                "environment variable {} not set",
                app_token_env
            ))
        })?;

        // Validate app token format
        if !app_token.starts_with("xapp-") {
            return Err(CatClawError::Config(format!(
                "slack app-level token (from {}) should start with 'xapp-'. \
                 Did you swap the bot token and app token?",
                app_token_env
            )));
        }

        let filter = Arc::new(std::sync::RwLock::new(AdapterFilter::from_config(config)));

        Ok((
            SlackAdapter::new(bot_token, app_token, filter.clone()),
            filter,
        ))
    }

    /// Get a clone of the shared filter Arc (for gateway hot-reload).
    #[allow(dead_code)]
    pub fn filter(&self) -> Arc<std::sync::RwLock<AdapterFilter>> {
        self.filter.clone()
    }

    // ── Internal helpers ──────────────────────────────────────────────

    /// Call a Slack Web API method. Returns the JSON response body.
    async fn api(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        slack_api(&self.http, &self.bot_token, method, body).await
    }
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    async fn start(&self, msg_tx: mpsc::Sender<MsgContext>) -> Result<()> {
        // Resolve bot user ID via auth.test
        let auth = self.api("auth.test", &serde_json::json!({})).await?;
        let bot_uid = auth
            .get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        info!(bot_user_id = %bot_uid, "Slack bot authenticated");
        {
            let mut uid = self.bot_user_id.write().await;
            *uid = Some(bot_uid.clone());
        }

        // Socket Mode reconnection loop
        let filter_arc = self.filter.clone();
        let approval_tx = self.approval_tx.clone();
        let http = self.http.clone();
        let bot_token = self.bot_token.clone();
        let app_token = self.app_token.clone();
        let user_cache = self.user_cache.clone();
        let channel_cache = self.channel_cache.clone();

        let mut backoff_secs = 1u64;

        // Dedup set for Socket Mode retries: Slack may redeliver events when
        // the ack doesn't arrive in time (e.g. during gateway restart).
        // We track recently seen client_msg_id values and skip duplicates.
        let seen_msgs: Arc<DashMap<String, std::time::Instant>> = Arc::new(DashMap::new());

        loop {
            // 1. Get WSS URL
            let wss_url = match slack_api(&http, &app_token, "apps.connections.open", &serde_json::json!({})).await {
                Ok(resp) => match resp.get("url").and_then(|u| u.as_str()) {
                    Some(url) => url.to_string(),
                    None => {
                        error!("apps.connections.open did not return URL");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(60);
                        continue;
                    }
                },
                Err(e) => {
                    error!(error = %e, "failed to open socket mode connection");
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(60);
                    continue;
                }
            };

            // 2. Connect WebSocket
            let ws_stream = match tokio_tungstenite::connect_async(&wss_url).await {
                Ok((stream, _)) => {
                    info!("slack socket mode connected");
                    backoff_secs = 1;
                    stream
                }
                Err(e) => {
                    error!(error = %e, "slack websocket connect failed");
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(60);
                    continue;
                }
            };

            // 3. Event loop
            use futures::{SinkExt, StreamExt};
            let (mut ws_tx, mut ws_rx) = ws_stream.split();

            let mut disconnected = false;
            while let Some(frame) = ws_rx.next().await {
                let frame = match frame {
                    Ok(f) => f,
                    Err(e) => {
                        warn!(error = %e, "slack ws frame error");
                        disconnected = true;
                        break;
                    }
                };

                let text = match frame {
                    tokio_tungstenite::tungstenite::Message::Text(t) => t,
                    tokio_tungstenite::tungstenite::Message::Ping(data) => {
                        let _ = ws_tx
                            .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                            .await;
                        continue;
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                        info!("slack ws received close frame");
                        disconnected = true;
                        break;
                    }
                    _ => continue,
                };

                let envelope: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "failed to parse slack envelope");
                        continue;
                    }
                };

                let envelope_id = envelope
                    .get("envelope_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let envelope_type = envelope
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Acknowledge the envelope. For interactive payloads, include
                // an empty payload so Slack clears the loading spinner immediately.
                if !envelope_id.is_empty() {
                    let ack = if envelope_type == "interactive" {
                        serde_json::json!({"envelope_id": &envelope_id, "payload": {}})
                    } else {
                        serde_json::json!({"envelope_id": &envelope_id})
                    };
                    let _ = ws_tx
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            ack.to_string(),
                        ))
                        .await;
                }

                match envelope_type {
                    "events_api" => {
                        let payload = match envelope.get("payload") {
                            Some(p) => p,
                            None => continue,
                        };
                        let event = match payload.get("event") {
                            Some(e) => e,
                            None => continue,
                        };
                        let event_type = event
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        if event_type == "assistant_thread_started" {
                            // Don't set thinking status here — the user hasn't sent
                            // a message yet. Status will be set when we receive their
                            // first message (below, before msg_tx.send).
                            continue;
                        }

                        if event_type == "assistant_thread_context_changed" {
                            // Context changed events are informational, skip
                            continue;
                        }

                        // Only handle message events
                        if event_type != "message" && event_type != "app_mention" {
                            continue;
                        }

                        // Skip subtypes like message_changed, bot_message, etc.
                        // but allow file_share (user uploaded a file with optional caption)
                        let subtype = event.get("subtype").and_then(|v| v.as_str());
                        if let Some(st) = subtype {
                            if st != "file_share" {
                                continue;
                            }
                        }
                        let event_user = event
                            .get("user")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if event_user.is_empty() || event_user == bot_uid {
                            continue;
                        }

                        // Dedup: skip Socket Mode retries of the same message.
                        // Use client_msg_id (preferred) or fall back to event_ts.
                        let dedup_key = event
                            .get("client_msg_id")
                            .or_else(|| event.get("event_ts"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !dedup_key.is_empty() {
                            if seen_msgs.contains_key(&dedup_key) {
                                continue;
                            }
                            seen_msgs.insert(dedup_key, std::time::Instant::now());
                            // Prune entries older than 60 seconds
                            seen_msgs.retain(|_, t| t.elapsed().as_secs() < 60);
                        }

                        let channel_id = event
                            .get("channel")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let text_raw = event
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        // Use thread_ts if the message is in a thread, otherwise None
                        // (reply goes to channel root, same as normal conversation)
                        let thread_ts = event
                            .get("thread_ts")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        // team may be missing on some event subtypes (e.g. file_share);
                        // fall back to payload.team_id which is always present.
                        let team_id = event
                            .get("team")
                            .or_else(|| payload.get("team_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        // DM detection: channel IDs starting with "D" are DMs
                        let is_dm = channel_id.starts_with('D');

                        // Check sender policy
                        let activation = {
                            let filter = filter_arc.read().unwrap();
                            if !filter.is_sender_allowed(is_dm, event_user) {
                                continue;
                            }
                            filter
                                .activation_for("slack:channel", &channel_id)
                                .to_string()
                        };

                        // Strip bot mention from text for cleaner input
                        let mention_tag = format!("<@{}>", bot_uid);
                        let clean_text = text_raw.replace(&mention_tag, "").trim().to_string();

                        // Bot commands bypass activation (explicit intent)
                        let cmd = clean_text.split_whitespace().next().unwrap_or("");
                        let is_bot_command = cmd == "/stop" || cmd == "/new";

                        let should_respond = is_bot_command
                            || match activation.as_str() {
                                "all" => true,
                                "mention" => {
                                    is_dm
                                        || event_type == "app_mention"
                                        || text_raw.contains(&mention_tag)
                                }
                                _ => false,
                            };

                        if !should_respond {
                            continue;
                        }

                        // Collect attachments from Slack files
                        let mut attachments = Vec::new();
                        if let Some(files) = event.get("files").and_then(|f| f.as_array()) {
                            for file in files {
                                let filename = file
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("file")
                                    .to_string();
                                // url_private_download requires bot token auth
                                let url = file
                                    .get("url_private_download")
                                    .or_else(|| file.get("url_private"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let content_type = file
                                    .get("mimetype")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let size = file.get("size").and_then(|v| v.as_u64());
                                if !url.is_empty() {
                                    attachments.push(Attachment {
                                        filename,
                                        url,
                                        content_type,
                                        size,
                                        auth_header: Some(format!("Bearer {}", bot_token)),
                                    });
                                }
                            }
                        }

                        // Resolve sender name
                        let sender_name = resolve_user_name_cached(
                            &http,
                            &bot_token,
                            &user_cache,
                            event_user,
                        )
                        .await;

                        // Resolve channel name for session key context (cached)
                        let channel_name = if is_dm {
                            Some(format!("dm.{}", sender_name))
                        } else if let Some(cached) = channel_cache.get(&channel_id) {
                            Some(cached.clone())
                        } else {
                            let name = match slack_api(
                                &http,
                                &bot_token,
                                "conversations.info",
                                &serde_json::json!({"channel": &channel_id}),
                            )
                            .await
                            {
                                Ok(resp) => resp
                                    .get("channel")
                                    .and_then(|c| c.get("name"))
                                    .and_then(|n| n.as_str())
                                    .map(|s| s.to_string()),
                                Err(_) => None,
                            };
                            if let Some(ref n) = name {
                                channel_cache.insert(channel_id.clone(), n.clone());
                            }
                            name
                        };

                        // Set "thinking" status before routing (we have thread_ts here).
                        // This provides the visual indicator in Slack assistant threads.
                        if let Some(ref tts) = &thread_ts {
                            let _ = slack_api(
                                &http,
                                &bot_token,
                                "assistant.threads.setStatus",
                                &serde_json::json!({
                                    "channel_id": &channel_id,
                                    "thread_ts": tts,
                                    "status": "is thinking..."
                                }),
                            )
                            .await;
                        }

                        // Capture message timestamp for reaction status indicator
                        let msg_ts = event
                            .get("ts")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let ctx = MsgContext {
                            channel_type: ChannelType::Slack,
                            channel_id: channel_id.clone(),
                            peer_id: channel_id.clone(),
                            sender_id: event_user.to_string(),
                            sender_name,
                            text: clean_text,
                            attachments,
                            reply_to: None,
                            thread_id: thread_ts,
                            is_direct_message: is_dm,
                            raw_event: serde_json::json!({
                                "team": &team_id,
                                "channel_type": if is_dm { "dm" } else { "channel" },
                            }),
                            channel_name,
                            guild_id: if team_id.is_empty() {
                                None
                            } else {
                                Some(team_id)
                            },
                            message_id: msg_ts,
                        };

                        if let Err(e) = msg_tx.send(ctx).await {
                            error!(error = %e, "failed to send slack message to router");
                        }
                    }

                    "interactive" => {
                        // Block Kit button clicks (approval flow)
                        let payload = match envelope.get("payload") {
                            Some(p) => p,
                            None => continue,
                        };
                        let actions = match payload.get("actions").and_then(|a| a.as_array()) {
                            Some(a) => a,
                            None => continue,
                        };

                        // Extract message info for updating the card after click
                        let msg_channel = payload
                            .get("channel")
                            .and_then(|c| c.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let msg_ts = payload
                            .get("message")
                            .and_then(|m| m.get("ts"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let clicker = payload
                            .get("user")
                            .and_then(|u| u.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("someone");

                        for action in actions {
                            let action_id = action
                                .get("action_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let (request_id, approved) =
                                if let Some(rid) = action_id.strip_prefix("approve:") {
                                    (rid.to_string(), true)
                                } else if let Some(rid) = action_id.strip_prefix("deny:") {
                                    (rid.to_string(), false)
                                } else {
                                    continue;
                                };
                            let _ = approval_tx.send((request_id, approved));

                            // Update the approval card: replace buttons with result
                            if !msg_channel.is_empty() && !msg_ts.is_empty() {
                                let status = if approved { "Approved" } else { "Denied" };
                                let emoji = if approved { ":white_check_mark:" } else { ":x:" };
                                let _ = slack_api(
                                    &http,
                                    &bot_token,
                                    "chat.update",
                                    &serde_json::json!({
                                        "channel": msg_channel,
                                        "ts": msg_ts,
                                        "blocks": [
                                            {
                                                "type": "section",
                                                "text": {
                                                    "type": "mrkdwn",
                                                    "text": format!("{} {} by {}", emoji, status, clicker)
                                                }
                                            }
                                        ],
                                        "text": format!("{} by {}", status, clicker),
                                    }),
                                )
                                .await;
                            }
                        }
                    }

                    "slash_commands" => {
                        let payload = match envelope.get("payload") {
                            Some(p) => p,
                            None => continue,
                        };
                        let command = payload
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let channel_id = payload
                            .get("channel_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let user_id = payload
                            .get("user_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let team_id = payload
                            .get("team_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let slash_text = match command {
                            "/stop" => "/stop",
                            "/new" => "/new",
                            _ => continue,
                        };

                        let thread_ts = payload
                            .get("thread_ts")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());

                        let is_dm = channel_id.starts_with('D');

                        // Check sender policy (slash commands bypass activation but
                        // still respect sender allowlist, same as Telegram)
                        {
                            let filter = filter_arc.read().unwrap();
                            if !filter.is_sender_allowed(is_dm, &user_id) {
                                continue;
                            }
                        }

                        let sender_name = resolve_user_name_cached(
                            &http,
                            &bot_token,
                            &user_cache,
                            &user_id,
                        )
                        .await;

                        let ctx = MsgContext {
                            channel_type: ChannelType::Slack,
                            channel_id: channel_id.clone(),
                            peer_id: channel_id.clone(),
                            sender_id: user_id,
                            sender_name,
                            text: slash_text.to_string(),
                            attachments: vec![],
                            reply_to: None,
                            thread_id: thread_ts,
                            is_direct_message: is_dm,
                            raw_event: serde_json::json!({
                                "team": &team_id,
                            }),
                            channel_name: None,
                            guild_id: if team_id.is_empty() {
                                None
                            } else {
                                Some(team_id)
                            },
                            message_id: None,
                        };

                        if let Err(e) = msg_tx.send(ctx).await {
                            error!(error = %e, "failed to send slack slash command to router");
                        }
                    }

                    "disconnect" => {
                        info!("slack socket mode received disconnect, will reconnect");
                        disconnected = true;
                        break;
                    }

                    _ => {
                        // hello, etc. — ignore
                    }
                }
            }

            if disconnected {
                info!("slack socket mode disconnected, reconnecting...");
                // Clean disconnect (server-initiated) — reset backoff and reconnect quickly
                backoff_secs = 1;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            // Stream ended without explicit disconnect
            warn!("slack ws stream ended unexpectedly, reconnecting...");
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(60);
        }
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let mut body = serde_json::json!({
            "channel": msg.channel_id,
            "text": msg.text,
        });
        if let Some(ref ts) = msg.thread_id {
            body["thread_ts"] = serde_json::Value::String(ts.clone());
        }
        self.api("chat.postMessage", &body).await?;
        Ok(())
    }

    async fn send_approval(
        &self,
        channel_id: &str,
        _peer_id: &str,
        thread_id: Option<&str>,
        request_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> Result<()> {
        let input_str = serde_json::to_string_pretty(tool_input)
            .unwrap_or_else(|_| tool_input.to_string());
        let input_preview = if input_str.len() > 3000 {
            format!("{}...", &input_str[..3000])
        } else {
            input_str
        };

        let blocks = serde_json::json!([
            {
                "type": "header",
                "text": {"type": "plain_text", "text": "Approval Required", "emoji": true}
            },
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!("*Tool:* `{}`\n```{}```", tool_name, input_preview)
                }
            },
            {
                "type": "actions",
                "elements": [
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Approve", "emoji": true},
                        "style": "primary",
                        "action_id": format!("approve:{}", request_id)
                    },
                    {
                        "type": "button",
                        "text": {"type": "plain_text", "text": "Deny", "emoji": true},
                        "style": "danger",
                        "action_id": format!("deny:{}", request_id)
                    }
                ]
            }
        ]);

        let mut body = serde_json::json!({
            "channel": channel_id,
            "text": format!("Approval Required: {}", tool_name),
            "blocks": blocks,
        });
        if let Some(tts) = thread_id {
            body["thread_ts"] = serde_json::Value::String(tts.to_string());
        }
        self.api("chat.postMessage", &body).await?;

        Ok(())
    }

    async fn start_typing(&self, _channel_id: &str, _peer_id: &str) -> Result<TypingGuard> {
        // Slack's typing indicator (assistant.threads.setStatus) requires thread_ts
        // which the ChannelAdapter trait doesn't provide. Thinking status is set in
        // the event handler (before msg_tx.send) where thread_ts is available, and
        // cleared automatically when the bot replies.
        Ok(TypingGuard::noop())
    }

    async fn create_thread(&self, channel_id: &str, title: &str) -> Result<String> {
        let resp = self
            .api(
                "chat.postMessage",
                &serde_json::json!({
                    "channel": channel_id,
                    "text": title,
                }),
            )
            .await?;
        resp.get("ts")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CatClawError::Slack("chat.postMessage did not return ts".into()))
    }

    fn name(&self) -> &str {
        "slack"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            threading: true,
            // Slack's assistant.threads.setStatus requires thread_ts which the trait
            // doesn't provide. Thinking status is set in the event handler instead.
            typing_indicator: false,
            message_editing: true,
            max_message_length: 40000,
            attachments: true,
            streaming: true,
        }
    }

    // ── Streaming API ─────────────────────────────────────────────────

    async fn send_stream_start(&self, channel_id: &str, thread_ts: &str) -> Result<String> {
        if thread_ts.is_empty() {
            return Err(CatClawError::Slack(
                "chat.startStream requires thread_ts".into(),
            ));
        }
        let body = serde_json::json!({
            "channel": channel_id,
            "thread_ts": thread_ts,
        });
        let resp = self.api("chat.startStream", &body).await?;
        resp.get("ts")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CatClawError::Slack("chat.startStream did not return ts".into()))
    }

    async fn send_stream_append(&self, msg_ts: &str, channel_id: &str, text: &str) -> Result<()> {
        self.api(
            "chat.appendStream",
            &serde_json::json!({
                "channel": channel_id,
                "ts": msg_ts,
                "text": text,
            }),
        )
        .await?;
        Ok(())
    }

    async fn send_stream_stop(&self, msg_ts: &str, channel_id: &str, text: Option<&str>) -> Result<()> {
        let mut body = serde_json::json!({
            "channel": channel_id,
            "ts": msg_ts,
        });
        if let Some(final_text) = text {
            body["text"] = serde_json::Value::String(final_text.to_string());
        }
        self.api("chat.stopStream", &body).await?;
        Ok(())
    }

    // ── Platform actions (MCP tools) ──────────────────────────────────

    async fn execute(&self, action: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        match action {
            // ── Messages ──────────────────────────────────────────────
            "send_message" => {
                let channel = p_str(&params, "channel")?;
                let text = p_str(&params, "text")?;
                let mut body = serde_json::json!({"channel": channel, "text": text});
                if let Some(ts) = params.get("thread_ts").and_then(|v| v.as_str()) {
                    body["thread_ts"] = serde_json::Value::String(ts.to_string());
                }
                let resp = self.api("chat.postMessage", &body).await?;
                let ts = resp.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::json!({"ts": ts, "ok": true}))
            }
            "edit_message" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                let text = p_str(&params, "text")?;
                self.api(
                    "chat.update",
                    &serde_json::json!({"channel": channel, "ts": ts, "text": text}),
                )
                .await?;
                Ok(serde_json::json!({"edited": true}))
            }
            "delete_message" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                self.api(
                    "chat.delete",
                    &serde_json::json!({"channel": channel, "ts": ts}),
                )
                .await?;
                Ok(serde_json::json!({"deleted": true}))
            }
            "get_messages" => {
                let channel = p_str(&params, "channel")?;
                let limit = params
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20);
                let resp = self
                    .api(
                        "conversations.history",
                        &serde_json::json!({"channel": channel, "limit": limit}),
                    )
                    .await?;
                Ok(resp
                    .get("messages")
                    .cloned()
                    .unwrap_or(serde_json::json!([])))
            }

            // ── Reactions ─────────────────────────────────────────────
            "react" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                let emoji = p_str(&params, "name")?;
                self.api(
                    "reactions.add",
                    &serde_json::json!({"channel": channel, "timestamp": ts, "name": emoji}),
                )
                .await?;
                Ok(serde_json::json!({"ok": true}))
            }
            "delete_reaction" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                let emoji = p_str(&params, "name")?;
                self.api(
                    "reactions.remove",
                    &serde_json::json!({"channel": channel, "timestamp": ts, "name": emoji}),
                )
                .await?;
                Ok(serde_json::json!({"ok": true}))
            }
            "get_reactions" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                let resp = self
                    .api(
                        "reactions.get",
                        &serde_json::json!({"channel": channel, "timestamp": ts, "full": true}),
                    )
                    .await?;
                Ok(resp
                    .get("message")
                    .and_then(|m| m.get("reactions"))
                    .cloned()
                    .unwrap_or(serde_json::json!([])))
            }

            // ── Pins ──────────────────────────────────────────────────
            "pin_message" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                self.api(
                    "pins.add",
                    &serde_json::json!({"channel": channel, "timestamp": ts}),
                )
                .await?;
                Ok(serde_json::json!({"pinned": true}))
            }
            "unpin_message" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                self.api(
                    "pins.remove",
                    &serde_json::json!({"channel": channel, "timestamp": ts}),
                )
                .await?;
                Ok(serde_json::json!({"unpinned": true}))
            }
            "list_pins" => {
                let channel = p_str(&params, "channel")?;
                let resp = self
                    .api("pins.list", &serde_json::json!({"channel": channel}))
                    .await?;
                Ok(resp.get("items").cloned().unwrap_or(serde_json::json!([])))
            }

            // ── Channels ──────────────────────────────────────────────
            "get_channels" => {
                let types = params
                    .get("types")
                    .and_then(|v| v.as_str())
                    .unwrap_or("public_channel,private_channel");
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200);
                let resp = self
                    .api(
                        "conversations.list",
                        &serde_json::json!({"types": types, "limit": limit}),
                    )
                    .await?;
                Ok(resp
                    .get("channels")
                    .cloned()
                    .unwrap_or(serde_json::json!([])))
            }
            "channel_info" => {
                let channel = p_str(&params, "channel")?;
                let resp = self
                    .api(
                        "conversations.info",
                        &serde_json::json!({"channel": channel}),
                    )
                    .await?;
                Ok(resp
                    .get("channel")
                    .cloned()
                    .unwrap_or(serde_json::json!({})))
            }
            "create_channel" => {
                let name = p_str(&params, "name")?;
                let is_private = params
                    .get("is_private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let resp = self
                    .api(
                        "conversations.create",
                        &serde_json::json!({"name": name, "is_private": is_private}),
                    )
                    .await?;
                Ok(resp
                    .get("channel")
                    .cloned()
                    .unwrap_or(serde_json::json!({})))
            }
            "archive_channel" => {
                let channel = p_str(&params, "channel")?;
                self.api(
                    "conversations.archive",
                    &serde_json::json!({"channel": channel}),
                )
                .await?;
                Ok(serde_json::json!({"archived": true}))
            }

            // ── Threads ───────────────────────────────────────────────
            "get_thread_replies" => {
                let channel = p_str(&params, "channel")?;
                let ts = p_str(&params, "ts")?;
                let resp = self
                    .api(
                        "conversations.replies",
                        &serde_json::json!({"channel": channel, "ts": ts}),
                    )
                    .await?;
                Ok(resp
                    .get("messages")
                    .cloned()
                    .unwrap_or(serde_json::json!([])))
            }

            // ── Users ─────────────────────────────────────────────────
            "user_info" => {
                let user = p_str(&params, "user")?;
                let resp = self
                    .api("users.info", &serde_json::json!({"user": user}))
                    .await?;
                Ok(resp.get("user").cloned().unwrap_or(serde_json::json!({})))
            }
            "list_users" => {
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200);
                let resp = self
                    .api("users.list", &serde_json::json!({"limit": limit}))
                    .await?;
                Ok(resp
                    .get("members")
                    .cloned()
                    .unwrap_or(serde_json::json!([])))
            }

            // ── File Upload ────────────────────────────────────────────
            "upload_file" => {
                let channel = p_str(&params, "channel")?;
                let file_path = p_str(&params, "file_path")?;

                let path = std::path::Path::new(file_path);
                if !path.is_absolute() {
                    return Err(CatClawError::Slack("file_path must be absolute".into()));
                }
                let data = tokio::fs::read(path).await.map_err(|e| {
                    CatClawError::Slack(format!("failed to read '{}': {}", file_path, e))
                })?;
                let length = data.len();

                let filename = params
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                    })
                    .to_string();

                // Step 1: get upload URL
                let step1 = self
                    .api(
                        "files.getUploadURLExternal",
                        &serde_json::json!({"filename": filename, "length": length}),
                    )
                    .await?;
                let upload_url = step1
                    .get("upload_url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        CatClawError::Slack("files.getUploadURLExternal: no upload_url".into())
                    })?;
                let file_id = step1
                    .get("file_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        CatClawError::Slack("files.getUploadURLExternal: no file_id".into())
                    })?
                    .to_string();

                // Step 2: upload binary data
                let upload_resp = self
                    .http
                    .post(upload_url)
                    .header("Content-Type", "application/octet-stream")
                    .body(data)
                    .send()
                    .await
                    .map_err(|e| CatClawError::Slack(format!("upload binary: {}", e)))?;
                if !upload_resp.status().is_success() {
                    return Err(CatClawError::Slack(format!(
                        "upload binary: HTTP {}",
                        upload_resp.status()
                    )));
                }

                // Step 3: complete upload and share to channel
                let title = filename.clone();
                let mut complete_body = serde_json::json!({
                    "files": [{"id": file_id, "title": title}],
                    "channel_id": channel,
                });
                if let Some(msg) = params.get("message").and_then(|v| v.as_str()) {
                    complete_body["initial_comment"] = serde_json::Value::String(msg.to_string());
                }
                if let Some(ts) = params.get("thread_ts").and_then(|v| v.as_str()) {
                    complete_body["thread_ts"] = serde_json::Value::String(ts.to_string());
                }
                self.api("files.completeUploadExternal", &complete_body)
                    .await?;

                Ok(serde_json::json!({"ok": true, "file_id": file_id}))
            }

            _ => Err(CatClawError::Channel(format!(
                "slack action '{}' not supported",
                action
            ))),
        }
    }

    fn supported_actions(&self) -> Vec<ActionInfo> {
        slack_action_infos()
    }

    async fn create_reaction_handle(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<super::reaction::ReactionHandle> {
        Some(super::reaction::spawn_slack(
            self.http.clone(),
            self.bot_token.clone(),
            channel_id.to_string(),
            message_id.to_string(),
        ))
    }
}

// ── Free functions ────────────────────────────────────────────────────

/// Generic Slack Web API caller.
///
/// Uses form-encoded POST (not JSON) because Slack's bot token APIs
/// intermittently reject JSON bodies with errors like `user_not_found`
/// or `invalid_arguments` for valid requests. Form-encoded is universally
/// reliable. Complex values (arrays/objects) are JSON-stringified as
/// form field values per Slack's documentation.
async fn slack_api(
    client: &reqwest::Client,
    token: &str,
    method: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let url = format!("https://slack.com/api/{}", method);

    // Convert JSON object to form fields; stringify complex values
    let mut form: Vec<(String, String)> = Vec::new();
    if let Some(obj) = body.as_object() {
        for (key, value) in obj {
            let v = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Null => continue,
                // Arrays and objects → JSON string
                _ => value.to_string(),
            };
            form.push((key.clone(), v));
        }
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .form(&form)
        .send()
        .await
        .map_err(|e| CatClawError::Slack(format!("{}: {}", method, e)))?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CatClawError::Slack(format!("{}: failed to parse response: {}", method, e)))?;

    if !status.is_success() {
        return Err(CatClawError::Slack(format!(
            "{}: HTTP {}",
            method, status
        )));
    }

    let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let err = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown_error");
        return Err(CatClawError::Slack(format!("{}: {}", method, err)));
    }

    Ok(json)
}

/// Resolve user name with shared cache (used from inside the event loop where &self isn't available).
async fn resolve_user_name_cached(
    http: &reqwest::Client,
    token: &str,
    cache: &DashMap<String, String>,
    user_id: &str,
) -> String {
    if let Some(name) = cache.get(user_id) {
        return name.clone();
    }
    match slack_api(
        http,
        token,
        "users.info",
        &serde_json::json!({"user": user_id}),
    )
    .await
    {
        Ok(resp) => {
            let resolved = resp
                .get("user")
                .and_then(|u| {
                    u.get("profile")
                        .and_then(|p| p.get("display_name").and_then(|n| n.as_str()))
                        .filter(|n| !n.is_empty())
                        .or_else(|| u.get("real_name").and_then(|n| n.as_str()))
                        .or_else(|| u.get("name").and_then(|n| n.as_str()))
                })
                .unwrap_or("")
                .to_string();
            if !resolved.is_empty() {
                cache.insert(user_id.to_string(), resolved.clone());
                resolved
            } else {
                user_id.to_string()
            }
        }
        Err(e) => {
            warn!(error = %e, user_id = user_id, "failed to resolve slack user name");
            user_id.to_string()
        }
    }
}

// ── Parameter helpers ─────────────────────────────────────────────────

fn p_str<'a>(params: &'a serde_json::Value, field: &str) -> Result<&'a str> {
    params
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatClawError::Slack(format!("missing '{}'", field)))
}

// ── Action schema definitions ─────────────────────────────────────────

fn slack_action(name: &str, description: &str, schema: serde_json::Value) -> ActionInfo {
    ActionInfo {
        name: name.into(),
        description: description.into(),
        params_schema: schema,
    }
}

fn slack_action_infos() -> Vec<ActionInfo> {
    let ch = serde_json::json!({"type": "string", "description": "Slack channel ID"});
    let ts = serde_json::json!({"type": "string", "description": "Message timestamp (ts)"});
    let uid = serde_json::json!({"type": "string", "description": "Slack user ID"});

    vec![
        // Messages
        slack_action(
            "send_message",
            "Send a message to a channel",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": ch, "text": {"type": "string"},
                    "thread_ts": {"type": "string", "description": "Thread timestamp to reply in"}
                },
                "required": ["channel", "text"]
            }),
        ),
        slack_action(
            "edit_message",
            "Edit a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts, "text": {"type": "string"}},
                "required": ["channel", "ts", "text"]
            }),
        ),
        slack_action(
            "delete_message",
            "Delete a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts},
                "required": ["channel", "ts"]
            }),
        ),
        slack_action(
            "get_messages",
            "Get recent messages from a channel",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "limit": {"type": "integer", "description": "Number of messages (default 20)"}},
                "required": ["channel"]
            }),
        ),
        // Reactions
        slack_action(
            "react",
            "Add a reaction to a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts, "name": {"type": "string", "description": "Emoji name without colons"}},
                "required": ["channel", "ts", "name"]
            }),
        ),
        slack_action(
            "delete_reaction",
            "Remove a reaction from a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts, "name": {"type": "string"}},
                "required": ["channel", "ts", "name"]
            }),
        ),
        slack_action(
            "get_reactions",
            "Get reactions on a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts},
                "required": ["channel", "ts"]
            }),
        ),
        // Pins
        slack_action(
            "pin_message",
            "Pin a message in a channel",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts},
                "required": ["channel", "ts"]
            }),
        ),
        slack_action(
            "unpin_message",
            "Unpin a message",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts},
                "required": ["channel", "ts"]
            }),
        ),
        slack_action(
            "list_pins",
            "List pinned messages in a channel",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch},
                "required": ["channel"]
            }),
        ),
        // Channels
        slack_action(
            "get_channels",
            "List conversations (channels) in the workspace",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "types": {"type": "string", "description": "Comma-separated types: public_channel,private_channel,mpim,im"},
                    "limit": {"type": "integer"}
                },
                "required": []
            }),
        ),
        slack_action(
            "channel_info",
            "Get information about a channel",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch},
                "required": ["channel"]
            }),
        ),
        slack_action(
            "create_channel",
            "Create a new channel",
            serde_json::json!({
                "type": "object",
                "properties": {"name": {"type": "string"}, "is_private": {"type": "boolean"}},
                "required": ["name"]
            }),
        ),
        slack_action(
            "archive_channel",
            "Archive a channel",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch},
                "required": ["channel"]
            }),
        ),
        // Threads
        slack_action(
            "get_thread_replies",
            "Get replies in a thread",
            serde_json::json!({
                "type": "object",
                "properties": {"channel": ch, "ts": ts},
                "required": ["channel", "ts"]
            }),
        ),
        // Users
        slack_action(
            "user_info",
            "Get information about a user",
            serde_json::json!({
                "type": "object",
                "properties": {"user": uid},
                "required": ["user"]
            }),
        ),
        slack_action(
            "list_users",
            "List workspace members",
            serde_json::json!({
                "type": "object",
                "properties": {"limit": {"type": "integer"}},
                "required": []
            }),
        ),
        // File Upload
        slack_action(
            "upload_file",
            "Upload a local file to a Slack channel",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": ch,
                    "file_path": {"type": "string", "description": "Absolute path to the local file"},
                    "filename": {"type": "string", "description": "Display filename (defaults to basename of file_path)"},
                    "message": {"type": "string", "description": "Initial comment to attach"},
                    "thread_ts": {"type": "string", "description": "Thread timestamp to upload into"}
                },
                "required": ["channel", "file_path"]
            }),
        ),
    ]
}
