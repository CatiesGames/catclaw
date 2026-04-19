//! Outbound pipeline: agent → contacts_reply → draft → forward (mirror) → approval → send.
//!
//! 對應 plan §2/§3:
//! - agent 不准繞過 pipeline(channel adapter raw send 不開放給 agent)
//! - forward_channel 鏡射 work card 至管理頻道(預覽 + 操作元件)
//! - approval_required 控制是否等管理者按鈕
//! - ai_paused 拒絕新草稿
//! - 失敗自癒(沿用 social ensure_inbox_card_restored 模式)

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::channel::{ChannelAdapter, ChannelType, OutboundMessage};
use crate::contacts::{Contact, ContactDraft, ContactPayload};
use crate::error::{CatClawError, Result};
use crate::state::StateDb;

/// Parsed forward_channel reference.
/// Format: "{platform}:{channel_id}" or "{platform}:{guild_id}/{channel_id}"
#[derive(Debug, Clone)]
pub struct ForwardTarget {
    pub platform: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
}

impl ForwardTarget {
    pub fn parse(s: &str) -> Option<Self> {
        let (platform, rest) = s.split_once(':')?;
        if rest.contains('/') {
            let (guild, ch) = rest.split_once('/')?;
            Some(ForwardTarget {
                platform: platform.to_string(),
                channel_id: ch.to_string(),
                guild_id: Some(guild.to_string()),
            })
        } else {
            Some(ForwardTarget {
                platform: platform.to_string(),
                channel_id: rest.to_string(),
                guild_id: None,
            })
        }
    }
}

/// Card representation for the management channel work-card.
/// Adapters render this with their native UI (Discord buttons, Slack Block Kit, etc.).
#[derive(Debug, Clone)]
pub struct ContactWorkCard {
    pub draft_id: i64,
    pub contact_id: String,
    pub contact_name: String,
    pub role: String,
    pub via_platform: Option<String>,
    pub payload_preview: String,
    /// "pending" | "awaiting_approval" | "sent" | "ignored" | "revising" | "failed" | "publishing" | "ai_paused"
    pub status_label: String,
    /// When true, render approve/edit/discard/revise/pause buttons. Hidden when status is terminal.
    pub show_actions: bool,
}

impl ContactWorkCard {
    pub fn from_draft(draft: &ContactDraft, contact: &Contact, label: &str, show_actions: bool) -> Self {
        let payload: serde_json::Value = draft.payload.clone();
        let preview = match serde_json::from_value::<ContactPayload>(payload.clone()) {
            Ok(p) => p.preview(),
            Err(_) => payload.to_string(),
        };
        ContactWorkCard {
            draft_id: draft.id,
            contact_id: contact.id.clone(),
            contact_name: contact.display_name.clone(),
            role: contact.role.as_str().to_string(),
            via_platform: draft.via_platform.clone(),
            payload_preview: preview,
            status_label: label.to_string(),
            show_actions,
        }
    }

    /// Plain-text rendering (used as fallback when an adapter doesn't implement
    /// rich rendering and as the body of forward mirroring of inbound messages).
    pub fn to_text(&self) -> String {
        let actions_hint = if self.show_actions {
            "\n\n回覆 'approve <id>' / 'discard <id>' / 'pause <contact_id>' 操作。"
        } else {
            ""
        };
        format!(
            "📨 Contact: {} (role={}, via={})\nDraft #{} [{}]\n────\n{}{}",
            self.contact_name,
            self.role,
            self.via_platform.as_deref().unwrap_or("auto"),
            self.draft_id,
            self.status_label,
            self.payload_preview,
            actions_hint
        )
    }
}

/// Inbound mirror payload (for showing client → agent traffic to the management channel).
#[derive(Debug, Clone)]
pub struct InboundMirror {
    pub contact_id: String,
    pub contact_name: String,
    pub from_platform: String,
    pub text: String,
    /// Attachment summaries (filename + url, rendered by adapter as needed).
    pub attachments: Vec<String>,
    pub ai_paused: bool,
}

