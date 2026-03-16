use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{
    ChatId, ChatKind, ChatPermissions, InlineKeyboardButton, InlineKeyboardMarkup,
    MessageId as TgMessageId, ThreadId, UserId as TgUserId,
};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info};

use super::{
    ActionInfo, AdapterFilter, ChannelAdapter, ChannelCapabilities, ChannelType, MsgContext,
    OutboundMessage, TypingGuard,
};
use crate::error::{CatClawError, Result};

/// Telegram channel adapter using teloxide (long polling)
pub struct TelegramAdapter {
    token: String,
    filter: std::sync::Arc<std::sync::RwLock<AdapterFilter>>,
    bot: RwLock<Option<Bot>>,
    /// Sender half for approval decisions from callback_query → gateway.
    approval_tx: mpsc::UnboundedSender<(String, bool)>,
    /// Receiver half — taken once by gateway to wire into approval handling loop.
    approval_rx: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<(String, bool)>>>,
}

impl TelegramAdapter {
    pub fn new(token: String, filter: std::sync::Arc<std::sync::RwLock<AdapterFilter>>) -> Self {
        let (approval_tx, approval_rx) = mpsc::unbounded_channel();
        TelegramAdapter {
            token,
            filter,
            bot: RwLock::new(None),
            approval_tx,
            approval_rx: tokio::sync::Mutex::new(Some(approval_rx)),
        }
    }

    /// Take the approval receiver (called once by gateway to wire into approval handling).
    pub async fn take_approval_rx(&self) -> Option<mpsc::UnboundedReceiver<(String, bool)>> {
        self.approval_rx.lock().await.take()
    }

    pub fn from_config(config: &crate::config::ChannelConfig) -> Result<(Self, std::sync::Arc<std::sync::RwLock<AdapterFilter>>)> {
        let token = std::env::var(&config.token_env).map_err(|_| {
            CatClawError::Config(format!(
                "environment variable {} not set",
                config.token_env
            ))
        })?;

        let filter = std::sync::Arc::new(std::sync::RwLock::new(AdapterFilter::from_config(config)));

        Ok((
            TelegramAdapter::new(token, filter.clone()),
            filter,
        ))
    }

