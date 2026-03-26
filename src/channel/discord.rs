use async_trait::async_trait;
use serenity::all::{
    ButtonStyle, ChannelId, ChannelType as SerenityChannelType, Command, Context, CreateActionRow,
    CreateAttachment, CreateButton, CreateChannel, CreateCommand, CreateEmbed,
    CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, CreateThread,
    EditChannel, EditMessage, EventHandler, GatewayIntents, GuildId, Interaction, Message,
    MessageId, MessageType, PermissionOverwrite, PermissionOverwriteType, Permissions,
    ReactionType, RoleId, Ready, UserId,
};
use serenity::builder::GetMessages;
use serenity::Client;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info};

use super::{
    ActionInfo, AdapterFilter, Attachment, ChannelAdapter, ChannelCapabilities, ChannelType,
    MsgContext, OutboundMessage, TypingGuard,
};
use crate::error::{CatClawError, Result};

/// Discord channel adapter using serenity
pub struct DiscordAdapter {
    token: String,
    filter: Arc<std::sync::RwLock<AdapterFilter>>,
    http: RwLock<Option<Arc<serenity::http::Http>>>,
    cache: RwLock<Option<Arc<serenity::cache::Cache>>>,
    /// Sender half for approval decisions from interaction_create → gateway.
    approval_tx: mpsc::UnboundedSender<(String, bool)>,
    /// Receiver half — taken once by gateway to wire into approval handling loop.
    approval_rx: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<(String, bool)>>>,
    /// Sender half for social inbox button actions (inbox_id, action, hint) from interaction_create → gateway.
    social_action_tx: mpsc::UnboundedSender<(i64, String, Option<String>)>,
    /// Receiver half — taken once by gateway.
    #[allow(clippy::type_complexity)]
    social_action_rx: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<(i64, String, Option<String>)>>>,
}

struct Handler {
    msg_tx: mpsc::Sender<MsgContext>,
    filter: Arc<std::sync::RwLock<AdapterFilter>>,
    /// Channel to send approval decisions (request_id, approved) back to gateway.
    approval_tx: mpsc::UnboundedSender<(String, bool)>,
    /// Channel to send social inbox button actions back to gateway.
    social_action_tx: mpsc::UnboundedSender<(i64, String, Option<String>)>,
}

impl DiscordAdapter {
    pub fn new(token: String, filter: Arc<std::sync::RwLock<AdapterFilter>>) -> Self {
        let (approval_tx, approval_rx) = mpsc::unbounded_channel();
        let (social_action_tx, social_action_rx) = mpsc::unbounded_channel();
        DiscordAdapter {
            token,
            filter,
            http: RwLock::new(None),
            cache: RwLock::new(None),
            approval_tx,
            approval_rx: tokio::sync::Mutex::new(Some(approval_rx)),
            social_action_tx,
            social_action_rx: tokio::sync::Mutex::new(Some(social_action_rx)),
        }
    }

    /// Take the approval receiver (called once by gateway to wire into approval handling).
    pub async fn take_approval_rx(&self) -> Option<mpsc::UnboundedReceiver<(String, bool)>> {
        self.approval_rx.lock().await.take()
    }

    /// Take the social action receiver (called once by gateway).
    pub async fn take_social_action_rx(&self) -> Option<mpsc::UnboundedReceiver<(i64, String, Option<String>)>> {
        self.social_action_rx.lock().await.take()
    }

    pub fn from_config(config: &crate::config::ChannelConfig) -> Result<(Self, Arc<std::sync::RwLock<AdapterFilter>>)> {
        let token = std::env::var(&config.token_env).map_err(|_| {
            CatClawError::Config(format!(
                "environment variable {} not set",
                config.token_env
            ))
        })?;

        let filter = Arc::new(std::sync::RwLock::new(AdapterFilter::from_config(config)));

        Ok((
            DiscordAdapter::new(token, filter.clone()),
            filter,
        ))
    }