impl InboundMirror {
    pub fn to_text(&self) -> String {
        let pause_hint = if self.ai_paused {
            "\n⏸ AI paused — manual reply required (just type in this channel)."
        } else {
            ""
        };
        let att = if self.attachments.is_empty() {
            String::new()
        } else {
            format!("\n📎 {}", self.attachments.join(", "))
        };
        format!(
            "📥 {} (via {}): {}{}{}",
            self.contact_name,
            self.from_platform,
            self.text,
            att,
            pause_hint
        )
    }
}

/// Outbound result for a single contact reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundResult {
    pub draft_id: i64,
    /// "queued_for_approval" | "sent" | "rejected_paused" | "rejected_no_channel"
    pub status: String,
    pub message: String,
}

/// Outbound pipeline entry point — invoked by `contacts_reply` MCP tool.
///
/// Steps:
/// 1. Refuse if ai_paused (agent shouldn't be sending while paused).
/// 2. Persist draft (status=pending).
/// 3. Mirror work card to forward_channel (if set).
/// 4. If approval not required → send immediately and update card.
///    Otherwise → mark awaiting_approval (button handler will trigger send later).
pub async fn submit_reply(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    contact_id: &str,
    payload: serde_json::Value,
    via: Option<String>,
) -> Result<OutboundResult> {
    let contact = db
        .get_contact(contact_id)?
        .ok_or_else(|| CatClawError::Other(format!("contact '{}' not found", contact_id)))?;

    if contact.ai_paused {
        return Ok(OutboundResult {
            draft_id: 0,
            status: "rejected_paused".into(),
            message: format!(
                "Contact {} has AI paused. Use contacts_ai_resume to re-enable, or have the human reply manually in the forward channel.",
                contact.display_name
            ),
        });
    }

    // 1. Persist draft.
    let mut draft = ContactDraft::new(&contact.id, &contact.agent_id, payload);
    draft.via_platform = via;
    let draft_id = db.insert_contact_draft(&draft)?;
    draft.id = draft_id;

    // 2. Mirror to forward_channel (best-effort).
    if let Some(ref fc) = contact.forward_channel {
        if let Some(target) = ForwardTarget::parse(fc) {
            let initial_label = if contact.approval_required {
                "awaiting approval"
            } else {
                "auto-sending..."
            };
            let card = ContactWorkCard::from_draft(&draft, &contact, initial_label, true);
            match send_work_card(adapters, &target, &card).await {
                Ok(Some(msg_ref)) => {
                    let _ = db.update_contact_draft_forward_ref(draft_id, &msg_ref);
                    draft.forward_ref = Some(msg_ref);
                }
                Ok(None) => {
                    info!(
                        contact_id,
                        "forward adapter returned no message ref (text fallback)"
                    );
                }
                Err(e) => {
                    warn!(error = %e, contact_id, "failed to mirror work card");
                }
            }
        } else {
            warn!(
                contact_id,
                forward_channel = %fc,
                "invalid forward_channel format (expected 'platform:[guild/]channel')"
            );
        }
    }

    // 3. Approval branch.
    if contact.approval_required {
        db.update_contact_draft_status(draft_id, "awaiting_approval")?;
        Ok(OutboundResult {
            draft_id,
            status: "queued_for_approval".into(),
            message: format!(
                "Draft #{} queued for approval. Notify in forward channel: {}",
                draft_id,
                contact.forward_channel.as_deref().unwrap_or("(none)")
            ),
        })
    } else {
        // Auto-send.
        match send_to_contact(db, adapters, &contact, &draft).await {
            Ok(()) => {
                db.update_contact_draft_sent(draft_id)?;
                refresh_card(db, adapters, &contact, draft_id, "sent", false).await;
                Ok(OutboundResult {
                    draft_id,
                    status: "sent".into(),
                    message: format!("Sent to {} immediately (no approval).", contact.display_name),
                })
            }
            Err(e) => {
                let err = format!("{}", e);
                db.update_contact_draft_failed(draft_id, &err)?;
                refresh_card(db, adapters, &contact, draft_id, "failed", false).await;
                Err(e)
            }
        }
    }
}