    /// Get a clone of the shared filter Arc (for gateway hot-reload).
    #[allow(dead_code)]
    pub fn filter(&self) -> std::sync::Arc<std::sync::RwLock<AdapterFilter>> {
        self.filter.clone()
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn start(&self, msg_tx: mpsc::Sender<MsgContext>) -> Result<()> {
        let bot = Bot::new(&self.token);

        // Store bot reference for send()
        {
            let mut bot_ref = self.bot.write().await;
            *bot_ref = Some(bot.clone());
        }

        let filter_arc = self.filter.clone();

        // Get bot info for mention detection
        let me = bot
            .get_me()
            .await
            .map_err(|e| CatClawError::Telegram(format!("failed to get bot info: {}", e)))?;
        let bot_username = me.username.clone().unwrap_or_default();
        info!(username = %bot_username, "Telegram bot connected");

        // Use teloxide dispatcher with long polling
        let handler = Update::filter_message().endpoint(
            move |bot: Bot, msg: teloxide::types::Message| {
                let msg_tx = msg_tx.clone();
                let filter_arc = filter_arc.clone();
                let bot_username = bot_username.clone();
                async move {
                    // Extract text: from text message or caption on media messages
                    let text = msg
                        .text()
                        .or_else(|| msg.caption())
                        .map(|t| t.to_string())
                        .unwrap_or_default();

                    // Collect attachments (document, photo, audio, video, voice)
                    let mut attachments: Vec<super::Attachment> = Vec::new();

                    if let Some(doc) = msg.document() {
                        if let Ok(file) = bot.get_file(&doc.file.id).await {
                            let url = format!(
                                "https://api.telegram.org/file/bot{}/{}",
                                bot.token(),
                                file.path
                            );
                            attachments.push(super::Attachment {
                                filename: doc.file_name.clone().unwrap_or_else(|| "document".into()),
                                url,
                                content_type: doc.mime_type.as_ref().map(|m| m.to_string()),
                                size: Some(doc.file.size as u64),
                            });
                        }
                    }

                    if let Some(photos) = msg.photo() {
                        // Use the largest photo (last in the array)
                        if let Some(photo) = photos.last() {
                            if let Ok(file) = bot.get_file(&photo.file.id).await {
                                let url = format!(
                                    "https://api.telegram.org/file/bot{}/{}",
                                    bot.token(),
                                    file.path
                                );
                                attachments.push(super::Attachment {
                                    filename: "photo.jpg".into(),
                                    url,
                                    content_type: Some("image/jpeg".into()),
                                    size: Some(photo.file.size as u64),
                                });
                            }
                        }
                    }

                    if let Some(audio) = msg.audio() {
                        if let Ok(file) = bot.get_file(&audio.file.id).await {
                            let url = format!(
                                "https://api.telegram.org/file/bot{}/{}",
                                bot.token(),
                                file.path
                            );
                            attachments.push(super::Attachment {
                                filename: audio.file_name.clone().unwrap_or_else(|| "audio".into()),
                                url,
                                content_type: audio.mime_type.as_ref().map(|m| m.to_string()),
                                size: Some(audio.file.size as u64),
                            });
                        }
                    }

                    if let Some(video) = msg.video() {
                        if let Ok(file) = bot.get_file(&video.file.id).await {
                            let url = format!(
                                "https://api.telegram.org/file/bot{}/{}",
                                bot.token(),
                                file.path
                            );
                            attachments.push(super::Attachment {
                                filename: video.file_name.clone().unwrap_or_else(|| "video".into()),
                                url,
                                content_type: video.mime_type.as_ref().map(|m| m.to_string()),
                                size: Some(video.file.size as u64),
                            });
                        }
                    }

                    if let Some(voice) = msg.voice() {
                        if let Ok(file) = bot.get_file(&voice.file.id).await {
                            let url = format!(
                                "https://api.telegram.org/file/bot{}/{}",
                                bot.token(),
                                file.path
                            );
                            attachments.push(super::Attachment {
                                filename: "voice.ogg".into(),
                                url,
                                content_type: voice.mime_type.as_ref().map(|m| m.to_string()),
                                size: Some(voice.file.size as u64),
                            });
                        }
                    }

                    // Skip messages with no text and no attachments
                    if text.is_empty() && attachments.is_empty() {
                        return Ok::<(), teloxide::RequestError>(());
                    }

                    let chat_id_str = msg.chat.id.0.to_string();
                    let is_private = matches!(msg.chat.kind, ChatKind::Private { .. });

                    let sender_id = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.0.to_string())
                        .unwrap_or_default();

                    // Read filter (nanosecond read lock)
                    let activation = {
                        let filter = filter_arc.read().unwrap();

                        // Check sender policy
                        if !filter.is_sender_allowed(is_private, &sender_id) {
                            return Ok(());
                        }

                        filter.activation_for("telegram:chat", &chat_id_str).to_string()
                    };

                    let should_respond = match activation.as_str() {
                        "all" => true,
                        "mention" => {
                            is_private
                                || text.contains(&format!("@{}", bot_username))
                        }
                        _ => false,
                    };

                    if !should_respond {
                        return Ok(());
                    }

                    // Build sender info
                    let sender_name = if let Some(from) = &msg.from {
                        let mut name = from.first_name.clone();
                        if let Some(ref last) = from.last_name {
                            name.push(' ');
                            name.push_str(last);
                        }
                        name
                    } else {
                        "Unknown".into()
                    };

                    let channel_name = if is_private {
                        let username = msg
                            .from
                            .as_ref()
                            .and_then(|u| u.username.clone())
                            .unwrap_or_else(|| sender_name.clone());
                        Some(format!("dm.{}", username))
                    } else {
                        msg.chat.title().map(|t| t.to_string())
                    };

                    let thread_id = msg.thread_id.map(|t| t.to_string());

                    let ctx = MsgContext {
                        channel_type: ChannelType::Telegram,
                        channel_id: chat_id_str.clone(),
                        peer_id: chat_id_str,
                        sender_id,
                        sender_name,
                        text,
                        attachments,
                        reply_to: msg.reply_to_message().map(|r| {
                            super::ReplyContext {
                                message_id: r.id.0.to_string(),
                                text: r.text().map(|t| t.to_string()),
                            }
                        }),
                        thread_id,
                        is_direct_message: is_private,
                        raw_event: serde_json::json!({
                            "message_id": msg.id.0.to_string(),
                        }),
                        channel_name,
                        guild_id: None,
                    };

                    if let Err(e) = msg_tx.send(ctx).await {
                        error!(error = %e, "failed to send telegram message to router");
                    }

                    Ok(())
                }
            },
        );

        // Callback query handler for approval button presses
        let approval_tx = self.approval_tx.clone();
        let callback_handler = Update::filter_callback_query().endpoint(
            move |bot: Bot, q: teloxide::types::CallbackQuery| {
                let approval_tx = approval_tx.clone();
                async move {
                    if let Some(data) = &q.data {
                        let (request_id, approved) = if let Some(rid) = data.strip_prefix("approve:") {
                            (rid.to_string(), true)
                        } else if let Some(rid) = data.strip_prefix("deny:") {
                            (rid.to_string(), false)
                        } else {
                            return Ok(());
                        };

                        let _ = approval_tx.send((request_id, approved));

                        let answer = if approved { "✅ Approved" } else { "❌ Denied" };
                        if let Err(e) = bot.answer_callback_query(&q.id).text(answer).await {
                            error!(error = %e, "failed to answer callback query");
                        }

                        // Remove inline keyboard from original message
                        if let Some(msg) = &q.message {
                            let chat_id = msg.chat().id;
                            let msg_id = msg.id();
                            let empty_kb = InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new());
                            if let Err(e) = bot.edit_message_reply_markup(chat_id, msg_id)
                                .reply_markup(empty_kb)
                                .await
                            {
                                error!(error = %e, "failed to remove approval keyboard");
                            }
                        }
                    }
                    Ok::<(), teloxide::RequestError>(())
                }
            },
        );

