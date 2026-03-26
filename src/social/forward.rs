#![allow(dead_code)]

// Forward card builder and delivery for Social Inbox events.
//
// Sends "forward" and "draft review" cards to the configured admin_channel,
// with action buttons that map to: social:{action}:{inbox_id}
//
// Button custom_id format: "social:{action}:{inbox_id}"
//   - social:ai_reply:{id}
//   - social:manual_reply:{id}
//   - social:ignore:{id}
//   - social:approve_draft:{id}
//   - social:discard_draft:{id}

use crate::channel::ChannelAdapter;
use crate::config::parse_admin_channel;
use crate::error::{CatClawError, Result};
use crate::state::SocialInboxRow;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::warn;

// ── Forward card ──────────────────────────────────────────────────────────────

/// Build a forward card payload for the given inbox row.
/// Returns a JSON Value in a channel-neutral format that `send_forward_card` can translate.
pub fn build_forward_card(row: &SocialInboxRow) -> ForwardCard {
    let platform_label = match row.platform.as_str() {
        "instagram" => "Instagram",
        "threads" => "Threads",
        other => other,
    };
    let event_label = row.event_type.as_str();
    let author = row.author_name.as_deref().unwrap_or("unknown");
    let text = row.text.as_deref().unwrap_or("(no text)");
    let permalink = row
        .metadata
        .as_ref()
        .and_then(|m| {
            serde_json::from_str::<Value>(m).ok()
        })
        .and_then(|v| v.get("permalink").and_then(|p| p.as_str()).map(str::to_string));

    ForwardCard {
        inbox_id: row.id,
        title: format!("{} {}", platform_label, event_label),
        author: author.to_string(),
        text: text.to_string(),
        permalink,
        created_at: row.created_at.clone(),
        card_type: ForwardCardType::Incoming,
    }
}

/// Build a draft review card after LLM has produced a draft reply.
pub fn build_draft_card(row: &SocialInboxRow, draft: &str) -> ForwardCard {
    let platform_label = match row.platform.as_str() {
        "instagram" => "Instagram",
        "threads" => "Threads",
        other => other,
    };
    let author = row.author_name.as_deref().unwrap_or("unknown");
    let original_text = row.text.as_deref().unwrap_or("(no text)");

    ForwardCard {
        inbox_id: row.id,
        title: format!("{} Draft Reply", platform_label),
        author: author.to_string(),
        text: format!("Original ({}): {}\nDraft: {}", author, original_text, draft),
        permalink: None,
        created_at: row.created_at.clone(),
        card_type: ForwardCardType::DraftReview,
    }
}

#[derive(Debug, Clone)]
pub enum ForwardCardType {
    Incoming,
    DraftReview,
    /// Terminal state — show status text, remove all buttons.
    Resolved(String),
}

#[derive(Debug, Clone)]
pub struct ForwardCard {
    pub inbox_id: i64,
    pub title: String,
    pub author: String,
    pub text: String,
    pub permalink: Option<String>,
    pub created_at: String,
    pub card_type: ForwardCardType,
}

/// Build a resolved card (terminal state, no buttons) from an existing card.
pub fn build_resolved_card(card: &ForwardCard, status: &str) -> ForwardCard {
    ForwardCard {
        inbox_id: card.inbox_id,
        title: card.title.clone(),
        author: card.author.clone(),
        text: card.text.clone(),
        permalink: card.permalink.clone(),
        created_at: card.created_at.clone(),
        card_type: ForwardCardType::Resolved(status.to_string()),
    }
}