/// Approve a queued draft and send it.
pub async fn approve_draft(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    draft_id: i64,
) -> Result<OutboundResult> {
    let draft = db
        .get_contact_draft(draft_id)?
        .ok_or_else(|| CatClawError::Other(format!("draft #{} not found", draft_id)))?;
    if draft.status == "sent" {
        return Ok(OutboundResult {
            draft_id,
            status: "sent".into(),
            message: "already sent".into(),
        });
    }
    let contact = db
        .get_contact(&draft.contact_id)?
        .ok_or_else(|| CatClawError::Other(format!("contact '{}' not found", draft.contact_id)))?;

    refresh_card(db, adapters, &contact, draft_id, "publishing", false).await;

    match send_to_contact(db, adapters, &contact, &draft).await {
        Ok(()) => {
            db.update_contact_draft_sent(draft_id)?;
            refresh_card(db, adapters, &contact, draft_id, "sent", false).await;
            Ok(OutboundResult {
                draft_id,
                status: "sent".into(),
                message: format!("Approved and sent to {}.", contact.display_name),
            })
        }
        Err(e) => {
            let err = format!("{}", e);
            db.update_contact_draft_failed(draft_id, &err)?;
            refresh_card(db, adapters, &contact, draft_id, "failed", true).await;
            Err(e)
        }
    }
}

/// Discard a queued draft.
pub async fn discard_draft(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    draft_id: i64,
) -> Result<OutboundResult> {
    let draft = db
        .get_contact_draft(draft_id)?
        .ok_or_else(|| CatClawError::Other(format!("draft #{} not found", draft_id)))?;
    if let Ok(Some(c)) = db.get_contact(&draft.contact_id) {
        refresh_card(db, adapters, &c, draft_id, "discarded", false).await;
    }
    db.update_contact_draft_status(draft_id, "ignored")?;
    Ok(OutboundResult {
        draft_id,
        status: "discarded".into(),
        message: "draft discarded".into(),
    })
}

/// Request agent to revise a draft (send it back through the agent's session).
/// The actual session re-injection is performed by the WS handler;
/// here we just persist the revision_note and update the card to a holding state.
pub async fn request_revision(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    draft_id: i64,
    note: &str,
) -> Result<OutboundResult> {
    let draft = db
        .get_contact_draft(draft_id)?
        .ok_or_else(|| CatClawError::Other(format!("draft #{} not found", draft_id)))?;
    db.update_contact_draft_revision_note(draft_id, note)?;
    if let Ok(Some(c)) = db.get_contact(&draft.contact_id) {
        refresh_card(db, adapters, &c, draft_id, "revising", false).await;
    }
    Ok(OutboundResult {
        draft_id,
        status: "revising".into(),
        message: format!("Revision requested: {}", note),
    })
}

/// Edit a draft's payload (manual edit by admin) and approve in one step.
pub async fn edit_and_approve(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    draft_id: i64,
    new_payload: serde_json::Value,
) -> Result<OutboundResult> {
    db.update_contact_draft_payload(draft_id, &new_payload)?;
    approve_draft(db, adapters, draft_id).await
}

