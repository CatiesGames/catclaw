//! MCP tool schemas + dispatch for `mcp__catclaw__contacts_*`.
//!
//! Stage 2 提供 CRUD/bind/list/draft 管理。`contacts_reply`、`contacts_ai_pause`、
//! `contacts_ai_resume`、`contacts_draft_request_revision` 等需要 outbound pipeline
//! 的 tools 在 Stage 3 補齊。

use serde_json::Value;

use crate::contacts::{Contact, ContactChannel, ContactRole, ContactsFilter};
use crate::error::{CatClawError, Result};
use crate::state::StateDb;

/// Tool list for tools/list response.
pub fn build_contacts_tools() -> Vec<Value> {
    vec![
        tool(
            "contacts_create",
            "Create a new contact (a person you communicate with on any channel: LINE/Discord/Telegram/Slack). \
             Returns the new contact id. agent_id defaults to the default agent.",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "name":{"type":"string","description":"Display name"},
                    "agent_id":{"type":"string","description":"Owning agent id (optional, defaults to default agent)"},
                    "role":{"type":"string","enum":["admin","client","unknown"],"description":"Role hint for the agent. Default: unknown"},
                    "tags":{"type":"array","items":{"type":"string"},"description":"Free-form tags"},
                    "approval_required":{"type":"boolean","description":"Whether agent replies need admin approval. Default: true"}
                },
                "required":["name"]
            }),
        ),
        tool(
            "contacts_get",
            "Get a contact by id, or look up by platform user id (e.g. LINE userId). Returns contact + bound channels.",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "id":{"type":"string","description":"Contact id (uuid)"},
                    "platform":{"type":"string","description":"Platform name (line/discord/telegram/slack) — use with platform_user_id"},
                    "platform_user_id":{"type":"string","description":"Platform-specific user id"}
                }
            }),
        ),
        tool(
            "contacts_list",
            "List contacts. Filter by agent_id, role, or tag.",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "agent_id":{"type":"string"},
                    "role":{"type":"string","enum":["admin","client","unknown"]},
                    "tag":{"type":"string"}
                }
            }),
        ),
        tool(
            "contacts_update",
            "Update a contact. Only provided fields are changed.",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "id":{"type":"string"},
                    "display_name":{"type":"string"},
                    "role":{"type":"string","enum":["admin","client","unknown"]},
                    "tags":{"type":"array","items":{"type":"string"}},
                    "forward_channel":{"type":"string","description":"Mirror target like 'discord:guild_id/channel_id'. Pass empty string to clear."},
                    "approval_required":{"type":"boolean"},
                    "ai_paused":{"type":"boolean","description":"When true, inbound messages from this contact are NOT dispatched to the agent (manual takeover)."},
                    "external_ref":{"type":"object","description":"Free-form JSON to store pointers into external systems (Notion page id, etc)."},
                    "metadata":{"type":"object","description":"Free-form JSON for slow-changing profile data (allergies, goals, etc)."}
                },
                "required":["id"]
            }),
        ),
        tool(
            "contacts_delete",
            "Delete a contact and all its channel bindings + drafts (cascade).",
            serde_json::json!({
                "type":"object",
                "properties":{"id":{"type":"string"}},
                "required":["id"]
            }),
        ),
        tool(
            "contacts_bind_channel",
            "Bind a platform user id to a contact. Same contact can have multiple channels (LINE+TG+Discord).",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "id":{"type":"string","description":"Contact id"},
                    "platform":{"type":"string","enum":["line","discord","telegram","slack"]},
                    "platform_user_id":{"type":"string"},
                    "is_primary":{"type":"boolean","description":"Mark as primary channel. Default: false"}
                },
                "required":["id","platform","platform_user_id"]
            }),
        ),
        tool(
            "contacts_unbind_channel",
            "Remove a channel binding by (platform, platform_user_id).",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "platform":{"type":"string"},
                    "platform_user_id":{"type":"string"}
                },
                "required":["platform","platform_user_id"]
            }),
        ),
        tool(
            "contacts_drafts_list",
            "List outbound drafts pending approval (or other status). Filter by contact_id and status.",
            serde_json::json!({
                "type":"object",
                "properties":{
                    "contact_id":{"type":"string"},
                    "status":{"type":"string","enum":["pending","awaiting_approval","revising","sent","ignored","failed"]},
                    "limit":{"type":"integer","description":"Max rows. Default 50"}
                }
            }),
        ),
        tool(
            "contacts_draft_approve",
            "Approve a pending outbound draft and trigger send. (Stage 3: full pipeline; Stage 2: marks status only.)",
            serde_json::json!({
                "type":"object",
                "properties":{"draft_id":{"type":"integer"}},
                "required":["draft_id"]
            }),
        ),
        tool(
            "contacts_draft_discard",
            "Discard a pending outbound draft.",
            serde_json::json!({
                "type":"object",
                "properties":{"draft_id":{"type":"integer"}},
                "required":["draft_id"]
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

/// Dispatch a `contacts_*` tool call.
pub async fn execute_contacts_tool(
    db: &StateDb,
    default_agent_id: &str,
    tool_name: &str,
    args: Value,
) -> Result<Value> {
    match tool_name {
        "contacts_create" => {
            let name = str_arg(&args, "name")?;
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or(default_agent_id)
                .to_string();
            let mut c = Contact::new(agent_id, name);
            if let Some(r) = args.get("role").and_then(|v| v.as_str()) {
                c.role = ContactRole::parse(r);
            }
            if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
                c.tags = tags
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
            }
            if let Some(b) = args.get("approval_required").and_then(|v| v.as_bool()) {
                c.approval_required = b;
            }
            db.insert_contact(&c)?;
            Ok(serde_json::to_value(&c).unwrap_or(Value::Null))
        }
        "contacts_get" => {
            if let Some(id) = args.get("id").and_then(|v| v.as_str()) {
                let c = db
                    .get_contact(id)?
                    .ok_or_else(|| CatClawError::Other(format!("contact '{}' not found", id)))?;
                let channels = db.list_contact_channels(&c.id)?;
                Ok(serde_json::json!({
                    "contact": c,
                    "channels": channels,
                }))
            } else if let (Some(p), Some(uid)) = (
                args.get("platform").and_then(|v| v.as_str()),
                args.get("platform_user_id").and_then(|v| v.as_str()),
            ) {
                let c = db.get_contact_by_platform_user(p, uid)?.ok_or_else(|| {
                    CatClawError::Other(format!(
                        "no contact bound to {}:{}",
                        p, uid
                    ))
                })?;
                let channels = db.list_contact_channels(&c.id)?;
                Ok(serde_json::json!({
                    "contact": c,
                    "channels": channels,
                }))
            } else {
                Err(CatClawError::Other(
                    "contacts_get: provide either 'id' or ('platform','platform_user_id')".into(),
                ))
            }
        }
        "contacts_list" => {
            let filter = ContactsFilter {
                agent_id: args.get("agent_id").and_then(|v| v.as_str()).map(String::from),
                role: args
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(ContactRole::parse),
                tag: args.get("tag").and_then(|v| v.as_str()).map(String::from),
            };
            let rows = db.list_contacts(&filter)?;
            Ok(serde_json::to_value(&rows).unwrap_or(Value::Null))
        }
        "contacts_update" => {
            let id = str_arg(&args, "id")?;
            let mut c = db
                .get_contact(id)?
                .ok_or_else(|| CatClawError::Other(format!("contact '{}' not found", id)))?;

            if let Some(v) = args.get("display_name").and_then(|v| v.as_str()) {
                c.display_name = v.to_string();
            }
            if let Some(v) = args.get("role").and_then(|v| v.as_str()) {
                c.role = ContactRole::parse(v);
            }
            if let Some(arr) = args.get("tags").and_then(|v| v.as_array()) {
                c.tags = arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            }
            if let Some(v) = args.get("forward_channel") {
                if let Some(s) = v.as_str() {
                    c.forward_channel = if s.is_empty() { None } else { Some(s.to_string()) };
                } else if v.is_null() {
                    c.forward_channel = None;
                }
            }
            if let Some(b) = args.get("approval_required").and_then(|v| v.as_bool()) {
                c.approval_required = b;
            }
            if let Some(b) = args.get("ai_paused").and_then(|v| v.as_bool()) {
                c.ai_paused = b;
            }
            if let Some(v) = args.get("external_ref") {
                c.external_ref = v.clone();
            }
            if let Some(v) = args.get("metadata") {
                c.metadata = v.clone();
            }
            db.update_contact(&c)?;
            Ok(serde_json::to_value(&c).unwrap_or(Value::Null))
        }
        "contacts_delete" => {
            let id = str_arg(&args, "id")?;
            db.delete_contact(id)?;
            Ok(serde_json::json!({"deleted": id}))
        }
        "contacts_bind_channel" => {
            let id = str_arg(&args, "id")?;
            let platform = str_arg(&args, "platform")?;
            let pu = str_arg(&args, "platform_user_id")?;
            // Verify contact exists
            db.get_contact(id)?
                .ok_or_else(|| CatClawError::Other(format!("contact '{}' not found", id)))?;
            let mut ch = ContactChannel::new(id, platform, pu);
            ch.is_primary = args.get("is_primary").and_then(|v| v.as_bool()).unwrap_or(false);
            db.upsert_contact_channel(&ch)?;
            Ok(serde_json::to_value(&ch).unwrap_or(Value::Null))
        }
        "contacts_unbind_channel" => {
            let platform = str_arg(&args, "platform")?;
            let pu = str_arg(&args, "platform_user_id")?;
            db.delete_contact_channel(platform, pu)?;
            Ok(serde_json::json!({"unbound": {"platform": platform, "platform_user_id": pu}}))
        }
        "contacts_drafts_list" => {
            let contact_id = args.get("contact_id").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            let rows = db.list_contact_drafts(contact_id, status, limit)?;
            Ok(serde_json::to_value(&rows).unwrap_or(Value::Null))
        }
        "contacts_draft_approve" => {
            let id = args
                .get("draft_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| CatClawError::Other("missing draft_id".into()))?;
            // Stage 2 stub: mark approved (Stage 3 wires actual send pipeline)
            db.update_contact_draft_status(id, "awaiting_approval")?;
            Ok(serde_json::json!({"approved": id, "note": "send pipeline activates in Stage 3"}))
        }
        "contacts_draft_discard" => {
            let id = args
                .get("draft_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| CatClawError::Other("missing draft_id".into()))?;
            db.update_contact_draft_status(id, "ignored")?;
            Ok(serde_json::json!({"discarded": id}))
        }
        other => Err(CatClawError::Other(format!(
            "unknown contacts tool '{}'",
            other
        ))),
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| CatClawError::Other(format!("missing argument '{}'", key)))
}
