#![allow(dead_code)]
//! Contacts subsystem — platform-agnostic identity layer.
//!
//! CatClaw 只管「通訊與身份」,業務資料(營養紀錄、健身數據等)由 agent 自選工具
//! (Notion MCP / Palace / 自管 SQLite)儲存,external_ref 欄位作為與外部系統的橋接點。
//!
//! Contact 機制跨平台:DC / TG / Slack / LINE 皆適用。未綁定的 sender 維持原行為(零回歸)。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod pipeline;
pub mod tools;

/// Contact role — agent 行為大方向 hint(非權限系統)。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactRole {
    Admin,
    Client,
    #[default]
    Unknown,
}

impl ContactRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContactRole::Admin => "admin",
            ContactRole::Client => "client",
            ContactRole::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "admin" => ContactRole::Admin,
            "client" => ContactRole::Client,
            _ => ContactRole::Unknown,
        }
    }
}

/// 一個身份,可綁定多個平台帳號。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub agent_id: String,
    pub display_name: String,
    pub role: ContactRole,
    pub tags: Vec<String>,
    /// 鏡射目標,e.g. "discord:guild123/channel456"
    pub forward_channel: Option<String>,
    /// 預設 true:agent 回覆需管理者過審
    pub approval_required: bool,
    /// true 時個案的訊息不派給 agent,純人工接手
    pub ai_paused: bool,
    /// agent 自由 JSON,用於指向外部系統(Notion page id 等)
    pub external_ref: serde_json::Value,
    /// 慢變 profile(過敏源、目標等)。CatClaw 不解讀
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

impl Contact {
    pub fn new(agent_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        let now = Utc::now().to_rfc3339();
        Contact {
            id: Uuid::new_v4().to_string(),
            agent_id: agent_id.into(),
            display_name: display_name.into(),
            role: ContactRole::Unknown,
            tags: Vec::new(),
            forward_channel: None,
            approval_required: true,
            ai_paused: false,
            external_ref: serde_json::json!({}),
            metadata: serde_json::json!({}),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// **多 agent 擴充預備** — 所有讀取 agent_id 的呼叫端應走此 helper,
    /// 未來改為多對多時只改此函式內部即可。
    pub fn owning_agents(&self) -> Vec<String> {
        vec![self.agent_id.clone()]
    }
}

/// 平台綁定(多對多)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactChannel {
    pub contact_id: String,
    pub platform: String,
    pub platform_user_id: String,
    pub is_primary: bool,
    pub last_active_at: Option<String>,
    pub created_at: String,
}

impl ContactChannel {
    pub fn new(
        contact_id: impl Into<String>,
        platform: impl Into<String>,
        platform_user_id: impl Into<String>,
    ) -> Self {
        ContactChannel {
            contact_id: contact_id.into(),
            platform: platform.into(),
            platform_user_id: platform_user_id.into(),
            is_primary: false,
            last_active_at: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

/// Outbound 草稿:agent 想回覆 contact 時建立,經過 forward + approval pipeline。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactDraft {
    pub id: i64,
    pub contact_id: String,
    pub agent_id: String,
    /// 指定發送平台;None = last_active 策略
    pub via_platform: Option<String>,
    /// JSON: {type: "text"|"image"|"flex", ...}
    pub payload: serde_json::Value,
    /// pending | awaiting_approval | revising | sent | ignored | failed
    pub status: String,
    /// 鏡射到管理頻道的 message ref
    pub forward_ref: Option<String>,
    /// 管理者要求 AI 重寫時的指示
    pub revision_note: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sent_at: Option<String>,
}

impl ContactDraft {
    pub fn new(
        contact_id: impl Into<String>,
        agent_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        ContactDraft {
            id: 0,
            contact_id: contact_id.into(),
            agent_id: agent_id.into(),
            via_platform: None,
            payload,
            status: "pending".to_string(),
            forward_ref: None,
            revision_note: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
            sent_at: None,
        }
    }
}

/// 訊息 payload 型別。LINE 專屬格式(flex)由 LINE adapter 序列化;
/// 跨平台訊息(text/image)由各 adapter 透明轉換。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContactPayload {
    Text { text: String },
    Image { url: String, caption: Option<String> },
    Flex { contents: serde_json::Value },
}

impl ContactPayload {
    /// 用於 work card 預覽的純文字摘要。
    pub fn preview(&self) -> String {
        match self {
            ContactPayload::Text { text } => text.clone(),
            ContactPayload::Image { url, caption } => match caption {
                Some(c) => format!("[image] {} ({})", c, url),
                None => format!("[image] {}", url),
            },
            ContactPayload::Flex { .. } => "[flex message]".to_string(),
        }
    }
}

/// 列表查詢 filter。
#[derive(Debug, Clone, Default)]
pub struct ContactsFilter {
    pub agent_id: Option<String>,
    pub role: Option<ContactRole>,
    pub tag: Option<String>,
}

/// Work card button action emitted by an adapter when an admin presses a button
/// (or submits a modal) in the forward channel. Gateway dispatches to the
/// appropriate pipeline call.
#[derive(Debug, Clone)]
pub enum ContactAction {
    Approve(i64),
    Discard(i64),
    /// (draft_id, revision_note)
    Revise(i64, String),
    /// (contact_id)
    Pause(String),
    /// (contact_id)
    Resume(String),
}