/// Send a `ContactPayload` to a contact via the appropriate adapter.
/// Selection: explicit via_platform > primary channel > most recently active channel.
async fn send_to_contact(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    contact: &Contact,
    draft: &ContactDraft,
) -> Result<()> {
    let channels = db.list_contact_channels(&contact.id)?;
    if channels.is_empty() {
        return Err(CatClawError::Other(format!(
            "contact {} has no bound channels",
            contact.display_name
        )));
    }

    let chosen = if let Some(ref via) = draft.via_platform {
        channels.iter().find(|c| &c.platform == via)
    } else {
        // list_contact_channels is ORDER BY is_primary DESC, last_active_at DESC NULLS LAST
        channels.first()
    };
    let chosen = chosen.ok_or_else(|| {
        CatClawError::Other(format!(
            "no channel matched via='{:?}' for contact {}",
            draft.via_platform, contact.display_name
        ))
    })?;

    let adapter = adapters.get(&chosen.platform).ok_or_else(|| {
        CatClawError::Other(format!(
            "platform '{}' has no running adapter (is it configured?)",
            chosen.platform
        ))
    })?;

    let payload: ContactPayload = serde_json::from_value(draft.payload.clone())
        .map_err(|e| CatClawError::Other(format!("invalid payload JSON: {e}")))?;

    let text = match payload {
        ContactPayload::Text { text } => text,
        ContactPayload::Image { url, caption } => match caption {
            Some(c) => format!("{}\n{}", c, url),
            None => url,
        },
        ContactPayload::Flex { contents } => {
            // Generic adapters fall back to text; LINE adapter may override via execute().
            // For now, send a JSON dump (LINE adapter Flex rendering is Stage 5).
            serde_json::to_string(&contents).unwrap_or_default()
        }
    };

    adapter
        .send(OutboundMessage {
            channel_type: ChannelType::Tui, // placeholder — concrete adapters ignore (see send_approval)
            channel_id: chosen.platform_user_id.clone(),
            peer_id: chosen.platform_user_id.clone(),
            text,
            thread_id: None,
            reply_to_message_id: None,
        })
        .await
}

/// Mirror an inbound message from a contact to the forward channel.
/// Best-effort — failure to mirror does not block routing to the agent.
pub async fn mirror_inbound(
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    contact: &Contact,
    from_platform: &str,
    text: &str,
    attachments: Vec<String>,
) {
    let Some(ref fc) = contact.forward_channel else { return };
    let Some(target) = ForwardTarget::parse(fc) else {
        warn!(forward_channel = %fc, "mirror_inbound: invalid forward_channel format");
        return;
    };
    let mirror = InboundMirror {
        contact_id: contact.id.clone(),
        contact_name: contact.display_name.clone(),
        from_platform: from_platform.to_string(),
        text: text.to_string(),
        attachments,
        ai_paused: contact.ai_paused,
    };
    if let Some(adapter) = adapters.get(&target.platform) {
        let body = mirror.to_text();
        if let Err(e) = adapter
            .send(OutboundMessage {
                channel_type: ChannelType::Tui,
                channel_id: target.channel_id.clone(),
                peer_id: target.channel_id.clone(),
                text: body,
                thread_id: None,
                reply_to_message_id: None,
            })
            .await
        {
            warn!(error = %e, contact_id = %contact.id, "mirror_inbound: send failed");
        }
    } else {
        warn!(
            contact_id = %contact.id,
            platform = %target.platform,
            "mirror_inbound: no adapter for forward target platform"
        );
    }
}

/// Send a work card via best available rendering: adapter's native rich card or text fallback.
async fn send_work_card(
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    target: &ForwardTarget,
    card: &ContactWorkCard,
) -> Result<Option<String>> {
    let adapter = adapters.get(&target.platform).ok_or_else(|| {
        CatClawError::Other(format!(
            "forward platform '{}' has no running adapter",
            target.platform
        ))
    })?;
    // Try rich card via execute() — adapters that implement contact_work_card
    // return a message id. Otherwise fall back to plain text send().
    let action_args = serde_json::json!({
        "channel_id": target.channel_id,
        "draft_id": card.draft_id,
        "contact_id": card.contact_id,
        "contact_name": card.contact_name,
        "role": card.role,
        "via_platform": card.via_platform,
        "payload_preview": card.payload_preview,
        "status_label": card.status_label,
        "show_actions": card.show_actions,
    });
    match adapter.execute("contact_work_card", action_args).await {
        Ok(v) => Ok(v.get("message_id").and_then(|x| x.as_str()).map(String::from)),
        Err(_) => {
            adapter
                .send(OutboundMessage {
                    channel_type: ChannelType::Tui,
                    channel_id: target.channel_id.clone(),
                    peer_id: target.channel_id.clone(),
                    text: card.to_text(),
                    thread_id: None,
                    reply_to_message_id: None,
                })
                .await?;
            Ok(None)
        }
    }
}