        let combined_handler = dptree::entry()
            .branch(handler)
            .branch(callback_handler);

        let mut dispatcher = Dispatcher::builder(bot, combined_handler)
            .enable_ctrlc_handler()
            .build();

        dispatcher.dispatch().await;

        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let bot = self.bot.read().await;
        let bot = bot
            .as_ref()
            .ok_or_else(|| CatClawError::Telegram("not connected".into()))?;

        let chat_id: i64 = msg
            .channel_id
            .parse()
            .map_err(|_| CatClawError::Telegram("invalid chat_id".into()))?;

        bot.send_message(ChatId(chat_id), &msg.text)
            .await
            .map_err(|e| CatClawError::Telegram(format!("send_message: {}", e)))?;

        Ok(())
    }

    async fn send_approval(
        &self,
        channel_id: &str,
        _peer_id: &str,
        request_id: &str,
        tool_name: &str,
        tool_input: &serde_json::Value,
    ) -> Result<()> {
        let bot = self.bot.read().await;
        let bot = bot
            .as_ref()
            .ok_or_else(|| CatClawError::Telegram("not connected".into()))?;

        let chat_id: i64 = channel_id
            .parse()
            .map_err(|_| CatClawError::Telegram("invalid chat_id".into()))?;

        let input_str = serde_json::to_string_pretty(tool_input)
            .unwrap_or_else(|_| tool_input.to_string());
        // Truncate for Telegram message limits
        let input_preview = if input_str.len() > 3000 {
            format!("{}…", &input_str[..3000])
        } else {
            input_str
        };

        let text = format!(
            "🔒 *Approval Required*\nTool: `{}`\n```json\n{}\n```",
            tool_name, input_preview
        );

        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("✅ Approve", format!("approve:{}", request_id)),
            InlineKeyboardButton::callback("❌ Deny", format!("deny:{}", request_id)),
        ]]);

        bot.send_message(ChatId(chat_id), &text)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await
            .map_err(|e| CatClawError::Telegram(format!("send_approval: {}", e)))?;

        Ok(())
    }

    async fn start_typing(&self, channel_id: &str, _peer_id: &str) -> Result<TypingGuard> {
        let bot = self.bot.read().await;
        let bot = match bot.as_ref() {
            Some(b) => b.clone(),
            None => return Ok(TypingGuard::noop()),
        };

        let chat_id: i64 = channel_id
            .parse()
            .map_err(|_| CatClawError::Telegram("invalid chat_id".into()))?;

        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            loop {
                let _ = bot
                    .send_chat_action(ChatId(chat_id), teloxide::types::ChatAction::Typing)
                    .await;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {},
                    _ = &mut cancel_rx => break,
                }
            }
        });

        Ok(TypingGuard::new(cancel_tx))
    }

    async fn create_thread(&self, _channel_id: &str, _title: &str) -> Result<String> {
        Err(CatClawError::Telegram(
            "threads not supported in Telegram adapter".into(),
        ))
    }

    fn name(&self) -> &str {
        "telegram"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            threading: false,
            typing_indicator: true,
            message_editing: true,
            max_message_length: 4096,
            attachments: true,
        }
    }

    async fn execute(&self, action: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let bot = self.bot.read().await;
        let bot = bot
            .as_ref()
            .ok_or_else(|| CatClawError::Telegram("not connected".into()))?;

        match action {
            // ── Messages ──────────────────────────────────────────────
            "send_message" => {
                let cid = p_chat(&params)?;
                let text = p_str(&params, "text")?;
                let msg = bot.send_message(cid, text).await
                    .map_err(|e| CatClawError::Telegram(format!("send_message: {}", e)))?;
                Ok(serde_json::json!({"message_id": msg.id.0}))
            }
            "edit_message" => {
                let cid = p_chat(&params)?;
                let mid = p_msg_id(&params)?;
                let text = p_str(&params, "text")?;
                bot.edit_message_text(cid, mid, text).await
                    .map_err(|e| CatClawError::Telegram(format!("edit_message: {}", e)))?;
                Ok(serde_json::json!({"edited": true}))
            }
            "delete_message" => {
                let cid = p_chat(&params)?;
                let mid = p_msg_id(&params)?;
                bot.delete_message(cid, mid).await
                    .map_err(|e| CatClawError::Telegram(format!("delete_message: {}", e)))?;
                Ok(serde_json::json!({"deleted": true}))
            }
            "forward_message" => {
                let cid = p_chat(&params)?;
                let from = parse_chat_id(&params, "from_chat_id")?;
                let mid = p_msg_id(&params)?;
                let msg = bot.forward_message(cid, ChatId(from), mid).await
                    .map_err(|e| CatClawError::Telegram(format!("forward_message: {}", e)))?;
                Ok(serde_json::json!({"message_id": msg.id.0}))
            }
            "copy_message" => {
                let cid = p_chat(&params)?;
                let from = parse_chat_id(&params, "from_chat_id")?;
                let mid = p_msg_id(&params)?;
                let result = bot.copy_message(cid, ChatId(from), mid).await
                    .map_err(|e| CatClawError::Telegram(format!("copy_message: {}", e)))?;
                Ok(serde_json::json!({"message_id": result.0}))
            }

            // ── Pins ──────────────────────────────────────────────────
            "pin_message" => {
                let cid = p_chat(&params)?;
                let mid = p_msg_id(&params)?;
                bot.pin_chat_message(cid, mid).await
                    .map_err(|e| CatClawError::Telegram(format!("pin_message: {}", e)))?;
                Ok(serde_json::json!({"pinned": true}))
            }
            "unpin_message" => {
                let cid = p_chat(&params)?;
                let mid = p_msg_id(&params)?;
                bot.unpin_chat_message(cid).message_id(mid).await
                    .map_err(|e| CatClawError::Telegram(format!("unpin_message: {}", e)))?;
                Ok(serde_json::json!({"unpinned": true}))
            }
            "unpin_all" => {
                let cid = p_chat(&params)?;
                bot.unpin_all_chat_messages(cid).await
                    .map_err(|e| CatClawError::Telegram(format!("unpin_all: {}", e)))?;
                Ok(serde_json::json!({"unpinned_all": true}))
            }

            // ── Chat info ─────────────────────────────────────────────
            "get_chat" => {
                let cid = p_chat(&params)?;
                let chat = bot.get_chat(cid).await
                    .map_err(|e| CatClawError::Telegram(format!("get_chat: {}", e)))?;
                Ok(serde_json::json!({
                    "id": chat.id.0,
                    "title": chat.title(),
                    "kind": format!("{:?}", chat.kind),
                }))
            }
            "get_chat_member_count" => {
                let cid = p_chat(&params)?;
                let count = bot.get_chat_member_count(cid).await
                    .map_err(|e| CatClawError::Telegram(format!("get_chat_member_count: {}", e)))?;
                Ok(serde_json::json!({"count": count}))
            }
            "get_chat_member" => {
                let cid = p_chat(&params)?;
                let uid = p_user_id(&params)?;
                let member = bot.get_chat_member(cid, uid).await
                    .map_err(|e| CatClawError::Telegram(format!("get_chat_member: {}", e)))?;
                Ok(serde_json::json!({
                    "user_id": member.user.id.0,
                    "username": member.user.username,
                    "first_name": member.user.first_name,
                    "status": format!("{:?}", member.kind),
                }))
            }
            "get_chat_administrators" => {
                let cid = p_chat(&params)?;
                let admins = bot.get_chat_administrators(cid).await
                    .map_err(|e| CatClawError::Telegram(format!("get_chat_administrators: {}", e)))?;
                let result: Vec<serde_json::Value> = admins.iter().map(|m| serde_json::json!({
                    "user_id": m.user.id.0,
                    "username": m.user.username,
                    "first_name": m.user.first_name,
                    "status": format!("{:?}", m.kind),
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Chat management ───────────────────────────────────────
            "set_chat_title" => {
                let cid = p_chat(&params)?;
                let title = p_str(&params, "title")?;
                bot.set_chat_title(cid, title).await
                    .map_err(|e| CatClawError::Telegram(format!("set_chat_title: {}", e)))?;
                Ok(serde_json::json!({"updated": true}))
            }
            "set_chat_description" => {
                let cid = p_chat(&params)?;
                let desc = params.get("description").and_then(|v| v.as_str()).unwrap_or("");
                bot.set_chat_description(cid).description(desc).await
                    .map_err(|e| CatClawError::Telegram(format!("set_chat_description: {}", e)))?;
                Ok(serde_json::json!({"updated": true}))
            }

            // ── Moderation ────────────────────────────────────────────
            "ban_member" => {
                let cid = p_chat(&params)?;
                let uid = p_user_id(&params)?;
                let mut req = bot.ban_chat_member(cid, uid);
                if let Some(revoke) = params.get("revoke_messages").and_then(|v| v.as_bool()) {
                    req = req.revoke_messages(revoke);
                }
                req.await.map_err(|e| CatClawError::Telegram(format!("ban_member: {}", e)))?;
                Ok(serde_json::json!({"banned": true}))
            }
            "unban_member" => {
                let cid = p_chat(&params)?;
                let uid = p_user_id(&params)?;
                bot.unban_chat_member(cid, uid).await
                    .map_err(|e| CatClawError::Telegram(format!("unban_member: {}", e)))?;
                Ok(serde_json::json!({"unbanned": true}))
            }
            "restrict_member" => {
                let cid = p_chat(&params)?;
                let uid = p_user_id(&params)?;
                let mut perms = ChatPermissions::empty();
                if params.get("can_send_messages").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::SEND_MESSAGES;
                }
                if params.get("can_send_media").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::SEND_MEDIA_MESSAGES;
                }
                if params.get("can_send_other").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::SEND_OTHER_MESSAGES;
                }
                bot.restrict_chat_member(cid, uid, perms).await
                    .map_err(|e| CatClawError::Telegram(format!("restrict_member: {}", e)))?;
                Ok(serde_json::json!({"restricted": true}))
            }
            "promote_member" => {
                let cid = p_chat(&params)?;
                let uid = p_user_id(&params)?;
                let mut req = bot.promote_chat_member(cid, uid);
                if let Some(v) = params.get("can_manage_chat").and_then(|v| v.as_bool()) {
                    req = req.can_manage_chat(v);
                }
                if let Some(v) = params.get("can_delete_messages").and_then(|v| v.as_bool()) {
                    req = req.can_delete_messages(v);
                }
                if let Some(v) = params.get("can_restrict_members").and_then(|v| v.as_bool()) {
                    req = req.can_restrict_members(v);
                }
                if let Some(v) = params.get("can_promote_members").and_then(|v| v.as_bool()) {
                    req = req.can_promote_members(v);
                }
                if let Some(v) = params.get("can_pin_messages").and_then(|v| v.as_bool()) {
                    req = req.can_pin_messages(v);
                }
                if let Some(v) = params.get("can_invite_users").and_then(|v| v.as_bool()) {
                    req = req.can_invite_users(v);
                }
                req.await.map_err(|e| CatClawError::Telegram(format!("promote_member: {}", e)))?;
                Ok(serde_json::json!({"promoted": true}))
            }

            // ── Polls ─────────────────────────────────────────────────
            "send_poll" => {
                let cid = p_chat(&params)?;
                let question = p_str(&params, "question")?;
                let options: Vec<String> = params.get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .ok_or_else(|| CatClawError::Telegram("missing 'options' array".into()))?;
                let mut req = bot.send_poll(cid, question, options);
                if let Some(anon) = params.get("is_anonymous").and_then(|v| v.as_bool()) {
                    req = req.is_anonymous(anon);
                }
                let msg = req.await
                    .map_err(|e| CatClawError::Telegram(format!("send_poll: {}", e)))?;
                Ok(serde_json::json!({"message_id": msg.id.0}))
            }
            "stop_poll" => {
                let cid = p_chat(&params)?;
                let mid = p_msg_id(&params)?;
                bot.stop_poll(cid, mid).await
                    .map_err(|e| CatClawError::Telegram(format!("stop_poll: {}", e)))?;
                Ok(serde_json::json!({"stopped": true}))
            }

            // ── Forum Topics ──────────────────────────────────────────
            "create_forum_topic" => {
                let cid = p_chat(&params)?;
                let name = p_str(&params, "name")?;
                let color = params.get("icon_color").and_then(|v| v.as_u64()).unwrap_or(0x6FB9F0) as u32;
                let emoji_id = params.get("icon_custom_emoji_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let topic = bot.create_forum_topic(cid, name, color, emoji_id).await
                    .map_err(|e| CatClawError::Telegram(format!("create_forum_topic: {}", e)))?;
                Ok(serde_json::json!({
                    "thread_id": topic.thread_id.0.0,
                    "name": topic.name,
                }))
            }
            "close_forum_topic" => {
                let cid = p_chat(&params)?;
                let thread_id = p_thread_id(&params)?;
                bot.close_forum_topic(cid, thread_id).await
                    .map_err(|e| CatClawError::Telegram(format!("close_forum_topic: {}", e)))?;
                Ok(serde_json::json!({"closed": true}))
            }
            "reopen_forum_topic" => {
                let cid = p_chat(&params)?;
                let thread_id = p_thread_id(&params)?;
                bot.reopen_forum_topic(cid, thread_id).await
                    .map_err(|e| CatClawError::Telegram(format!("reopen_forum_topic: {}", e)))?;
                Ok(serde_json::json!({"reopened": true}))
            }
            "delete_forum_topic" => {
                let cid = p_chat(&params)?;
                let thread_id = p_thread_id(&params)?;
                bot.delete_forum_topic(cid, thread_id).await
                    .map_err(|e| CatClawError::Telegram(format!("delete_forum_topic: {}", e)))?;
                Ok(serde_json::json!({"deleted": true}))
            }

            // ── Chat permissions ──────────────────────────────────────
            "set_chat_permissions" => {
                let cid = p_chat(&params)?;
                let mut perms = ChatPermissions::empty();
                if params.get("can_send_messages").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::SEND_MESSAGES;
                }
                if params.get("can_send_media").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::SEND_MEDIA_MESSAGES;
                }
                if params.get("can_invite_users").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::INVITE_USERS;
                }
                if params.get("can_pin_messages").and_then(|v| v.as_bool()).unwrap_or(false) {
                    perms |= ChatPermissions::PIN_MESSAGES;
                }
                bot.set_chat_permissions(cid, perms).await
                    .map_err(|e| CatClawError::Telegram(format!("set_chat_permissions: {}", e)))?;
                Ok(serde_json::json!({"updated": true}))
            }

            // ── Invite links ──────────────────────────────────────────
            "create_invite_link" => {
                let cid = p_chat(&params)?;
                let mut req = bot.create_chat_invite_link(cid);
                if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
                    req = req.name(name);
                }
                if let Some(limit) = params.get("member_limit").and_then(|v| v.as_u64()) {
                    req = req.member_limit(limit as u32);
                }
                let link = req.await
                    .map_err(|e| CatClawError::Telegram(format!("create_invite_link: {}", e)))?;
                Ok(serde_json::json!({
                    "invite_link": link.invite_link,
                    "name": link.name,
                }))
            }

            _ => Err(CatClawError::Channel(format!(
                "telegram action '{}' not supported",
                action
            ))),
        }
    }

    fn supported_actions(&self) -> Vec<ActionInfo> {
        telegram_action_infos()
    }
}

