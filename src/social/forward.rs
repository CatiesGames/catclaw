#![allow(dead_code)]

// Forward card builder and delivery for Social Inbox events.
//
// Sends "forward" and "draft review" cards to the configured admin_channel,
// with action buttons that map to: {button_prefix}:{action}:{card_id}
//
// Inbox button format: "social:{action}:{inbox_id}"
//   - social:ai_reply:{id}
//   - social:ai_reply_hint:{id}
//   - social:manual_reply:{id}
//   - social:ignore:{id}
//   - social:approve_draft:{id}
//   - social:discard_draft:{id}
//
// Draft button format: "social_draft:{action}:{draft_id}"
//   - social_draft:approve:{id}
//   - social_draft:discard:{id}

use crate::channel::ChannelAdapter;
use crate::config::parse_admin_channel;
use crate::error::{CatClawError, Result};
use crate::state::{SocialDraftRow, SocialInboxRow};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::warn;

// ── Forward card ──────────────────────────────────────────────────────────────

/// Build a forward card payload for the given inbox row.
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
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
        .and_then(|v| v.get("permalink").and_then(|p| p.as_str()).map(str::to_string));

    ForwardCard {
        card_id: row.id,
        button_prefix: "social".to_string(),
        title: format!("{} {}", platform_label, event_label),
        author: author.to_string(),
        text: text.to_string(),
        permalink,
        image_url: None,
        created_at: row.created_at.clone(),
        card_type: ForwardCardType::Incoming,
    }
}

/// Build a draft review card after LLM has produced a draft reply (inbox-based, legacy path).
pub fn build_draft_card(row: &SocialInboxRow, draft: &str) -> ForwardCard {
    let platform_label = match row.platform.as_str() {
        "instagram" => "Instagram",
        "threads" => "Threads",
        other => other,
    };
    let author = row.author_name.as_deref().unwrap_or("unknown");
    let original_text = row.text.as_deref().unwrap_or("(no text)");

    ForwardCard {
        card_id: row.id,
        button_prefix: "social".to_string(),
        title: format!("{} Draft Reply", platform_label),
        author: author.to_string(),
        text: format!("Original ({}): {}\nDraft: {}", author, original_text, draft),
        permalink: None,
        image_url: None,
        created_at: row.created_at.clone(),
        card_type: ForwardCardType::DraftReview,
    }
}

/// Build a draft review card from a `SocialDraftRow` (new draft system).
pub fn build_social_draft_card(draft: &SocialDraftRow) -> ForwardCard {
    let platform_label = match draft.platform.as_str() {
        "instagram" => "Instagram",
        "threads" => "Threads",
        other => other,
    };
    let author = draft.original_author.as_deref().unwrap_or("unknown");

    let (title, text) = match draft.draft_type.as_str() {
        "reply" => {
            let original = draft.original_text.as_deref().unwrap_or("(no text)");
            (
                format!("{} Draft Reply", platform_label),
                format!("Original (@{}): {}\nDraft: {}", author, original, draft.content),
            )
        }
        "dm" => {
            let recipient = draft.reply_to_id.as_deref().unwrap_or("unknown");
            (
                format!("{} Draft DM", platform_label),
                format!("To: {}\nDraft: {}", recipient, draft.content),
            )
        }
        _ => {
            // "post" or anything else
            (format!("{} Draft Post", platform_label), format!("Draft: {}", draft.content))
        }
    };

    ForwardCard {
        card_id: draft.id,
        button_prefix: "social_draft".to_string(),
        title,
        author: author.to_string(),
        text,
        permalink: None,
        image_url: draft.media_url.clone(),
        created_at: draft.created_at.clone(),
        card_type: ForwardCardType::DraftReview,
    }
}

#[derive(Debug, Clone)]
pub enum ForwardCardType {
    Incoming,
    DraftReview,
    /// Failed — show error text with retry + discard buttons.
    Failed(String),
    /// Terminal state — show status text, remove all buttons.
    Resolved(String),
}

#[derive(Debug, Clone)]
pub struct ForwardCard {
    pub card_id: i64,
    /// Button ID prefix: "social" for inbox cards, "social_draft" for draft cards.
    pub button_prefix: String,
    pub title: String,
    pub author: String,
    pub text: String,
    pub permalink: Option<String>,
    /// Image URL to display in the card (e.g., draft post media).
    pub image_url: Option<String>,
    pub created_at: String,
    pub card_type: ForwardCardType,
}

/// Build a failed card (retry + discard buttons) from an existing card.
pub fn build_failed_card(card: &ForwardCard, status: &str) -> ForwardCard {
    ForwardCard {
        card_id: card.card_id,
        button_prefix: card.button_prefix.clone(),
        title: card.title.clone(),
        author: card.author.clone(),
        text: card.text.clone(),
        permalink: card.permalink.clone(),
        image_url: card.image_url.clone(),
        created_at: card.created_at.clone(),
        card_type: ForwardCardType::Failed(status.to_string()),
    }
}