impl ForwardCard {
    /// Render as a Discord embed JSON blob.
    pub fn to_discord_payload(&self) -> Value {
        let color = match &self.card_type {
            ForwardCardType::Incoming => 0x5865F2u64,    // blurple
            ForwardCardType::DraftReview => 0xFEE75Cu64, // yellow
            ForwardCardType::Resolved(_) => 0x57F287u64, // green
        };
        let buttons: Vec<Value> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                discord_button(&format!("social:ai_reply:{}", self.inbox_id), "AI 回覆", 1),
                discord_button(&format!("social:ai_reply_hint:{}", self.inbox_id), "建議 AI 回覆", 2),
                discord_button(&format!("social:manual_reply:{}", self.inbox_id), "手動回覆", 2),
                discord_button(&format!("social:ignore:{}", self.inbox_id), "忽略", 4),
            ],
            ForwardCardType::DraftReview => vec![
                discord_button(&format!("social:approve_draft:{}", self.inbox_id), "核准發送", 3),
                discord_button(&format!("social:discard_draft:{}", self.inbox_id), "捨棄", 4),
            ],
            ForwardCardType::Resolved(_) => vec![],
        };
        let mut fields = vec![
            json!({"name": "From", "value": format!("@{}", self.author), "inline": true}),
        ];
        if let Some(ref url) = self.permalink {
            fields.push(json!({"name": "Post", "value": url, "inline": false}));
        }
        let ts = DateTime::parse_from_rfc3339(&self.created_at)
            .map(|d| d.with_timezone(&Utc).to_rfc3339())
            .unwrap_or_else(|_| self.created_at.clone());

        let description = if let ForwardCardType::Resolved(ref s) = self.card_type {
            format!("{}\n\n_{}_", self.text, s)
        } else {
            self.text.clone()
        };

        let components: Value = if buttons.is_empty() {
            json!([])
        } else {
            json!([{ "type": 1, "components": buttons }])
        };

        json!({
            "embeds": [{
                "title": self.title,
                "description": description,
                "color": color,
                "fields": fields,
                "footer": { "text": format!("inbox_id: {}", self.inbox_id) },
                "timestamp": ts
            }],
            "components": components
        })
    }

    /// Render as a Telegram message with inline keyboard.
    pub fn to_telegram_text_and_keyboard(&self) -> (String, Value) {
        let text = format!(
            "*{}*\nFrom: @{}\n\n{}{}",
            escape_markdown(&self.title),
            escape_markdown(&self.author),
            escape_markdown(&self.text),
            self.permalink
                .as_ref()
                .map(|u| format!("\n[Post]({})", u))
                .unwrap_or_default()
        );
        let keyboard: Vec<Vec<Value>> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                vec![
                    tg_button("AI 回覆", &format!("social:ai_reply:{}", self.inbox_id)),
                    tg_button("建議 AI 回覆", &format!("social:ai_reply_hint:{}", self.inbox_id)),
                ],
                vec![
                    tg_button("手動回覆", &format!("social:manual_reply:{}", self.inbox_id)),
                    tg_button("忽略", &format!("social:ignore:{}", self.inbox_id)),
                ],
            ],
            ForwardCardType::DraftReview => vec![vec![
                tg_button("核准發送", &format!("social:approve_draft:{}", self.inbox_id)),
                tg_button("捨棄", &format!("social:discard_draft:{}", self.inbox_id)),
            ]],
            ForwardCardType::Resolved(_) => vec![],
        };
        (text, json!({ "inline_keyboard": keyboard }))
    }

    /// Render as a Slack Block Kit message.
    pub fn to_slack_blocks(&self) -> Value {
        let header = json!({
            "type": "header",
            "text": { "type": "plain_text", "text": self.title }
        });
        let body = json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": format!("*From:* @{}\n{}", self.author, self.text) }
        });
        let actions: Vec<Value> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                slack_button(&format!("social:ai_reply:{}", self.inbox_id), "AI 回覆", "primary"),
                slack_button(&format!("social:ai_reply_hint:{}", self.inbox_id), "建議 AI 回覆", "default"),
                slack_button(&format!("social:manual_reply:{}", self.inbox_id), "手動回覆", "default"),
                slack_button(&format!("social:ignore:{}", self.inbox_id), "忽略", "danger"),
            ],
            ForwardCardType::DraftReview => vec![
                slack_button(&format!("social:approve_draft:{}", self.inbox_id), "核准發送", "primary"),
                slack_button(&format!("social:discard_draft:{}", self.inbox_id), "捨棄", "danger"),
            ],
            ForwardCardType::Resolved(s) => {
                return json!({
                    "blocks": [header, body, json!({
                        "type": "context",
                        "elements": [{ "type": "mrkdwn", "text": s }]
                    })]
                });
            }
        };
        json!({
            "blocks": [header, body, { "type": "actions", "elements": actions }]
        })
    }
}

// ── Delivery ──────────────────────────────────────────────────────────────────

/// Send a forward card to the admin_channel, using the matching adapter.
pub async fn send_forward_card(
    card: ForwardCard,
    admin_channel: &str,
    adapters: &[Arc<dyn ChannelAdapter>],
) -> Result<Option<String>> {
    let (platform, channel_id) = parse_admin_channel(admin_channel).ok_or_else(|| {
        CatClawError::Social(format!(
            "invalid admin_channel '{}' — use discord:channel:<id>|telegram:chat:<id>|slack:channel:<id>",
            admin_channel
        ))
    })?;
    for adapter in adapters {
        if adapter.platform_name() == platform {
            return adapter.send_social_card(&channel_id, &card).await;
        }
    }
    warn!(
        "no adapter found for platform '{}' (admin_channel = {})",
        platform, admin_channel
    );
    Ok(None)
}

/// Update an existing forward card in-place (e.g., after a button action).
/// `message_id` is the platform message ID stored in `forward_ref`.
pub async fn update_forward_card(
    card: ForwardCard,
    message_id: &str,
    admin_channel: &str,
    adapters: &[Arc<dyn ChannelAdapter>],
) {
    let (platform, channel_id) = match parse_admin_channel(admin_channel) {
        Some(pc) => pc,
        None => return,
    };
    for adapter in adapters {
        if adapter.platform_name() == platform {
            if let Err(e) = adapter.update_social_card(&channel_id, message_id, &card).await {
                warn!(error = %e, "update_forward_card: failed to edit card");
            }
            return;
        }
    }
}

// ── Button helpers ────────────────────────────────────────────────────────────

fn discord_button(custom_id: &str, label: &str, style: u8) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id
    })
}

fn tg_button(text: &str, callback_data: &str) -> Value {
    json!({ "text": text, "callback_data": callback_data })
}

fn slack_button(action_id: &str, text: &str, style: &str) -> Value {
    let mut b = json!({
        "type": "button",
        "text": { "type": "plain_text", "text": text },
        "action_id": action_id
    });
    if style != "default" {
        b["style"] = json!(style);
    }
    b
}

fn escape_markdown(s: &str) -> String {
    s.replace('_', "\\_")
        .replace('*', "\\*")
        .replace('[', "\\[")
        .replace(']', "\\]")
}