// ── Helper functions ──────────────────────────────────────────────────

fn parse_chat_id(params: &serde_json::Value, field: &str) -> Result<i64> {
    params.get(field)
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_i64()))
        .ok_or_else(|| CatClawError::Telegram(format!("missing or invalid '{}'", field)))
}

fn p_chat(params: &serde_json::Value) -> Result<ChatId> {
    Ok(ChatId(parse_chat_id(params, "chat_id")?))
}

fn p_msg_id(params: &serde_json::Value) -> Result<TgMessageId> {
    let id = params.get("message_id")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i32>().ok()).or_else(|| v.as_i64().map(|n| n as i32)))
        .ok_or_else(|| CatClawError::Telegram("missing 'message_id'".into()))?;
    Ok(TgMessageId(id))
}

fn p_user_id(params: &serde_json::Value) -> Result<TgUserId> {
    let id = params.get("user_id")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .ok_or_else(|| CatClawError::Telegram("missing 'user_id'".into()))?;
    Ok(TgUserId(id))
}

fn p_str<'a>(params: &'a serde_json::Value, field: &str) -> Result<&'a str> {
    params.get(field).and_then(|v| v.as_str())
        .ok_or_else(|| CatClawError::Telegram(format!("missing '{}'", field)))
}

fn p_thread_id(params: &serde_json::Value) -> Result<ThreadId> {
    let id = params.get("thread_id")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<i32>().ok()).or_else(|| v.as_i64().map(|n| n as i32)))
        .ok_or_else(|| CatClawError::Telegram("missing 'thread_id'".into()))?;
    Ok(ThreadId(TgMessageId(id)))
}

