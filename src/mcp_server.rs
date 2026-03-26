use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::Value;

use crate::channel::ChannelAdapter;
use crate::gateway::GatewayHandle;

/// Build the MCP router (mounted at `/mcp`).
/// Merged into the main gateway server alongside WebSocket.
/// Shares `Arc<GatewayHandle>` state with the WS handler.
pub fn router() -> Router<Arc<GatewayHandle>> {
    Router::new().route("/mcp", post(handle_mcp))
}

/// Handle MCP JSON-RPC requests
async fn handle_mcp(
    State(gw): State<Arc<GatewayHandle>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let adapters = &gw.adapters;
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("");

    match method {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "catclaw",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            jsonrpc_ok(id, result)
        }
        "notifications/initialized" => {
            // Client acknowledgement — no response needed for notifications,
            // but since this is HTTP we return empty success
            (StatusCode::OK, Json(serde_json::json!({})))
        }
        "tools/list" => {
            let mut tools = build_tool_list(adapters);
            tools.extend(build_social_tools(&gw));
            let result = serde_json::json!({ "tools": tools });
            jsonrpc_ok(id, result)
        }
        "tools/call" => {
            let params = body.get("params").cloned().unwrap_or(Value::Null);
            let tool_name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));

            // Route social tools first, then fall back to adapter tools.
            if tool_name.starts_with("instagram_") || tool_name.starts_with("threads_") {
                match execute_social_tool(&gw, tool_name, arguments).await {
                    Ok(result) => {
                        let response = serde_json::json!({
                            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }]
                        });
                        return jsonrpc_ok(id, response);
                    }
                    Err(e) => {
                        let response = serde_json::json!({
                            "content": [{ "type": "text", "text": format!("Error: {}", e) }],
                            "isError": true
                        });
                        return jsonrpc_ok(id, response);
                    }
                }
            }

            match execute_tool(adapters, tool_name, arguments).await {
                Ok(result) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
                        }]
                    });
                    jsonrpc_ok(id, response)
                }
                Err(e) => {
                    let response = serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {}", e)
                        }],
                        "isError": true
                    });
                    jsonrpc_ok(id, response)
                }
            }
        }
        "ping" => jsonrpc_ok(id, serde_json::json!({})),
        _ => jsonrpc_error(id, -32601, &format!("method not found: {}", method)),
    }
}

/// Build the MCP tool list from all adapter supported_actions
fn build_tool_list(adapters: &HashMap<String, Arc<dyn ChannelAdapter>>) -> Vec<Value> {
    let mut tools = Vec::new();

    for (adapter_name, adapter) in adapters {
        for action in adapter.supported_actions() {
            let tool_name = format!("{}_{}", adapter_name, action.name);
            tools.push(serde_json::json!({
                "name": tool_name,
                "description": action.description,
                "inputSchema": action.params_schema,
            }));
        }
    }

    tools
}

/// Execute a tool call by routing to the correct adapter
async fn execute_tool(
    adapters: &HashMap<String, Arc<dyn ChannelAdapter>>,
    tool_name: &str,
    arguments: Value,
) -> crate::error::Result<Value> {
    // Parse tool name: "{adapter}_{action}"
    let (adapter_name, action) = tool_name
        .split_once('_')
        .ok_or_else(|| {
            crate::error::CatClawError::Channel(format!(
                "invalid tool name '{}': expected format 'adapter_action'",
                tool_name
            ))
        })?;

    let adapter = adapters.get(adapter_name).ok_or_else(|| {
        crate::error::CatClawError::Channel(format!(
            "no adapter '{}' found for tool '{}'",
            adapter_name, tool_name
        ))
    })?;

    adapter.execute(action, arguments).await
}

// ── Social MCP Tools ──────────────────────────────────────────────────────────

/// Build social tool definitions (only when social config exists).
fn build_social_tools(gw: &GatewayHandle) -> Vec<Value> {
    let cfg = gw.config.read().unwrap();
    let mut tools = Vec::new();

    if cfg.social.instagram.is_some() {
        tools.extend(instagram_tools());
    }
    if cfg.social.threads.is_some() {
        tools.extend(threads_tools());
    }
    tools
}