/// Build a resolved card (terminal state, no buttons) from an existing card.
pub fn build_resolved_card(card: &ForwardCard, status: &str) -> ForwardCard {
    ForwardCard {
        card_id: card.card_id,
        button_prefix: card.button_prefix.clone(),
        title: card.title.clone(),
        author: card.author.clone(),
        text: card.text.clone(),
        permalink: card.permalink.clone(),
        image_url: card.image_url.clone(),
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
            ForwardCardType::Failed(_) => 0xED4245u64,   // red
            ForwardCardType::Resolved(_) => 0x57F287u64, // green
        };
        let pfx = &self.button_prefix;
        let id = self.card_id;
        let buttons: Vec<Value> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                discord_button(&format!("{pfx}:ai_reply:{id}"), "AI 回覆", 1),
                discord_button(&format!("{pfx}:ai_reply_hint:{id}"), "建議 AI 回覆", 2),
                discord_button(&format!("{pfx}:manual_reply:{id}"), "手動回覆", 2),
                discord_button(&format!("{pfx}:ignore:{id}"), "忽略", 4),
            ],
            ForwardCardType::DraftReview | ForwardCardType::Failed(_) => vec![
                discord_button(&format!("{pfx}:approve:{id}"), "重試發送", 3),
                discord_button(&format!("{pfx}:discard:{id}"), "捨棄", 4),
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

        let description = match &self.card_type {
            ForwardCardType::Resolved(ref s) => format!("{}\n\n_{}_", self.text, s),
            ForwardCardType::Failed(ref s) => format!("{}\n\n⚠️ _{}_", self.text, s),
            _ => self.text.clone(),
        };

        let components: Value = if buttons.is_empty() {
            json!([])
        } else {
            json!([{ "type": 1, "components": buttons }])
        };

        let mut embed = json!({
            "title": self.title,
            "description": description,
            "color": color,
            "fields": fields,
            "footer": { "text": format!("id: {}", self.card_id) },
            "timestamp": ts
        });
        if let Some(ref url) = self.image_url {
            embed["image"] = json!({"url": url});
        }

        json!({
            "embeds": [embed],
            "components": components
        })
    }

    /// Render as a Telegram message with inline keyboard.
    pub fn to_telegram_text_and_keyboard(&self) -> (String, Value) {
        let status_line = match &self.card_type {
            ForwardCardType::Failed(ref s) => format!("\n\n⚠️ _{}_", escape_markdown(s)),
            _ => String::new(),
        };
        let text = format!(
            "*{}*\nFrom: @{}\n\n{}{}{}{}",
            escape_markdown(&self.title),
            escape_markdown(&self.author),
            escape_markdown(&self.text),
            self.permalink
                .as_ref()
                .map(|u| format!("\n[Post]({})", u))
                .unwrap_or_default(),
            self.image_url
                .as_ref()
                .map(|u| format!("\n[Media]({})", escape_markdown(u)))
                .unwrap_or_default(),
            status_line
        );
        let pfx = &self.button_prefix;
        let id = self.card_id;
        let keyboard: Vec<Vec<Value>> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                vec![
                    tg_button("AI 回覆", &format!("{pfx}:ai_reply:{id}")),
                    tg_button("建議 AI 回覆", &format!("{pfx}:ai_reply_hint:{id}")),
                ],
                vec![
                    tg_button("手動回覆", &format!("{pfx}:manual_reply:{id}")),
                    tg_button("忽略", &format!("{pfx}:ignore:{id}")),
                ],
            ],
            ForwardCardType::DraftReview | ForwardCardType::Failed(_) => vec![vec![
                tg_button("重試發送", &format!("{pfx}:approve:{id}")),
                tg_button("捨棄", &format!("{pfx}:discard:{id}")),
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
        let mut blocks = vec![header, body];
        if let Some(ref url) = self.image_url {
            blocks.push(json!({
                "type": "image",
                "image_url": url,
                "alt_text": "Draft media"
            }));
        }
        let pfx = &self.button_prefix;
        let id = self.card_id;
        let actions: Vec<Value> = match &self.card_type {
            ForwardCardType::Incoming => vec![
                slack_button(&format!("{pfx}:ai_reply:{id}"), "AI 回覆", "primary"),
                slack_button(&format!("{pfx}:ai_reply_hint:{id}"), "建議 AI 回覆", "default"),
                slack_button(&format!("{pfx}:manual_reply:{id}"), "手動回覆", "default"),
                slack_button(&format!("{pfx}:ignore:{id}"), "忽略", "danger"),
            ],
            ForwardCardType::DraftReview => vec![
                slack_button(&format!("{pfx}:approve:{id}"), "核准發送", "primary"),
                slack_button(&format!("{pfx}:discard:{id}"), "捨棄", "danger"),
            ],
            ForwardCardType::Failed(s) => {
                blocks.push(json!({
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": format!("⚠️ {}", s) }]
                }));
                vec![
                    slack_button(&format!("{pfx}:approve:{id}"), "重試發送", "primary"),
                    slack_button(&format!("{pfx}:discard:{id}"), "捨棄", "danger"),
                ]
            }
            ForwardCardType::Resolved(s) => {
                blocks.push(json!({
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": s }]
                }));
                return json!({ "blocks": blocks });
            }
        };
        blocks.push(json!({ "type": "actions", "elements": actions }));
        json!({ "blocks": blocks })
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