    /// Get a clone of the shared filter Arc (for gateway hot-reload).
    #[allow(dead_code)]
    pub fn filter(&self) -> Arc<std::sync::RwLock<AdapterFilter>> {
        self.filter.clone()
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        // Ignore bot messages
        if msg.author.bot {
            return;
        }

        // Ignore thread creation system messages — these appear in the parent channel
        // when a user creates a thread. Without this filter, the thread title gets
        // routed to the parent channel session as if it were a regular user message.
        if matches!(msg.kind, MessageType::ThreadCreated | MessageType::ThreadStarterMessage) {
            return;
        }

        let channel_id_str = msg.channel_id.get().to_string();
        let is_dm = msg.guild_id.is_none();
        let sender_id = msg.author.id.get().to_string();

        // Read filter inside a block so the guard is dropped before any await
        let activation = {
            let filter = self.filter.read().unwrap();

            // Check guild filter
            if let Some(guild_id) = msg.guild_id {
                if !filter.guilds.is_empty() && !filter.guilds.contains(&guild_id.get()) {
                    return;
                }
            }

            // Check sender policy
            if !filter.is_sender_allowed(is_dm, &sender_id) {
                return;
            }

            filter.activation_for("discord:channel", &channel_id_str).to_string()
        };

        // Check if we should respond
        let should_respond = match activation.as_str() {
            "all" => true,
            "mention" => {
                is_dm || msg.mentions_me(&_ctx.http).await.unwrap_or(false)
            }
            _ => false,
        };

        if !should_respond {
            return;
        }

        // Resolve human-readable channel name from cache
        let channel_name = resolve_channel_name(
            &_ctx.cache,
            msg.channel_id,
            msg.guild_id,
            is_dm,
            &msg.author.name,
        );

        // Build MsgContext
        let text = msg.content.clone();
        let attachments = msg
            .attachments
            .iter()
            .map(|a| Attachment {
                filename: a.filename.clone(),
                url: a.url.clone(),
                content_type: a.content_type.clone(),
                size: Some(a.size as u64),
                auth_header: None,
            })
            .collect();

        // Detect if this message is inside a thread.
        // In Discord, thread channels have their own channel_id.
        // Check guild cache for thread channel type (guild.threads, not guild.channels).
        let thread_id = if !is_dm {
            let is_thread = msg.guild_id.and_then(|gid| {
                _ctx.cache.guild(gid).and_then(|guild| {
                    // guild.channels doesn't include threads; guild.threads does
                    guild.threads.iter()
                        .find(|t| t.id == msg.channel_id)
                        .map(|t| matches!(t.kind, SerenityChannelType::PublicThread | SerenityChannelType::PrivateThread))
                })
            }).unwrap_or(false);
            if is_thread {
                Some(channel_id_str.clone())
            } else {
                None
            }
        } else {
            None
        };

        let peer_id = if is_dm {
            msg.author.id.get().to_string()
        } else {
            channel_id_str.clone()
        };

        let guild_id_str = msg.guild_id.map(|g| g.get().to_string());

        let ctx = MsgContext {
            channel_type: ChannelType::Discord,
            channel_id: channel_id_str,
            peer_id,
            sender_id: msg.author.id.get().to_string(),
            sender_name: msg.author.name.clone(),
            text,
            attachments,
            reply_to: msg.referenced_message.as_ref().map(|r| {
                super::ReplyContext {
                    message_id: r.id.get().to_string(),
                    text: Some(r.content.clone()),
                }
            }),
            thread_id,
            is_direct_message: is_dm,
            raw_event: serde_json::json!({
                "message_id": msg.id.get().to_string(),
                "guild_id": &guild_id_str,
            }),
            channel_name,
            guild_id: guild_id_str,
            message_id: Some(msg.id.get().to_string()),
        };

        if let Err(e) = self.msg_tx.send(ctx).await {
            error!(error = %e, "failed to send message to router");
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            user = %ready.user.name,
            guilds = ready.guilds.len(),
            "Discord bot connected"
        );

        // Register global slash commands (retry on transient HTTP errors)
        let commands = vec![
            CreateCommand::new("stop").description("Stop the current session"),
            CreateCommand::new("new").description("Start a new session (archives current)"),
        ];
        for attempt in 1..=3 {
            match Command::set_global_commands(&ctx.http, commands.clone()).await {
                Ok(_) => {
                    info!("registered global slash commands");
                    break;
                }
                Err(e) => {
                    if attempt < 3 {
                        error!(error = %e, attempt, "failed to register slash commands, retrying...");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    } else {
                        error!(error = %e, "failed to register slash commands after 3 attempts");
                    }
                }
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => {
                let name = cmd.data.name.as_str();
                if name == "stop" || name == "new" {
                    // Check guild filter and sender policy (same checks as message handler)
                    {
                        let filter = self.filter.read().unwrap();
                        if let Some(guild_id) = cmd.guild_id {
                            if !filter.guilds.is_empty()
                                && !filter.guilds.contains(&guild_id.get())
                            {
                                return;
                            }
                        }
                        let is_dm = cmd.guild_id.is_none();
                        let sender_id = cmd.user.id.get().to_string();
                        if !filter.is_sender_allowed(is_dm, &sender_id) {
                            return;
                        }
                    }

                    // Ephemeral ack (only visible to the invoking user)
                    let ack = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!("Running /{}...", name))
                            .ephemeral(true),
                    );
                    let _ = cmd.create_response(&ctx.http, ack).await;

                    // Build MsgContext and route through normal pipeline
                    let channel_id_str = cmd.channel_id.get().to_string();
                    let is_dm = cmd.guild_id.is_none();
                    let sender_id = cmd.user.id.get().to_string();
                    let sender_name = cmd.user.name.clone();

                    let channel_name = resolve_channel_name(
                        &ctx.cache,
                        cmd.channel_id,
                        cmd.guild_id,
                        is_dm,
                        &sender_name,
                    );

                    let peer_id = if is_dm {
                        sender_id.clone()
                    } else {
                        channel_id_str.clone()
                    };

                    // Detect if the command was invoked inside a thread.
                    // Cache::channel() only searches guild.channels (not threads),
                    // so we also check guild.threads for thread channels.
                    let thread_id = if !is_dm {
                        let is_thread = cmd.guild_id.and_then(|gid| {
                            ctx.cache.guild(gid).map(|guild| {
                                // Check guild.channels first, then guild.threads
                                guild
                                    .channels
                                    .get(&cmd.channel_id)
                                    .map(|ch| ch.kind)
                                    .or_else(|| {
                                        guild
                                            .threads
                                            .iter()
                                            .find(|t| t.id == cmd.channel_id)
                                            .map(|t| t.kind)
                                    })
                                    .map(|kind| {
                                        kind == SerenityChannelType::PublicThread
                                            || kind == SerenityChannelType::PrivateThread
                                    })
                                    .unwrap_or(false)
                            })
                        });
                        if is_thread == Some(true) {
                            Some(channel_id_str.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let guild_id_str = cmd.guild_id.map(|g| g.get().to_string());

                    let msg_ctx = MsgContext {
                        channel_type: ChannelType::Discord,
                        channel_id: channel_id_str,
                        peer_id,
                        sender_id,
                        sender_name,
                        text: format!("/{}", name),
                        attachments: vec![],
                        reply_to: None,
                        thread_id,
                        is_direct_message: is_dm,
                        raw_event: serde_json::json!({
                            "guild_id": &guild_id_str,
                        }),
                        channel_name,
                        guild_id: guild_id_str,
                        message_id: None, // slash commands don't have a message to react to
                    };

                    if let Err(e) = self.msg_tx.send(msg_ctx).await {
                        error!(error = %e, "failed to send slash command to router");
                    }
                }
            }
            Interaction::Component(comp) => {
                let custom_id = comp.data.custom_id.clone();

                // Social inbox button: social:{action}:{inbox_id}
                if let Some(rest) = custom_id.strip_prefix("social:") {
                    let parts: Vec<&str> = rest.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        if let Ok(inbox_id) = parts[1].parse::<i64>() {
                            let action = parts[0];
                            // ai_reply_hint opens a modal to collect the hint text.
                            if action == "ai_reply_hint" {
                                use serenity::all::{
                                    CreateInputText, CreateModal, InputTextStyle,
                                };
                                let modal_id = format!("social:ai_reply_hint_submit:{}", inbox_id);
                                let input = CreateInputText::new(InputTextStyle::Paragraph, "回覆建議", "hint")
                                    .placeholder("請輸入 AI 回覆的方向或建議…")
                                    .required(true)
                                    .max_length(500);
                                let modal = CreateModal::new(modal_id, "建議 AI 回覆")
                                    .components(vec![serenity::all::CreateActionRow::InputText(input)]);
                                let _ = comp.create_response(
                                    &ctx.http,
                                    CreateInteractionResponse::Modal(modal),
                                ).await;
                                return;
                            }
                            let _ = self.social_action_tx.send((inbox_id, action.to_string(), None));
                        }
                    }
                    // Acknowledge without modifying the message
                    let _ = comp.create_response(
                        &ctx.http,
                        CreateInteractionResponse::Acknowledge,
                    ).await;
                    return;
                }

                let (request_id, approved) =
                    if let Some(rid) = custom_id.strip_prefix("approve:") {
                        (rid.to_string(), true)
                    } else if let Some(rid) = custom_id.strip_prefix("deny:") {
                        (rid.to_string(), false)
                    } else {
                        return;
                    };

                // Send decision to gateway
                let _ = self.approval_tx.send((request_id.clone(), approved));

                // Acknowledge the interaction (required by Discord within 3s)
                let label = if approved { "Approved" } else { "Denied" };
                let emoji = if approved { "✅" } else { "❌" };
                let color = if approved { 0x00FF00 } else { 0xFF0000 };

                let mut response_msg =
                    CreateInteractionResponseMessage::new().components(vec![]);
                if let Some(original_embed) = comp.message.embeds.first() {
                    let mut new_embed = CreateEmbed::new()
                        .title(format!("{} Tool Approval — {}", emoji, label))
                        .color(color);
                    for field in &original_embed.fields {
                        new_embed =
                            new_embed.field(&field.name, &field.value, field.inline);
                    }
                    response_msg = response_msg.embed(new_embed);
                }

                let response = CreateInteractionResponse::UpdateMessage(response_msg);
                if let Err(e) = comp.create_response(&ctx.http, response).await {
                    error!(error = %e, "failed to update approval message");
                }
            }
            Interaction::Modal(modal) => {
                // ai_reply_hint modal submission: social:ai_reply_hint_submit:{inbox_id}
                if let Some(rest) = modal.data.custom_id.strip_prefix("social:ai_reply_hint_submit:") {
                    if let Ok(inbox_id) = rest.parse::<i64>() {
                        let hint = modal.data.components.iter()
                            .flat_map(|row| row.components.iter())
                            .find_map(|c| {
                                if let serenity::all::ActionRowComponent::InputText(t) = c {
                                    if t.custom_id == "hint" {
                                        return t.value.clone();
                                    }
                                }
                                None
                            });
                        let _ = self.social_action_tx.send((inbox_id, "ai_reply".to_string(), hint));
                    }
                    let _ = modal.create_response(&ctx.http, CreateInteractionResponse::Acknowledge).await;
                }
            }
            _ => {}
        }
    }
}

/// Resolve a human-readable channel name from the serenity cache.
fn resolve_channel_name(
    cache: &serenity::cache::Cache,
    channel_id: ChannelId,
    guild_id: Option<GuildId>,
    is_dm: bool,
    sender_name: &str,
) -> Option<String> {
    if is_dm {
        Some(format!("dm.{}", sender_name))
    } else if let Some(guild_id) = guild_id {
        cache.guild(guild_id).and_then(|guild| {
            // Check guild.channels first, then guild.threads (threads are separate)
            guild
                .channels
                .get(&channel_id)
                .map(|ch| ch.name.clone())
                .or_else(|| {
                    guild
                        .threads
                        .iter()
                        .find(|t| t.id == channel_id)
                        .map(|t| t.name.clone())
                })
        })
    } else {
        None
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    async fn start(&self, msg_tx: mpsc::Sender<MsgContext>) -> Result<()> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILDS;

        let handler = Handler {
            msg_tx,
            filter: self.filter.clone(),
            approval_tx: self.approval_tx.clone(),
            social_action_tx: self.social_action_tx.clone(),
        };

        let mut client = Client::builder(&self.token, intents)
            .event_handler(handler)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to create client: {}", e)))?;

        // Store http and cache references
        {
            let mut http = self.http.write().await;
            *http = Some(client.http.clone());
        }
        {
            let mut cache = self.cache.write().await;
            *cache = Some(client.cache.clone());
        }

        // Start the client (blocks)
        client
            .start()
            .await
            .map_err(|e| CatClawError::Discord(format!("client error: {}", e)))?;

        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;

        // If thread_id is set, send to the thread channel instead.
        // In Discord, threads have their own channel ID — thread_id IS the channel to post in.
        let target_id = msg
            .thread_id
            .as_deref()
            .unwrap_or(&msg.channel_id)
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;

        let channel = ChannelId::new(target_id);
        let builder = CreateMessage::new().content(&msg.text);

        channel
            .send_message(http, builder)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to send message: {}", e)))?;

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
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;

        // In Discord, thread_id IS the channel to post in
        let target = thread_id.unwrap_or(channel_id);
        let ch_id = target
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;

        let input_str = serde_json::to_string_pretty(tool_input)
            .unwrap_or_else(|_| tool_input.to_string());
        // Truncate input preview to 1024 chars (Discord embed field limit)
        let input_preview = if input_str.len() > 1000 {
            format!("{}…", &input_str[..1000])
        } else {
            input_str
        };

        let embed = CreateEmbed::new()
            .title("🔒 Tool Approval Required")
            .field("Tool", format!("`{}`", tool_name), true)
            .field("Input", format!("```json\n{}\n```", input_preview), false)
            .color(0xFFA500); // orange

        let approve_btn = CreateButton::new(format!("approve:{}", request_id))
            .label("✅ Approve")
            .style(ButtonStyle::Success);
        let deny_btn = CreateButton::new(format!("deny:{}", request_id))
            .label("❌ Deny")
            .style(ButtonStyle::Danger);

        let builder = CreateMessage::new()
            .embed(embed)
            .components(vec![CreateActionRow::Buttons(vec![approve_btn, deny_btn])]);

        ChannelId::new(ch_id)
            .send_message(http, builder)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to send approval: {}", e)))?;

        Ok(())
    }

    async fn start_typing(&self, channel_id: &str, _peer_id: &str) -> Result<TypingGuard> {
        let http = self.http.read().await;
        let http = match http.as_ref() {
            Some(h) => h.clone(),
            None => return Ok(TypingGuard::noop()),
        };

        let channel_id = channel_id
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;

        let channel = ChannelId::new(channel_id);

        // Send typing indicator in a loop until cancelled
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            loop {
                let _ = channel.broadcast_typing(&http).await;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {},
                    _ = &mut cancel_rx => break,
                }
            }
        });

        Ok(TypingGuard::new(cancel_tx))
    }

    async fn create_thread(&self, channel_id: &str, title: &str) -> Result<String> {
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;

        let channel_id = channel_id
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;

        let channel = ChannelId::new(channel_id);
        let builder = CreateThread::new(title);

        let thread = channel
            .create_thread(http, builder)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to create thread: {}", e)))?;

        Ok(thread.id.get().to_string())
    }

    fn name(&self) -> &str {
        "discord"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            threading: true,
            typing_indicator: true,
            message_editing: true,
            max_message_length: 2000,
            attachments: true,
            streaming: false,
        }
    }

    async fn execute(&self, action: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;

        match action {
            // ── Messages ──────────────────────────────────────────────
            "get_messages" => {
                let cid = parse_channel_id(&params)?;
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u8;
                let messages = cid
                    .messages(http, GetMessages::new().limit(limit))
                    .await
                    .map_err(|e| CatClawError::Discord(format!("get_messages: {}", e)))?;
                let result: Vec<serde_json::Value> = messages
                    .iter()
                    .map(|m| serde_json::json!({
                        "id": m.id.get().to_string(),
                        "author": m.author.name,
                        "author_id": m.author.id.get().to_string(),
                        "content": m.content,
                        "timestamp": m.timestamp.to_string(),
                        "pinned": m.pinned,
                    }))
                    .collect();
                Ok(serde_json::json!(result))
            }
            "send_message" => {
                let cid = parse_channel_id(&params)?;
                let text = require_str(&params, "text")?;
                let builder = CreateMessage::new().content(text);
                let msg = cid.send_message(http, builder).await
                    .map_err(|e| CatClawError::Discord(format!("send_message: {}", e)))?;
                Ok(serde_json::json!({"id": msg.id.get().to_string()}))
            }
            "edit_message" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                let text = require_str(&params, "text")?;
                let builder = EditMessage::new().content(text);
                cid.edit_message(http, mid, builder).await
                    .map_err(|e| CatClawError::Discord(format!("edit_message: {}", e)))?;
                Ok(serde_json::json!({"edited": true}))
            }
            "delete_message" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                cid.delete_message(http, mid).await
                    .map_err(|e| CatClawError::Discord(format!("delete_message: {}", e)))?;
                Ok(serde_json::json!({"deleted": true}))
            }

            // ── Reactions ─────────────────────────────────────────────
            "react" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                let emoji = require_str(&params, "emoji")?;
                let reaction = parse_reaction(emoji);
                cid.create_reaction(http, mid, reaction).await
                    .map_err(|e| CatClawError::Discord(format!("react: {}", e)))?;
                Ok(serde_json::json!({"reacted": true}))
            }
            "get_reactions" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                let emoji = require_str(&params, "emoji")?;
                let limit = params.get("limit").and_then(|v| v.as_u64()).map(|l| l as u8);
                let reaction = parse_reaction(emoji);
                let users = cid.reaction_users(http, mid, reaction, limit, None::<UserId>).await
                    .map_err(|e| CatClawError::Discord(format!("get_reactions: {}", e)))?;
                let result: Vec<serde_json::Value> = users.iter().map(|u| serde_json::json!({
                    "id": u.id.get().to_string(), "name": u.name
                })).collect();
                Ok(serde_json::json!(result))
            }
            "delete_reaction" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                let emoji = require_str(&params, "emoji")?;
                let user_id = params.get("user_id").and_then(|v|
                    v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64())
                ).map(UserId::new);
                let reaction = parse_reaction(emoji);
                cid.delete_reaction(http, mid, user_id, reaction).await
                    .map_err(|e| CatClawError::Discord(format!("delete_reaction: {}", e)))?;
                Ok(serde_json::json!({"deleted": true}))
            }

            // ── Pins ──────────────────────────────────────────────────
            "pin_message" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                cid.pin(http, mid).await
                    .map_err(|e| CatClawError::Discord(format!("pin_message: {}", e)))?;
                Ok(serde_json::json!({"pinned": true}))
            }
            "unpin_message" => {
                let cid = parse_channel_id(&params)?;
                let mid = parse_message_id(&params)?;
                cid.unpin(http, mid).await
                    .map_err(|e| CatClawError::Discord(format!("unpin_message: {}", e)))?;
                Ok(serde_json::json!({"unpinned": true}))
            }
            "list_pins" => {
                let cid = parse_channel_id(&params)?;
                let pins = cid.pins(http).await
                    .map_err(|e| CatClawError::Discord(format!("list_pins: {}", e)))?;
                let result: Vec<serde_json::Value> = pins.iter().map(|m| serde_json::json!({
                    "id": m.id.get().to_string(),
                    "author": m.author.name,
                    "content": m.content,
                    "timestamp": m.timestamp.to_string(),
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Threads ───────────────────────────────────────────────
            "create_thread" => {
                let cid = parse_channel_id(&params)?;
                let name = require_str(&params, "name")?;
                let builder = CreateThread::new(name);
                let thread = cid.create_thread(http, builder).await
                    .map_err(|e| CatClawError::Discord(format!("create_thread: {}", e)))?;
                Ok(serde_json::json!({
                    "id": thread.id.get().to_string(),
                    "name": thread.name,
                }))
            }
            "list_threads" => {
                let guild_id = parse_guild_id(&params)?;
                let threads = guild_id.get_active_threads(http).await
                    .map_err(|e| CatClawError::Discord(format!("list_threads: {}", e)))?;
                let result: Vec<serde_json::Value> = threads.threads.iter().map(|t| serde_json::json!({
                    "id": t.id.get().to_string(),
                    "name": t.name,
                    "parent_id": t.parent_id.map(|p| p.get().to_string()),
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Channels ──────────────────────────────────────────────
            "get_channels" => {
                let guild_id = parse_guild_id(&params)?;
                let channels = guild_id.channels(http).await
                    .map_err(|e| CatClawError::Discord(format!("get_channels: {}", e)))?;
                let mut result: Vec<serde_json::Value> = channels.values().map(|c| serde_json::json!({
                    "id": c.id.get().to_string(),
                    "name": c.name,
                    "kind": format!("{:?}", c.kind),
                    "position": c.position,
                    "parent_id": c.parent_id.map(|p| p.get().to_string()),
                    "topic": c.topic,
                    "nsfw": c.nsfw,
                })).collect();
                result.sort_by_key(|c| c.get("position").and_then(|p| p.as_i64()).unwrap_or(0));
                Ok(serde_json::json!(result))
            }
            "channel_info" => {
                let cid = parse_channel_id(&params)?;
                let ch = cid.to_channel(http).await
                    .map_err(|e| CatClawError::Discord(format!("channel_info: {}", e)))?;
                if let Some(gc) = ch.guild() {
                    Ok(serde_json::json!({
                        "id": cid.get().to_string(),
                        "name": gc.name,
                        "kind": format!("{:?}", gc.kind),
                        "topic": gc.topic,
                        "nsfw": gc.nsfw,
                        "position": gc.position,
                        "parent_id": gc.parent_id.map(|p| p.get().to_string()),
                    }))
                } else {
                    Ok(serde_json::json!({"id": cid.get().to_string(), "kind": "DM"}))
                }
            }
            "create_channel" => {
                let guild_id = parse_guild_id(&params)?;
                let name = require_str(&params, "name")?;
                let mut builder = CreateChannel::new(name);
                if let Some(topic) = params.get("topic").and_then(|v| v.as_str()) {
                    builder = builder.topic(topic);
                }
                if let Some(parent_id) = params.get("parent_id").and_then(|v| v.as_str()) {
                    if let Ok(pid) = parent_id.parse::<u64>() {
                        builder = builder.category(ChannelId::new(pid));
                    }
                }
                if params.get("nsfw").and_then(|v| v.as_bool()).unwrap_or(false) {
                    builder = builder.nsfw(true);
                }
                let channel = guild_id.create_channel(http, builder).await
                    .map_err(|e| CatClawError::Discord(format!("create_channel: {}", e)))?;
                Ok(serde_json::json!({
                    "id": channel.id.get().to_string(),
                    "name": channel.name,
                }))
            }
            "create_category" => {
                let guild_id = parse_guild_id(&params)?;
                let name = require_str(&params, "name")?;
                let builder = CreateChannel::new(name).kind(SerenityChannelType::Category);
                let category = guild_id.create_channel(http, builder).await
                    .map_err(|e| CatClawError::Discord(format!("create_category: {}", e)))?;
                Ok(serde_json::json!({
                    "id": category.id.get().to_string(),
                    "name": category.name,
                }))
            }
            "edit_channel" => {
                let cid = parse_channel_id(&params)?;
                let mut builder = EditChannel::new();
                if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
                    builder = builder.name(name);
                }
                if let Some(topic) = params.get("topic").and_then(|v| v.as_str()) {
                    builder = builder.topic(topic);
                }
                if let Some(nsfw) = params.get("nsfw").and_then(|v| v.as_bool()) {
                    builder = builder.nsfw(nsfw);
                }
                if let Some(parent_id) = params.get("parent_id").and_then(|v| v.as_str()) {
                    if let Ok(pid) = parent_id.parse::<u64>() {
                        builder = builder.category(ChannelId::new(pid));
                    }
                }
                cid.edit(http, builder).await
                    .map_err(|e| CatClawError::Discord(format!("edit_channel: {}", e)))?;
                Ok(serde_json::json!({"edited": true}))
            }
            "delete_channel" => {
                let cid = parse_channel_id(&params)?;
                cid.delete(http).await
                    .map_err(|e| CatClawError::Discord(format!("delete_channel: {}", e)))?;
                Ok(serde_json::json!({"deleted": true}))
            }

            // ── Permissions ───────────────────────────────────────────
            "edit_permissions" => {
                let cid = parse_channel_id(&params)?;
                let target_id = parse_u64(&params, "target_id")?;
                let target_type = params.get("target_type").and_then(|v| v.as_str()).unwrap_or("role");
                let allow = params.get("allow").and_then(|v| v.as_u64()).unwrap_or(0);
                let deny = params.get("deny").and_then(|v| v.as_u64()).unwrap_or(0);
                let kind = match target_type {
                    "member" | "user" => PermissionOverwriteType::Member(UserId::new(target_id)),
                    _ => PermissionOverwriteType::Role(RoleId::new(target_id)),
                };
                let overwrite = PermissionOverwrite {
                    allow: Permissions::from_bits_truncate(allow),
                    deny: Permissions::from_bits_truncate(deny),
                    kind,
                };
                cid.create_permission(http, overwrite).await
                    .map_err(|e| CatClawError::Discord(format!("edit_permissions: {}", e)))?;
                Ok(serde_json::json!({"updated": true}))
            }

            // ── Guild ─────────────────────────────────────────────────
            "get_guilds" => {
                let cache = self.cache.read().await;
                if let Some(cache) = cache.as_ref() {
                    let guilds: Vec<serde_json::Value> = cache.guilds().iter().map(|gid| {
                        let name = cache.guild(*gid).map(|g| g.name.clone()).unwrap_or_default();
                        serde_json::json!({"id": gid.get().to_string(), "name": name})
                    }).collect();
                    Ok(serde_json::json!(guilds))
                } else {
                    Ok(serde_json::json!([]))
                }
            }
            "get_guild_info" => {
                let guild_id = parse_guild_id(&params)?;
                let info = guild_id.to_partial_guild(http).await
                    .map_err(|e| CatClawError::Discord(format!("get_guild_info: {}", e)))?;
                Ok(serde_json::json!({
                    "id": info.id.get().to_string(),
                    "name": info.name,
                    "owner_id": info.owner_id.get().to_string(),
                    "member_count": info.approximate_member_count,
                    "icon_url": info.icon_url(),
                }))
            }

            // ── Members ───────────────────────────────────────────────
            "member_info" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let member = guild_id.member(http, UserId::new(user_id)).await
                    .map_err(|e| CatClawError::Discord(format!("member_info: {}", e)))?;
                let roles: Vec<String> = member.roles.iter().map(|r| r.get().to_string()).collect();
                Ok(serde_json::json!({
                    "user_id": member.user.id.get().to_string(),
                    "username": member.user.name,
                    "display_name": member.display_name().to_string(),
                    "nick": member.nick,
                    "joined_at": member.joined_at.map(|t| t.to_string()),
                    "roles": roles,
                    "bot": member.user.bot,
                }))
            }
            "search_members" => {
                let guild_id = parse_guild_id(&params)?;
                let query = require_str(&params, "query")?;
                let limit = params.get("limit").and_then(|v| v.as_u64());
                let members = guild_id.search_members(http, query, limit).await
                    .map_err(|e| CatClawError::Discord(format!("search_members: {}", e)))?;
                let result: Vec<serde_json::Value> = members.iter().map(|m| serde_json::json!({
                    "user_id": m.user.id.get().to_string(),
                    "username": m.user.name,
                    "display_name": m.display_name().to_string(),
                    "nick": m.nick,
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Roles ─────────────────────────────────────────────────
            "list_roles" => {
                let guild_id = parse_guild_id(&params)?;
                let roles = guild_id.roles(http).await
                    .map_err(|e| CatClawError::Discord(format!("list_roles: {}", e)))?;
                let mut result: Vec<serde_json::Value> = roles.values().map(|r| serde_json::json!({
                    "id": r.id.get().to_string(),
                    "name": r.name,
                    "color": r.colour.0,
                    "position": r.position,
                    "mentionable": r.mentionable,
                    "permissions": r.permissions.bits().to_string(),
                })).collect();
                result.sort_by_key(|r| r.get("position").and_then(|p| p.as_i64()).unwrap_or(0));
                Ok(serde_json::json!(result))
            }
            "add_role" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let role_id = parse_u64(&params, "role_id")?;
                let member = guild_id.member(http, UserId::new(user_id)).await
                    .map_err(|e| CatClawError::Discord(format!("add_role: get member: {}", e)))?;
                member.add_role(http, RoleId::new(role_id)).await
                    .map_err(|e| CatClawError::Discord(format!("add_role: {}", e)))?;
                Ok(serde_json::json!({"added": true}))
            }
            "remove_role" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let role_id = parse_u64(&params, "role_id")?;
                let member = guild_id.member(http, UserId::new(user_id)).await
                    .map_err(|e| CatClawError::Discord(format!("remove_role: get member: {}", e)))?;
                member.remove_role(http, RoleId::new(role_id)).await
                    .map_err(|e| CatClawError::Discord(format!("remove_role: {}", e)))?;
                Ok(serde_json::json!({"removed": true}))
            }

            // ── Emojis ────────────────────────────────────────────────
            "list_emojis" => {
                let guild_id = parse_guild_id(&params)?;
                let emojis = guild_id.emojis(http).await
                    .map_err(|e| CatClawError::Discord(format!("list_emojis: {}", e)))?;
                let result: Vec<serde_json::Value> = emojis.iter().map(|e| serde_json::json!({
                    "id": e.id.get().to_string(),
                    "name": e.name,
                    "animated": e.animated,
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Moderation ────────────────────────────────────────────
            "timeout" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let duration_secs = params.get("duration_secs").and_then(|v| v.as_u64()).unwrap_or(60);
                let mut member = guild_id.member(http, UserId::new(user_id)).await
                    .map_err(|e| CatClawError::Discord(format!("timeout: get member: {}", e)))?;
                let until = serenity::model::Timestamp::from_unix_timestamp(
                    chrono::Utc::now().timestamp() + duration_secs as i64
                ).map_err(|e| CatClawError::Discord(format!("timeout: invalid timestamp: {}", e)))?;
                member.disable_communication_until_datetime(http, until).await
                    .map_err(|e| CatClawError::Discord(format!("timeout: {}", e)))?;
                Ok(serde_json::json!({"timed_out": true, "until": until.to_string()}))
            }
            "kick" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let reason = params.get("reason").and_then(|v| v.as_str());
                if let Some(reason) = reason {
                    guild_id.kick_with_reason(http, UserId::new(user_id), reason).await
                } else {
                    guild_id.kick(http, UserId::new(user_id)).await
                }.map_err(|e| CatClawError::Discord(format!("kick: {}", e)))?;
                Ok(serde_json::json!({"kicked": true}))
            }
            "ban" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                let delete_message_days = params.get("delete_message_days").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                let reason = params.get("reason").and_then(|v| v.as_str());
                if let Some(reason) = reason {
                    guild_id.ban_with_reason(http, UserId::new(user_id), delete_message_days, reason).await
                } else {
                    guild_id.ban(http, UserId::new(user_id), delete_message_days).await
                }.map_err(|e| CatClawError::Discord(format!("ban: {}", e)))?;
                Ok(serde_json::json!({"banned": true}))
            }
            "unban" => {
                let guild_id = parse_guild_id(&params)?;
                let user_id = parse_u64(&params, "user_id")?;
                guild_id.unban(http, UserId::new(user_id)).await
                    .map_err(|e| CatClawError::Discord(format!("unban: {}", e)))?;
                Ok(serde_json::json!({"unbanned": true}))
            }

            // ── Scheduled Events ──────────────────────────────────────
            "list_events" => {
                let guild_id = parse_guild_id(&params)?;
                let events = guild_id.scheduled_events(http, false).await
                    .map_err(|e| CatClawError::Discord(format!("list_events: {}", e)))?;
                let result: Vec<serde_json::Value> = events.iter().map(|ev| serde_json::json!({
                    "id": ev.id.get().to_string(),
                    "name": ev.name,
                    "description": ev.description,
                    "start_time": ev.start_time.to_string(),
                    "end_time": ev.end_time.map(|t| t.to_string()),
                    "status": format!("{:?}", ev.status),
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── Stickers ──────────────────────────────────────────────
            "list_stickers" => {
                let guild_id = parse_guild_id(&params)?;
                let stickers = guild_id.stickers(http).await
                    .map_err(|e| CatClawError::Discord(format!("list_stickers: {}", e)))?;
                let result: Vec<serde_json::Value> = stickers.iter().map(|s| serde_json::json!({
                    "id": s.id.get().to_string(),
                    "name": s.name,
                    "description": s.description,
                })).collect();
                Ok(serde_json::json!(result))
            }

            // ── File Upload ────────────────────────────────────────────
            "upload_file" => {
                let cid = parse_channel_id(&params)?;
                let file_path = require_str(&params, "file_path")?;

                let path = std::path::Path::new(file_path);
                if !path.is_absolute() {
                    return Err(CatClawError::Discord("file_path must be absolute".into()));
                }
                let data = tokio::fs::read(path).await.map_err(|e| {
                    CatClawError::Discord(format!("failed to read '{}': {}", file_path, e))
                })?;

                let filename = params
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("file")
                    })
                    .to_string();

                let attachment = CreateAttachment::bytes(data, &filename);
                let mut msg_builder = CreateMessage::new().add_file(attachment);
                if let Some(text) = params.get("text").and_then(|v| v.as_str()) {
                    msg_builder = msg_builder.content(text);
                }

                let sent = cid
                    .send_message(http, msg_builder)
                    .await
                    .map_err(|e| CatClawError::Discord(format!("upload_file: {}", e)))?;

                Ok(serde_json::json!({
                    "message_id": sent.id.get().to_string(),
                    "ok": true,
                }))
            }

            _ => Err(CatClawError::Channel(format!(
                "discord action '{}' not supported",
                action
            ))),
        }
    }

    fn supported_actions(&self) -> Vec<ActionInfo> {
        discord_action_infos()
    }

    async fn send_social_card(
        &self,
        channel_id: &str,
        card: &crate::social::forward::ForwardCard,
    ) -> crate::error::Result<Option<String>> {
        use crate::social::forward::ForwardCardType;
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;
        let ch_id = channel_id
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;

        let color = match &card.card_type {
            ForwardCardType::Incoming => 0x5865F2u32,
            ForwardCardType::DraftReview => 0xFEE75Cu32,
            ForwardCardType::Resolved(_) => 0x57F287u32,
        };
        let description = if let ForwardCardType::Resolved(ref s) = card.card_type {
            format!("{}\n\n_{}_", card.text, s)
        } else {
            card.text.clone()
        };
        let mut embed = CreateEmbed::new()
            .title(&card.title)
            .description(description)
            .color(color)
            .field("From", format!("@{}", card.author), true)
            .footer(serenity::all::CreateEmbedFooter::new(format!("inbox_id: {}", card.inbox_id)));

        if let Some(ref url) = card.permalink {
            embed = embed.field("Post", url, false);
        }

        let buttons: Vec<CreateButton> = match &card.card_type {
            ForwardCardType::Incoming => vec![
                CreateButton::new(format!("social:ai_reply:{}", card.inbox_id))
                    .label("AI 回覆")
                    .style(ButtonStyle::Primary),
                CreateButton::new(format!("social:ai_reply_hint:{}", card.inbox_id))
                    .label("建議 AI 回覆")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("social:manual_reply:{}", card.inbox_id))
                    .label("手動回覆")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("social:ignore:{}", card.inbox_id))
                    .label("忽略")
                    .style(ButtonStyle::Danger),
            ],
            ForwardCardType::DraftReview => vec![
                CreateButton::new(format!("social:approve_draft:{}", card.inbox_id))
                    .label("核准發送")
                    .style(ButtonStyle::Success),
                CreateButton::new(format!("social:discard_draft:{}", card.inbox_id))
                    .label("捨棄")
                    .style(ButtonStyle::Danger),
            ],
            ForwardCardType::Resolved(_) => vec![],
        };

        let builder = if buttons.is_empty() {
            CreateMessage::new().embed(embed)
        } else {
            CreateMessage::new()
                .embed(embed)
                .components(vec![CreateActionRow::Buttons(buttons)])
        };

        let msg = ChannelId::new(ch_id)
            .send_message(http, builder)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to send social card: {}", e)))?;

        Ok(Some(msg.id.to_string()))
    }

    async fn update_social_card(
        &self,
        channel_id: &str,
        message_id: &str,
        card: &crate::social::forward::ForwardCard,
    ) -> crate::error::Result<()> {
        use crate::social::forward::ForwardCardType;
        let http = self.http.read().await;
        let http = http
            .as_ref()
            .ok_or_else(|| CatClawError::Discord("not connected".to_string()))?;
        let ch_id = channel_id
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid channel id".to_string()))?;
        let msg_id = message_id
            .parse::<u64>()
            .map_err(|_| CatClawError::Discord("invalid message id".to_string()))?;

        let color = match &card.card_type {
            ForwardCardType::Incoming => 0x5865F2u32,
            ForwardCardType::DraftReview => 0xFEE75Cu32,
            ForwardCardType::Resolved(_) => 0x57F287u32,
        };

        let description = if let ForwardCardType::Resolved(ref s) = card.card_type {
            format!("{}\n\n_{}_", card.text, s)
        } else {
            card.text.clone()
        };

        let mut embed = CreateEmbed::new()
            .title(&card.title)
            .description(description)
            .color(color)
            .field("From", format!("@{}", card.author), true)
            .footer(serenity::all::CreateEmbedFooter::new(format!("inbox_id: {}", card.inbox_id)));

        if let Some(ref url) = card.permalink {
            embed = embed.field("Post", url, false);
        }

        let buttons: Vec<CreateButton> = match &card.card_type {
            ForwardCardType::Incoming => vec![
                CreateButton::new(format!("social:ai_reply:{}", card.inbox_id))
                    .label("AI 回覆")
                    .style(ButtonStyle::Primary),
                CreateButton::new(format!("social:ai_reply_hint:{}", card.inbox_id))
                    .label("建議 AI 回覆")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("social:manual_reply:{}", card.inbox_id))
                    .label("手動回覆")
                    .style(ButtonStyle::Secondary),
                CreateButton::new(format!("social:ignore:{}", card.inbox_id))
                    .label("忽略")
                    .style(ButtonStyle::Danger),
            ],
            ForwardCardType::DraftReview => vec![
                CreateButton::new(format!("social:approve_draft:{}", card.inbox_id))
                    .label("核准發送")
                    .style(ButtonStyle::Success),
                CreateButton::new(format!("social:discard_draft:{}", card.inbox_id))
                    .label("捨棄")
                    .style(ButtonStyle::Danger),
            ],
            ForwardCardType::Resolved(_) => vec![],
        };

        let mut builder = EditMessage::new().embed(embed);
        if buttons.is_empty() {
            builder = builder.components(vec![]);
        } else {
            builder = builder.components(vec![CreateActionRow::Buttons(buttons)]);
        }

        ChannelId::new(ch_id)
            .edit_message(http, MessageId::new(msg_id), builder)
            .await
            .map_err(|e| CatClawError::Discord(format!("failed to update social card: {}", e)))?;

        Ok(())
    }

    async fn create_reaction_handle(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Option<super::reaction::ReactionHandle> {
        let http = self.http.read().await;
        let http = http.as_ref()?;
        let cid = channel_id.parse::<u64>().ok()?;
        let mid = message_id.parse::<u64>().ok()?;
        Some(super::reaction::spawn(
            http.clone(),
            ChannelId::new(cid),
            MessageId::new(mid),
        ))
    }
}

// ── Helper functions ──────────────────────────────────────────────────

fn parse_u64(params: &serde_json::Value, field: &str) -> Result<u64> {
    params
        .get(field)
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()))
        .ok_or_else(|| CatClawError::Discord(format!("missing or invalid '{}'", field)))
}

fn parse_channel_id(params: &serde_json::Value) -> Result<ChannelId> {
    Ok(ChannelId::new(parse_u64(params, "channel_id")?))
}

fn parse_message_id(params: &serde_json::Value) -> Result<MessageId> {
    Ok(MessageId::new(parse_u64(params, "message_id")?))
}

fn parse_guild_id(params: &serde_json::Value) -> Result<GuildId> {
    Ok(GuildId::new(parse_u64(params, "guild_id")?))
}

fn require_str<'a>(params: &'a serde_json::Value, field: &str) -> Result<&'a str> {
    params
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatClawError::Discord(format!("missing '{}'", field)))
}

/// Parse an emoji string into a ReactionType.
/// Supports unicode emoji (e.g. "👍") and custom emoji (e.g. "<:name:123>").
fn parse_reaction(emoji: &str) -> ReactionType {
    // Custom emoji format: <:name:id> or <a:name:id>
    if emoji.starts_with('<') && emoji.ends_with('>') {
        let inner = &emoji[1..emoji.len() - 1];
        let parts: Vec<&str> = inner.split(':').collect();
        if parts.len() == 3 {
            let animated = parts[0] == "a";
            let name = parts[1].to_string();
            if let Ok(id) = parts[2].parse::<u64>() {
                return ReactionType::Custom {
                    animated,
                    id: serenity::model::id::EmojiId::new(id),
                    name: Some(name),
                };
            }
        }
    }
    ReactionType::Unicode(emoji.to_string())
}

/// All Discord action schemas for MCP tools/list
fn discord_action_infos() -> Vec<ActionInfo> {
    let ch = serde_json::json!({"type": "string", "description": "Discord channel ID"});
    let gid = serde_json::json!({"type": "string", "description": "Discord guild/server ID"});
    let mid = serde_json::json!({"type": "string", "description": "Discord message ID"});
    let uid = serde_json::json!({"type": "string", "description": "Discord user ID"});
    let rid = serde_json::json!({"type": "string", "description": "Discord role ID"});

    vec![
        // Messages
        action("get_messages", "Read recent messages from a channel", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "limit": {"type": "integer", "description": "Max messages (default 50, max 100)"}},
            "required": ["channel_id"]
        })),
        action("send_message", "Send a message to a channel", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "text": {"type": "string", "description": "Message content"}},
            "required": ["channel_id", "text"]
        })),
        action("edit_message", "Edit an existing message (bot's own)", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "message_id": mid, "text": {"type": "string", "description": "New content"}},
            "required": ["channel_id", "message_id", "text"]
        })),
        action("delete_message", "Delete a message", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "message_id": mid},
            "required": ["channel_id", "message_id"]
        })),
        // Reactions
        action("react", "Add a reaction to a message", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "message_id": mid, "emoji": {"type": "string", "description": "Emoji (unicode or <:name:id>)"}},
            "required": ["channel_id", "message_id", "emoji"]
        })),
        action("get_reactions", "Get users who reacted with an emoji", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "message_id": mid, "emoji": {"type": "string"}, "limit": {"type": "integer"}},
            "required": ["channel_id", "message_id", "emoji"]
        })),
        action("delete_reaction", "Remove a reaction (bot's own or specific user)", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "message_id": mid, "emoji": {"type": "string"}, "user_id": uid},
            "required": ["channel_id", "message_id", "emoji"]
        })),
        // Pins
        action("pin_message", "Pin a message", serde_json::json!({
            "type": "object", "properties": {"channel_id": ch, "message_id": mid}, "required": ["channel_id", "message_id"]
        })),
        action("unpin_message", "Unpin a message", serde_json::json!({
            "type": "object", "properties": {"channel_id": ch, "message_id": mid}, "required": ["channel_id", "message_id"]
        })),
        action("list_pins", "List all pinned messages in a channel", serde_json::json!({
            "type": "object", "properties": {"channel_id": ch}, "required": ["channel_id"]
        })),
        // Threads
        action("create_thread", "Create a new thread in a channel", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "name": {"type": "string", "description": "Thread name"}},
            "required": ["channel_id", "name"]
        })),
        action("list_threads", "List active threads in a guild", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        // Channels
        action("get_channels", "List all channels in a guild (sorted by position)", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        action("channel_info", "Get detailed info about a channel", serde_json::json!({
            "type": "object", "properties": {"channel_id": ch}, "required": ["channel_id"]
        })),
        action("create_channel", "Create a new text channel", serde_json::json!({
            "type": "object",
            "properties": {
                "guild_id": gid, "name": {"type": "string"}, "topic": {"type": "string"},
                "parent_id": {"type": "string", "description": "Category ID"}, "nsfw": {"type": "boolean"}
            },
            "required": ["guild_id", "name"]
        })),
        action("create_category", "Create a channel category", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "name": {"type": "string"}}, "required": ["guild_id", "name"]
        })),
        action("edit_channel", "Edit channel properties (name, topic, nsfw, category)", serde_json::json!({
            "type": "object",
            "properties": {"channel_id": ch, "name": {"type": "string"}, "topic": {"type": "string"}, "nsfw": {"type": "boolean"}, "parent_id": {"type": "string"}},
            "required": ["channel_id"]
        })),
        action("delete_channel", "Delete a channel", serde_json::json!({
            "type": "object", "properties": {"channel_id": ch}, "required": ["channel_id"]
        })),
        // Permissions
        action("edit_permissions", "Set permission overwrites for a role or user on a channel", serde_json::json!({
            "type": "object",
            "properties": {
                "channel_id": ch,
                "target_id": {"type": "string", "description": "Role or user ID"},
                "target_type": {"type": "string", "enum": ["role", "member"], "description": "Target type (default: role)"},
                "allow": {"type": "integer", "description": "Allowed permission bits"},
                "deny": {"type": "integer", "description": "Denied permission bits"}
            },
            "required": ["channel_id", "target_id"]
        })),
        // Guild
        action("get_guilds", "List guilds the bot is in", serde_json::json!({
            "type": "object", "properties": {}
        })),
        action("get_guild_info", "Get guild details", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        // Members
        action("member_info", "Get detailed info about a guild member", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "user_id": uid}, "required": ["guild_id", "user_id"]
        })),
        action("search_members", "Search guild members by name", serde_json::json!({
            "type": "object",
            "properties": {"guild_id": gid, "query": {"type": "string"}, "limit": {"type": "integer"}},
            "required": ["guild_id", "query"]
        })),
        // Roles
        action("list_roles", "List all roles in a guild (sorted by position)", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        action("add_role", "Add a role to a member", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "user_id": uid, "role_id": rid}, "required": ["guild_id", "user_id", "role_id"]
        })),
        action("remove_role", "Remove a role from a member", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "user_id": uid, "role_id": rid}, "required": ["guild_id", "user_id", "role_id"]
        })),
        // Emojis
        action("list_emojis", "List all custom emojis in a guild", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        // Moderation
        action("timeout", "Timeout a member (disable communication)", serde_json::json!({
            "type": "object",
            "properties": {"guild_id": gid, "user_id": uid, "duration_secs": {"type": "integer", "description": "Timeout duration in seconds (default 60, max 28 days)"}},
            "required": ["guild_id", "user_id"]
        })),
        action("kick", "Kick a member from the guild", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "user_id": uid, "reason": {"type": "string"}}, "required": ["guild_id", "user_id"]
        })),
        action("ban", "Ban a user from the guild", serde_json::json!({
            "type": "object",
            "properties": {"guild_id": gid, "user_id": uid, "delete_message_days": {"type": "integer", "description": "Days of messages to delete (0-7)"}, "reason": {"type": "string"}},
            "required": ["guild_id", "user_id"]
        })),
        action("unban", "Unban a user", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid, "user_id": uid}, "required": ["guild_id", "user_id"]
        })),
        // Events
        action("list_events", "List scheduled events in a guild", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        // Stickers
        action("list_stickers", "List custom stickers in a guild", serde_json::json!({
            "type": "object", "properties": {"guild_id": gid}, "required": ["guild_id"]
        })),
        // File Upload
        action("upload_file", "Upload a local file to a Discord channel", serde_json::json!({
            "type": "object",
            "properties": {
                "channel_id": ch,
                "file_path": {"type": "string", "description": "Absolute path to the local file"},
                "filename": {"type": "string", "description": "Display filename (defaults to basename of file_path)"},
                "text": {"type": "string", "description": "Message text to send with the file"}
            },
            "required": ["channel_id", "file_path"]
        })),
    ]
}

fn action(name: &str, description: &str, params_schema: serde_json::Value) -> ActionInfo {
    ActionInfo {
        name: name.into(),
        description: description.into(),
        params_schema,
    }
}