fn instagram_tools() -> Vec<Value> {
    vec![
        social_tool("instagram_get_profile", "Get Instagram account profile info", serde_json::json!({"type":"object","properties":{},"required":[]})),
        social_tool("instagram_get_media", "List recent Instagram posts", serde_json::json!({"type":"object","properties":{"limit":{"type":"integer","description":"Number of posts to fetch (default 10)"}},"required":[]})),
        social_tool("instagram_get_comments", "Get comments on an Instagram post", serde_json::json!({"type":"object","properties":{"media_id":{"type":"string","description":"Instagram media/post ID"}},"required":["media_id"]})),
        social_tool("instagram_reply_comment", "Reply to an Instagram comment (requires approval)", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID to reply to"},"message":{"type":"string","description":"Reply text"}},"required":["comment_id","message"]})),
        social_tool("instagram_stage_reply", "Stage a reply draft for human review (use instead of instagram_reply_comment for auto_reply flow)", serde_json::json!({"type":"object","properties":{"inbox_id":{"type":"integer","description":"Social inbox row ID"},"reply_text":{"type":"string","description":"Draft reply text"}},"required":["inbox_id","reply_text"]})),
        social_tool("instagram_reply_template", "Send a template reply to an Instagram comment", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID"},"template_name":{"type":"string","description":"Template name from catclaw.toml"}},"required":["comment_id","template_name"]})),
        social_tool("instagram_delete_comment", "Delete an Instagram comment (requires approval)", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID to delete"}},"required":["comment_id"]})),
        social_tool("instagram_get_insights", "Get Instagram account insights/analytics", serde_json::json!({"type":"object","properties":{"metric":{"type":"string","description":"Comma-separated metrics (e.g. impressions,reach)"},"period":{"type":"string","description":"Period: day, week, month"}},"required":["metric","period"]})),
        social_tool("instagram_get_inbox", "Query the Social Inbox for Instagram events", serde_json::json!({"type":"object","properties":{"status":{"type":"string","description":"Filter by status: pending, forwarded, draft_ready, sent, ignored, failed"},"limit":{"type":"integer","description":"Max rows to return (default 20)"}},"required":[]})),
        social_tool("instagram_create_post", "Create a new Instagram image post (requires approval)", serde_json::json!({"type":"object","properties":{"image_url":{"type":"string","description":"Public URL of the image to post (JPEG, max 8MB)"},"caption":{"type":"string","description":"Post caption (max 2200 characters)"}},"required":["image_url","caption"]})),
        social_tool("instagram_send_dm", "Send a direct message to an Instagram user (requires approval)", serde_json::json!({"type":"object","properties":{"recipient_id":{"type":"string","description":"Instagram-scoped user ID of the recipient"},"text":{"type":"string","description":"Message text (max 1000 characters)"}},"required":["recipient_id","text"]})),
    ]
}

fn threads_tools() -> Vec<Value> {
    vec![
        social_tool("threads_get_profile", "Get Threads account profile info", serde_json::json!({"type":"object","properties":{},"required":[]})),
        social_tool("threads_get_timeline", "List recent Threads posts", serde_json::json!({"type":"object","properties":{"limit":{"type":"integer","description":"Number of posts to fetch (default 10)"}},"required":[]})),
        social_tool("threads_get_replies", "Get replies on a Threads post", serde_json::json!({"type":"object","properties":{"post_id":{"type":"string","description":"Threads post ID"}},"required":["post_id"]})),
        social_tool("threads_create_post", "Create a new Threads post (requires approval)", serde_json::json!({"type":"object","properties":{"text":{"type":"string","description":"Post text content"}},"required":["text"]})),
        social_tool("threads_reply", "Reply to a Threads post (requires approval)", serde_json::json!({"type":"object","properties":{"post_id":{"type":"string","description":"Post ID to reply to"},"text":{"type":"string","description":"Reply text"}},"required":["post_id","text"]})),
        social_tool("threads_stage_reply", "Stage a reply draft for human review (use instead of threads_reply for auto_reply flow)", serde_json::json!({"type":"object","properties":{"inbox_id":{"type":"integer","description":"Social inbox row ID"},"reply_text":{"type":"string","description":"Draft reply text"}},"required":["inbox_id","reply_text"]})),
        social_tool("threads_reply_template", "Send a template reply to a Threads post", serde_json::json!({"type":"object","properties":{"post_id":{"type":"string","description":"Post ID"},"template_name":{"type":"string","description":"Template name from catclaw.toml"}},"required":["post_id","template_name"]})),
        social_tool("threads_delete_post", "Delete a Threads post (requires approval)", serde_json::json!({"type":"object","properties":{"post_id":{"type":"string","description":"Post ID to delete"}},"required":["post_id"]})),
        social_tool("threads_get_insights", "Get Threads account insights/analytics", serde_json::json!({"type":"object","properties":{"metric":{"type":"string","description":"Comma-separated metrics"}},"required":["metric"]})),
        social_tool("threads_get_inbox", "Query the Social Inbox for Threads events", serde_json::json!({"type":"object","properties":{"status":{"type":"string","description":"Filter by status: pending, forwarded, draft_ready, sent, ignored, failed"},"limit":{"type":"integer","description":"Max rows to return (default 20)"}},"required":[]})),
        social_tool("threads_keyword_search", "Search Threads posts by keyword", serde_json::json!({"type":"object","properties":{"q":{"type":"string","description":"Keyword to search for"},"search_type":{"type":"string","description":"TOP (default) or RECENT"},"limit":{"type":"integer","description":"Max results (default 25, max 100)"}},"required":["q"]})),
    ]
}

fn social_tool(name: &str, description: &str, schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
    })
}

/// Execute a social tool call.
async fn execute_social_tool(
    gw: &GatewayHandle,
    tool_name: &str,
    args: Value,
) -> crate::error::Result<Value> {
    use crate::social::instagram::InstagramClient;
    use crate::social::threads::ThreadsClient;
    use crate::error::CatClawError;

    let cfg = gw.config.read().unwrap().clone();

    match tool_name {
        // ── Instagram ────────────────────────────────────────────────────────
        "instagram_get_profile" => {
            let (token, uid) = ig_creds(&cfg)?;
            InstagramClient::new(token, uid).get_profile().await
        }
        "instagram_get_media" => {
            let (token, uid) = ig_creds(&cfg)?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
            InstagramClient::new(token, uid).get_media(limit).await
        }
        "instagram_get_comments" => {
            let (token, uid) = ig_creds(&cfg)?;
            let media_id = str_arg(&args, "media_id")?;
            InstagramClient::new(token, uid).get_comments(media_id, None).await
        }
        "instagram_reply_comment" => {
            let (token, uid) = ig_creds(&cfg)?;
            let comment_id = str_arg(&args, "comment_id")?;
            let message = str_arg(&args, "message")?;
            InstagramClient::new(token, uid).reply_comment(comment_id, message).await
        }
        "instagram_stage_reply" => {
            let inbox_id = args.get("inbox_id").and_then(|v| v.as_i64())
                .ok_or_else(|| CatClawError::Social("missing inbox_id".into()))?;
            let reply_text = str_arg(&args, "reply_text")?;
            gw.state_db.update_social_inbox_draft(inbox_id, reply_text, "draft_ready")?;
            Ok(serde_json::json!({ "status": "staged", "inbox_id": inbox_id }))
        }
        "instagram_reply_template" => {
            let (token, uid) = ig_creds(&cfg)?;
            let comment_id = str_arg(&args, "comment_id")?;
            let template_name = str_arg(&args, "template_name")?;
            let ig_cfg = cfg.social.instagram.as_ref()
                .ok_or_else(|| CatClawError::Social("no instagram config".into()))?;
            let text = ig_cfg.templates.get(template_name)
                .ok_or_else(|| CatClawError::Social(format!("template '{}' not found", template_name)))?
                .clone();
            InstagramClient::new(token, uid).reply_comment(comment_id, &text).await
        }
        "instagram_delete_comment" => {
            let (token, uid) = ig_creds(&cfg)?;
            let comment_id = str_arg(&args, "comment_id")?;
            InstagramClient::new(token, uid).delete_comment(comment_id).await
        }
        "instagram_get_insights" => {
            let (token, uid) = ig_creds(&cfg)?;
            let metric = str_arg(&args, "metric")?;
            let period = str_arg(&args, "period")?;
            InstagramClient::new(token, uid).get_insights(metric, period).await
        }
        "instagram_get_inbox" => {
            let status = args.get("status").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let rows = gw.state_db.list_social_inbox(Some("instagram"), status, limit)?;
            Ok(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
        }
        "instagram_create_post" => {
            let (token, uid) = ig_creds(&cfg)?;
            let image_url = str_arg(&args, "image_url")?;
            let caption = str_arg(&args, "caption")?;
            InstagramClient::new(token, uid).create_image_post(image_url, caption).await
        }
        "instagram_send_dm" => {
            let (token, uid) = ig_creds(&cfg)?;
            let recipient_id = str_arg(&args, "recipient_id")?;
            let text = str_arg(&args, "text")?;
            InstagramClient::new(token, uid).send_dm(recipient_id, text).await
        }

        // ── Threads ──────────────────────────────────────────────────────────
        "threads_get_profile" => {
            let (token, uid) = th_creds(&cfg)?;
            ThreadsClient::new(token, uid).get_profile().await
        }
        "threads_get_timeline" => {
            let (token, uid) = th_creds(&cfg)?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
            ThreadsClient::new(token, uid).get_timeline(limit).await
        }
        "threads_get_replies" => {
            let (token, uid) = th_creds(&cfg)?;
            let post_id = str_arg(&args, "post_id")?;
            ThreadsClient::new(token, uid).get_replies(post_id, None).await
        }
        "threads_create_post" => {
            let (token, uid) = th_creds(&cfg)?;
            let text = str_arg(&args, "text")?;
            ThreadsClient::new(token, uid).create_post(text).await
        }
        "threads_reply" => {
            let (token, uid) = th_creds(&cfg)?;
            let post_id = str_arg(&args, "post_id")?;
            let text = str_arg(&args, "text")?;
            ThreadsClient::new(token, uid).reply(post_id, text).await
        }
        "threads_stage_reply" => {
            let inbox_id = args.get("inbox_id").and_then(|v| v.as_i64())
                .ok_or_else(|| CatClawError::Social("missing inbox_id".into()))?;
            let reply_text = str_arg(&args, "reply_text")?;
            gw.state_db.update_social_inbox_draft(inbox_id, reply_text, "draft_ready")?;
            Ok(serde_json::json!({ "status": "staged", "inbox_id": inbox_id }))
        }
        "threads_reply_template" => {
            let (token, uid) = th_creds(&cfg)?;
            let post_id = str_arg(&args, "post_id")?;
            let template_name = str_arg(&args, "template_name")?;
            let th_cfg = cfg.social.threads.as_ref()
                .ok_or_else(|| CatClawError::Social("no threads config".into()))?;
            let text = th_cfg.templates.get(template_name)
                .ok_or_else(|| CatClawError::Social(format!("template '{}' not found", template_name)))?
                .clone();
            ThreadsClient::new(token, uid).reply(post_id, &text).await
        }
        "threads_delete_post" => {
            let (token, uid) = th_creds(&cfg)?;
            let post_id = str_arg(&args, "post_id")?;
            ThreadsClient::new(token, uid).delete_post(post_id).await
        }
        "threads_get_insights" => {
            let (token, uid) = th_creds(&cfg)?;
            let metric = str_arg(&args, "metric")?;
            ThreadsClient::new(token, uid).get_insights(metric).await
        }
        "threads_get_inbox" => {
            let status = args.get("status").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let rows = gw.state_db.list_social_inbox(Some("threads"), status, limit)?;
            Ok(serde_json::to_value(&rows).unwrap_or(serde_json::json!([])))
        }
        "threads_keyword_search" => {
            let (token, uid) = th_creds(&cfg)?;
            let q = str_arg(&args, "q")?;
            let search_type = args.get("search_type").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            ThreadsClient::new(token, uid).keyword_search(q, search_type, limit).await
        }

        other => Err(CatClawError::Social(format!("unknown social tool '{}'", other))),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ig_creds(cfg: &crate::config::Config) -> crate::error::Result<(String, String)> {
    let ig = cfg.social.instagram.as_ref()
        .ok_or_else(|| crate::error::CatClawError::Social("instagram not configured".into()))?;
    let token = std::env::var(&ig.token_env)
        .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", ig.token_env)))?;
    Ok((token, ig.user_id.clone()))
}

fn th_creds(cfg: &crate::config::Config) -> crate::error::Result<(String, String)> {
    let th = cfg.social.threads.as_ref()
        .ok_or_else(|| crate::error::CatClawError::Social("threads not configured".into()))?;
    let token = std::env::var(&th.token_env)
        .map_err(|_| crate::error::CatClawError::Social(format!("env '{}' not set", th.token_env)))?;
    Ok((token, th.user_id.clone()))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> crate::error::Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::CatClawError::Social(format!("missing argument '{}'", key)))
}

/// Build a JSON-RPC success response
fn jsonrpc_ok(id: Value, result: Value) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })),
    )
}

/// Build a JSON-RPC error response
fn jsonrpc_error(id: Value, code: i32, message: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message
            }
        })),
    )
}