/// Re-render the work card after a state change. Best-effort; ignores errors but logs.
async fn refresh_card(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    contact: &Contact,
    draft_id: i64,
    label: &str,
    show_actions: bool,
) {
    let Some(ref fc) = contact.forward_channel else { return };
    let Some(target) = ForwardTarget::parse(fc) else { return };
    let Ok(Some(d)) = db.get_contact_draft(draft_id) else { return };
    let card = ContactWorkCard::from_draft(&d, contact, label, show_actions);

    // If we have a forward_ref, attempt edit-in-place via execute("contact_work_card_edit").
    // Otherwise, send a fresh card.
    if let Some(ref msg_ref) = d.forward_ref {
        let args = serde_json::json!({
            "channel_id": target.channel_id,
            "message_id": msg_ref,
            "draft_id": card.draft_id,
            "contact_id": card.contact_id,
            "contact_name": card.contact_name,
            "role": card.role,
            "via_platform": card.via_platform,
            "payload_preview": card.payload_preview,
            "status_label": card.status_label,
            "show_actions": card.show_actions,
        });
        if let Some(adapter) = adapters.get(&target.platform) {
            if adapter.execute("contact_work_card_edit", args).await.is_ok() {
                return;
            }
        }
    }
    // Fallback: send a fresh card.
    if let Err(e) = send_work_card(adapters, &target, &card).await {
        warn!(error = %e, draft_id, "refresh_card: send fallback failed");
    }
}

/// Try to handle a manual reply detected in a forward channel.
/// Returns Some(()) if the channel matched a contact and the message was forwarded.
/// Returns None when this is just a regular adapter inbound (no contact mirror context).
pub async fn try_manual_reply(
    db: &StateDb,
    adapters: &Arc<HashMap<String, Arc<dyn ChannelAdapter>>>,
    inbound_platform: &str,
    inbound_channel_id: &str,
    inbound_text: &str,
    inbound_sender_id: &str,
) -> Option<()> {
    // Look up any contact whose forward_channel matches this (platform, channel_id).
    // Naive: scan all contacts. Cheap because contacts are typically O(100) per agent.
    let all = db.list_contacts(&Default::default()).ok()?;
    let target_str_a = format!("{}:{}", inbound_platform, inbound_channel_id);
    let mut matched: Option<Contact> = None;
    for c in all {
        let Some(ref fc) = c.forward_channel else { continue };
        if fc == &target_str_a {
            matched = Some(c);
            break;
        }
        // Also match guild-prefixed form
        if fc.starts_with(&format!("{}:", inbound_platform)) && fc.ends_with(&format!("/{}", inbound_channel_id)) {
            matched = Some(c);
            break;
        }
    }
    let contact = matched?;

    // Don't echo bot's own messages; many adapters guard against this in their own loop too.
    if inbound_sender_id == "bot" || inbound_text.is_empty() {
        return Some(());
    }

    info!(
        contact_id = %contact.id,
        platform = inbound_platform,
        "manual reply detected — forwarding to contact"
    );

    // Build a draft, mark it as a manual reply (no approval, immediate send).
    let payload = serde_json::json!({"type": "text", "text": inbound_text});
    let mut draft = ContactDraft::new(&contact.id, &contact.agent_id, payload);
    draft.via_platform = None;
    let draft_id = match db.insert_contact_draft(&draft) {
        Ok(id) => id,
        Err(e) => {
            error!(error = %e, "manual reply: failed to insert draft");
            return Some(());
        }
    };
    draft.id = draft_id;

    match send_to_contact(db, adapters, &contact, &draft).await {
        Ok(()) => {
            let _ = db.update_contact_draft_sent(draft_id);
        }
        Err(e) => {
            warn!(error = %e, contact_id = %contact.id, "manual reply: send failed");
            let _ = db.update_contact_draft_failed(draft_id, &format!("{e}"));
        }
    }
    Some(())
}