fn tg_action(name: &str, description: &str, schema: serde_json::Value) -> ActionInfo {
    ActionInfo { name: name.into(), description: description.into(), params_schema: schema }
}

fn telegram_action_infos() -> Vec<ActionInfo> {
    let cid = serde_json::json!({"type": "string", "description": "Telegram chat ID"});
    let mid = serde_json::json!({"type": "string", "description": "Telegram message ID"});
    let uid = serde_json::json!({"type": "string", "description": "Telegram user ID"});

    vec![
        // Messages
        tg_action("send_message", "Send a message to a chat", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "text": {"type": "string"}}, "required": ["chat_id", "text"]
        })),
        tg_action("edit_message", "Edit a text message", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "message_id": mid, "text": {"type": "string"}}, "required": ["chat_id", "message_id", "text"]
        })),
        tg_action("delete_message", "Delete a message", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "message_id": mid}, "required": ["chat_id", "message_id"]
        })),
        tg_action("forward_message", "Forward a message to another chat", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "from_chat_id": {"type": "string", "description": "Source chat ID"}, "message_id": mid},
            "required": ["chat_id", "from_chat_id", "message_id"]
        })),
        tg_action("copy_message", "Copy a message to another chat (without forward header)", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "from_chat_id": {"type": "string"}, "message_id": mid},
            "required": ["chat_id", "from_chat_id", "message_id"]
        })),
        // Pins
        tg_action("pin_message", "Pin a message", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "message_id": mid}, "required": ["chat_id", "message_id"]
        })),
        tg_action("unpin_message", "Unpin a specific message", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "message_id": mid}, "required": ["chat_id", "message_id"]
        })),
        tg_action("unpin_all", "Unpin all messages in a chat", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid}, "required": ["chat_id"]
        })),
        // Chat info
        tg_action("get_chat", "Get chat information", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid}, "required": ["chat_id"]
        })),
        tg_action("get_chat_member_count", "Get member count", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid}, "required": ["chat_id"]
        })),
        tg_action("get_chat_member", "Get info about a chat member", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "user_id": uid}, "required": ["chat_id", "user_id"]
        })),
        tg_action("get_chat_administrators", "List chat administrators", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid}, "required": ["chat_id"]
        })),
        // Chat management
        tg_action("set_chat_title", "Set chat title", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "title": {"type": "string"}}, "required": ["chat_id", "title"]
        })),
        tg_action("set_chat_description", "Set chat description", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "description": {"type": "string"}}, "required": ["chat_id"]
        })),
        // Moderation
        tg_action("ban_member", "Ban a user from a chat", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "user_id": uid, "revoke_messages": {"type": "boolean"}},
            "required": ["chat_id", "user_id"]
        })),
        tg_action("unban_member", "Unban a user", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "user_id": uid}, "required": ["chat_id", "user_id"]
        })),
        tg_action("restrict_member", "Restrict a member's permissions", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "user_id": uid, "can_send_messages": {"type": "boolean"}, "can_send_media": {"type": "boolean"}, "can_send_other": {"type": "boolean"}},
            "required": ["chat_id", "user_id"]
        })),
        tg_action("promote_member", "Promote a member to admin with specific permissions", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "user_id": uid, "can_manage_chat": {"type": "boolean"}, "can_delete_messages": {"type": "boolean"}, "can_restrict_members": {"type": "boolean"}, "can_promote_members": {"type": "boolean"}, "can_pin_messages": {"type": "boolean"}, "can_invite_users": {"type": "boolean"}},
            "required": ["chat_id", "user_id"]
        })),
        // Polls
        tg_action("send_poll", "Send a poll to a chat", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "question": {"type": "string"}, "options": {"type": "array", "items": {"type": "string"}}, "is_anonymous": {"type": "boolean"}},
            "required": ["chat_id", "question", "options"]
        })),
        tg_action("stop_poll", "Stop an active poll", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "message_id": mid}, "required": ["chat_id", "message_id"]
        })),
        // Forum Topics
        tg_action("create_forum_topic", "Create a forum topic in a supergroup", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "name": {"type": "string"}, "icon_color": {"type": "integer"}, "icon_custom_emoji_id": {"type": "string"}},
            "required": ["chat_id", "name"]
        })),
        tg_action("close_forum_topic", "Close a forum topic", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "thread_id": {"type": "integer"}}, "required": ["chat_id", "thread_id"]
        })),
        tg_action("reopen_forum_topic", "Reopen a closed forum topic", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "thread_id": {"type": "integer"}}, "required": ["chat_id", "thread_id"]
        })),
        tg_action("delete_forum_topic", "Delete a forum topic", serde_json::json!({
            "type": "object", "properties": {"chat_id": cid, "thread_id": {"type": "integer"}}, "required": ["chat_id", "thread_id"]
        })),
        // Permissions
        tg_action("set_chat_permissions", "Set default chat permissions for all members", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "can_send_messages": {"type": "boolean"}, "can_send_media": {"type": "boolean"}, "can_invite_users": {"type": "boolean"}, "can_pin_messages": {"type": "boolean"}},
            "required": ["chat_id"]
        })),
        // Invite links
        tg_action("create_invite_link", "Create a chat invite link", serde_json::json!({
            "type": "object",
            "properties": {"chat_id": cid, "name": {"type": "string"}, "member_limit": {"type": "integer"}},
            "required": ["chat_id"]
        })),
    ]
}
