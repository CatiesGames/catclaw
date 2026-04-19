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
            tools.extend(crate::memory::tools::build_memory_tools());
            tools.extend(crate::contacts::tools::build_contacts_tools());
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

            // Route memory/kg tools first.
            if tool_name.starts_with("memory_") || tool_name.starts_with("kg_") {
                match crate::memory::tools::execute_memory_tool(
                    &gw.state_db,
                    &gw.embedder,
                    tool_name,
                    arguments,
                )
                .await
                {
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

            // Route contacts tools.
            if tool_name.starts_with("contacts_") {
                let default_agent = gw.config.read().unwrap().default_agent_id().unwrap_or("main").to_string();
                match crate::contacts::tools::execute_contacts_tool(
                    &gw.state_db,
                    &gw.state_db,
                    &gw.adapters,
                    &gw.session_manager,
                    &gw.agent_registry,
                    &default_agent,
                    tool_name,
                    arguments,
                )
                .await
                {
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

            // Route social tools, then fall back to adapter tools.
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
        social_tool("instagram_reply_comment", "Reply to an Instagram comment. Auto-stages a draft. If approval is required, a review card is sent to the admin channel.", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID to reply to"},"message":{"type":"string","description":"Reply text"}},"required":["comment_id","message"]})),
        social_tool("instagram_upload_media", "Copy local image files to the gateway media_tmp dir and return public URLs for use with instagram_create_post. Supports batch upload.", serde_json::json!({"type":"object","properties":{"file_paths":{"type":"array","items":{"type":"string"},"description":"Absolute local paths to image files (jpg, png, gif, webp). Supports 1-10 files.","minItems":1,"maxItems":10}},"required":["file_paths"]})),
        social_tool("instagram_reply_template", "Send a template reply to an Instagram comment", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID"},"template_name":{"type":"string","description":"Template name from catclaw.toml"}},"required":["comment_id","template_name"]})),
        social_tool("instagram_delete_comment", "Delete an Instagram comment (requires approval)", serde_json::json!({"type":"object","properties":{"comment_id":{"type":"string","description":"Comment ID to delete"}},"required":["comment_id"]})),
        social_tool("instagram_get_insights", "Get Instagram account insights/analytics", serde_json::json!({"type":"object","properties":{"metric":{"type":"string","description":"Comma-separated metrics (e.g. impressions,reach)"},"period":{"type":"string","description":"Period: day, week, month"}},"required":["metric","period"]})),
        social_tool("instagram_get_inbox", "Query the Social Inbox for Instagram events", serde_json::json!({"type":"object","properties":{"status":{"type":"string","description":"Filter by status: pending, forwarded, draft_ready, sent, ignored, failed"},"limit":{"type":"integer","description":"Max rows to return (default 20)"}},"required":[]})),
        social_tool("instagram_create_post", "Create a new Instagram image post or carousel. Auto-stages a draft if not already staged. If approval is required, a review card is sent to the admin channel.", serde_json::json!({"type":"object","properties":{"image_urls":{"type":"array","items":{"type":"string"},"description":"Public URLs of images to post (JPEG, max 8MB each). 1 image = single post, 2-10 images = carousel.","minItems":1,"maxItems":10},"caption":{"type":"string","description":"Post caption (max 2200 characters)"}},"required":["image_urls","caption"]})),
        social_tool("instagram_send_dm", "Send a direct message to an Instagram user. Auto-stages a draft if not already staged. If approval is required, a review card is sent to the admin channel.", serde_json::json!({"type":"object","properties":{"recipient_id":{"type":"string","description":"Instagram-scoped user ID of the recipient"},"text":{"type":"string","description":"Message text (max 1000 characters)"}},"required":["recipient_id","text"]})),
    ]
}

fn threads_tools() -> Vec<Value> {
    vec![
        social_tool("threads_get_profile", "Get Threads account profile info", serde_json::json!({"type":"object","properties":{},"required":[]})),
        social_tool("threads_get_timeline", "List recent Threads posts", serde_json::json!({"type":"object","properties":{"limit":{"type":"integer","description":"Number of posts to fetch (default 10)"}},"required":[]})),
        social_tool("threads_get_replies", "Get replies on a Threads post. Each reply in the response has its own `id` — use that id (not the parent post_id) when calling threads_reply.", serde_json::json!({"type":"object","properties":{"post_id":{"type":"string","description":"Threads post ID"}},"required":["post_id"]})),
        social_tool("threads_create_post", "Create a new Threads post. Auto-stages a draft if not already staged. If approval is required, a review card is sent to the admin channel.", serde_json::json!({"type":"object","properties":{"text":{"type":"string","description":"Post text content"},"media_urls":{"type":"array","items":{"type":"string"},"description":"Public image URLs (optional). 1 image = single image post, 2-20 images = carousel.","maxItems":20}},"required":["text"]})),
        social_tool("threads_reply", "Reply to a specific Threads post or reply. IMPORTANT: reply_to_id must be the ID of the exact item you are replying to — if replying to a reply, use that reply's `id` from threads_get_replies, NOT the root post ID.", serde_json::json!({"type":"object","properties":{"reply_to_id":{"type":"string","description":"ID of the specific post or reply you are replying TO. Use the reply's own `id` from threads_get_replies, NOT the parent post ID."},"text":{"type":"string","description":"Reply text"}},"required":["reply_to_id","text"]})),
        social_tool("threads_upload_media", "Copy local image files to the gateway media_tmp dir and return public URLs for use with threads_create_post. Supports batch upload.", serde_json::json!({"type":"object","properties":{"file_paths":{"type":"array","items":{"type":"string"},"description":"Absolute local paths to image files (jpg, png, gif, webp). Supports 1-20 files.","minItems":1,"maxItems":20}},"required":["file_paths"]})),
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
            let agent_message = str_arg(&args, "message")?;
            // Auto-stage if no draft exists yet
            let draft = match gw.state_db.find_latest_draft_for_tool("instagram", "reply", Some(comment_id)).ok().flatten() {
                Some(d) => d,
                None => {
                    let mut row = crate::state::SocialDraftRow::new("instagram", "reply", agent_message);
                    row.reply_to_id = Some(comment_id.to_string());
                    // Try inbox first, then API fallback.
                    if let Ok(Some(inbox)) = gw.state_db.get_social_inbox_by_platform_id("instagram", comment_id) {
                        row.original_author = inbox.author_name.clone();
                        row.original_text = inbox.text.clone();
                    } else if let Ok(val) = InstagramClient::new(token.clone(), uid.clone())
                        .get_comment_by_id(comment_id).await {
                        row.original_author = val.get("username").and_then(|v| v.as_str()).map(str::to_string);
                        row.original_text = val.get("text").and_then(|v| v.as_str()).map(str::to_string);
                    }
                    let id = gw.state_db.insert_social_draft(&row)?;
                    gw.state_db.get_social_draft(id)?.ok_or_else(|| CatClawError::Social("failed to read auto-staged draft".into()))?
                }
            };
            let message = draft.content.as_str();
            let result = InstagramClient::new(token, uid).reply_comment(comment_id, message).await?;
            if let Some(reply_id) = result.get("id").and_then(|v| v.as_str()) {
                let _ = gw.state_db.update_social_draft_sent(draft.id, reply_id);
            }
            Ok(result)
        }
        "instagram_upload_media" => {
            let file_paths = arr_arg(&args, "file_paths")?;
            let base_url = cfg.general.webhook_base_url.as_deref()
                .ok_or_else(|| CatClawError::Social("webhook_base_url not configured".into()))?;
            let results: Vec<Value> = file_paths.iter()
                .map(|p| upload_media_file(p, base_url, &cfg.general.workspace, "instagram"))
                .collect::<crate::error::Result<Vec<_>>>()?;
            Ok(serde_json::json!(results))
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
            let image_urls = arr_arg(&args, "image_urls")?;
            let agent_caption = str_arg(&args, "caption")?;
            // Auto-stage if no draft exists yet
            let draft = match gw.state_db.find_latest_draft_for_tool("instagram", "post", None).ok().flatten() {
                Some(d) => d,
                None => {
                    let mut row = crate::state::SocialDraftRow::new("instagram", "post", agent_caption);
                    row.media_urls = image_urls.clone();
                    let id = gw.state_db.insert_social_draft(&row)?;
                    gw.state_db.get_social_draft(id)?.ok_or_else(|| CatClawError::Social("failed to read auto-staged draft".into()))?
                }
            };
            let caption = draft.content.as_str();
            let client = InstagramClient::new(token, uid);
            let result = match draft.media_urls.len() {
                0 => return Err(CatClawError::Social("instagram post requires at least one image".into())),
                1 => client.create_image_post(&draft.media_urls[0], caption).await?,
                _ => {
                    let refs: Vec<&str> = draft.media_urls.iter().map(|s| s.as_str()).collect();
                    client.create_carousel_post(&refs, caption).await?
                }
            };
            if let Some(post_id) = result.get("id").and_then(|v| v.as_str()) {
                let _ = gw.state_db.update_social_draft_sent(draft.id, post_id);
            }
            Ok(result)
        }
        "instagram_send_dm" => {
            let (token, uid) = ig_creds(&cfg)?;
            let recipient_id = str_arg(&args, "recipient_id")?;
            let agent_text = str_arg(&args, "text")?;
            // Auto-stage if no draft exists yet
            let draft = match gw.state_db.find_latest_draft_for_tool("instagram", "dm", Some(recipient_id)).ok().flatten() {
                Some(d) => d,
                None => {
                    let mut row = crate::state::SocialDraftRow::new("instagram", "dm", agent_text);
                    row.reply_to_id = Some(recipient_id.to_string());
                    let id = gw.state_db.insert_social_draft(&row)?;
                    gw.state_db.get_social_draft(id)?.ok_or_else(|| CatClawError::Social("failed to read auto-staged draft".into()))?
                }
            };
            let text = draft.content.as_str();
            let result = InstagramClient::new(token, uid).send_dm(recipient_id, text).await?;
            if let Some(msg_id) = result.get("message_id").and_then(|v| v.as_str()) {
                let _ = gw.state_db.update_social_draft_sent(draft.id, msg_id);
            }
            Ok(result)
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
            let agent_text = str_arg(&args, "text")?;
            let agent_media_urls = opt_arr_arg(&args, "media_urls");
            // Auto-stage if no draft exists yet
            let draft = match gw.state_db.find_latest_draft_for_tool("threads", "post", None).ok().flatten() {
                Some(d) => d,
                None => {
                    let mut row = crate::state::SocialDraftRow::new("threads", "post", agent_text);
                    row.media_urls = agent_media_urls;
                    let id = gw.state_db.insert_social_draft(&row)?;
                    gw.state_db.get_social_draft(id)?.ok_or_else(|| CatClawError::Social("failed to read auto-staged draft".into()))?
                }
            };
            let text = draft.content.as_str();
            let client = ThreadsClient::new(token, uid);
            let result = match draft.media_urls.len() {
                0 => client.create_post(text).await?,
                1 => client.create_image_post(&draft.media_urls[0], text).await?,
                _ => {
                    let refs: Vec<&str> = draft.media_urls.iter().map(|s| s.as_str()).collect();
                    client.create_carousel_post(&refs, text).await?
                }
            };
            if let Some(post_id) = result.get("id").and_then(|v| v.as_str()) {
                let _ = gw.state_db.update_social_draft_sent(draft.id, post_id);
            }
            Ok(result)
        }
        "threads_reply" => {
            let (token, uid) = th_creds(&cfg)?;
            let reply_to_id = str_arg(&args, "reply_to_id")?;
            let agent_text = str_arg(&args, "text")?;
            // Auto-stage if no draft exists yet
            let draft = match gw.state_db.find_latest_draft_for_tool("threads", "reply", Some(reply_to_id)).ok().flatten() {
                Some(d) => d,
                None => {
                    let mut row = crate::state::SocialDraftRow::new("threads", "reply", agent_text);
                    row.reply_to_id = Some(reply_to_id.to_string());
                    if let Ok(Some(inbox)) = gw.state_db.get_social_inbox_by_platform_id("threads", reply_to_id) {
                        row.original_author = inbox.author_name.clone();
                        row.original_text = inbox.text.clone();
                    } else if let Ok(val) = ThreadsClient::new(token.clone(), uid.clone())
                        .get_post_by_id(reply_to_id).await {
                        row.original_author = val.get("username").and_then(|v| v.as_str()).map(str::to_string);
                        row.original_text = val.get("text").and_then(|v| v.as_str()).map(str::to_string);
                    }
                    let id = gw.state_db.insert_social_draft(&row)?;
                    gw.state_db.get_social_draft(id)?.ok_or_else(|| CatClawError::Social("failed to read auto-staged draft".into()))?
                }
            };
            let text = draft.content.as_str();
            let result = ThreadsClient::new(token, uid).reply(reply_to_id, text).await?;
            if let Some(reply_id) = result.get("id").and_then(|v| v.as_str()) {
                let _ = gw.state_db.update_social_draft_sent(draft.id, reply_id);
            }
            Ok(result)
        }
        "threads_upload_media" => {
            let file_paths = arr_arg(&args, "file_paths")?;
            let base_url = cfg.general.webhook_base_url.as_deref()
                .ok_or_else(|| CatClawError::Social("webhook_base_url not configured".into()))?;
            let results: Vec<Value> = file_paths.iter()
                .map(|p| upload_media_file(p, base_url, &cfg.general.workspace, "threads"))
                .collect::<crate::error::Result<Vec<_>>>()?;
            Ok(serde_json::json!(results))
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

/// Copy a local image into `{workspace}/media_tmp/`, converting format if needed.
///
/// - Instagram: all formats → JPEG (Meta API requires JPEG for image posts)
/// - Threads: GIF/WebP → JPEG, JPEG/PNG kept as-is
///
/// Conversion preserves original dimensions and uses quality 95 for JPEG.
fn upload_media_file(
    file_path: &str,
    base_url: &str,
    workspace: &std::path::Path,
    platform: &str,
) -> crate::error::Result<Value> {
    use std::path::Path;

    let src = Path::new(file_path);
    if !src.exists() {
        return Err(crate::error::CatClawError::Social(format!(
            "file not found: {}", file_path
        )));
    }

    let ext = src.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") {
        return Err(crate::error::CatClawError::Social(format!(
            "unsupported file type '.{}' — must be jpg, png, gif, or webp", ext
        )));
    }

    let media_dir = workspace.join("media_tmp");
    std::fs::create_dir_all(&media_dir).map_err(|e| {
        crate::error::CatClawError::Social(format!("failed to create media_tmp dir: {e}"))
    })?;

    // Determine if conversion is needed
    let needs_jpeg = match platform {
        "instagram" => !matches!(ext.as_str(), "jpg" | "jpeg"),
        "threads" => !matches!(ext.as_str(), "jpg" | "jpeg" | "png"),
        _ => false,
    };

    let (filename, converted) = if needs_jpeg {
        let img = image::open(src).map_err(|e| {
            crate::error::CatClawError::Social(format!("failed to open image: {e}"))
        })?;
        let out_name = format!("{}.jpg", uuid_v4());
        let dest = media_dir.join(&out_name);
        let writer = std::io::BufWriter::new(std::fs::File::create(&dest).map_err(|e| {
            crate::error::CatClawError::Social(format!("failed to create output file: {e}"))
        })?);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 95);
        img.write_with_encoder(encoder).map_err(|e| {
            crate::error::CatClawError::Social(format!("failed to convert to JPEG: {e}"))
        })?;
        (out_name, true)
    } else {
        let out_name = format!("{}.{}", uuid_v4(), ext);
        let dest = media_dir.join(&out_name);
        std::fs::copy(src, &dest).map_err(|e| {
            crate::error::CatClawError::Social(format!("failed to copy file: {e}"))
        })?;
        (out_name, false)
    };

    let url = format!("{}/media/{}", base_url.trim_end_matches('/'), filename);
    Ok(serde_json::json!({
        "url": url,
        "filename": filename,
        "converted": converted,
        "original_format": ext,
    }))
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

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

/// Parse a JSON value that may be a real array or a stringified JSON array.
/// Agents sometimes pass `"[\"url1\"]"` (string) instead of `["url1"]` (array).
fn parse_string_or_array(v: &Value) -> Vec<String> {
    if let Some(arr) = v.as_array() {
        return arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    }
    if let Some(s) = v.as_str() {
        if s.starts_with('[') {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(s) {
                return parsed;
            }
        }
        if !s.is_empty() {
            return vec![s.to_string()];
        }
    }
    vec![]
}

/// Parse a required array-of-strings argument (tolerates stringified arrays).
fn arr_arg(args: &Value, key: &str) -> crate::error::Result<Vec<String>> {
    let v = args.get(key)
        .ok_or_else(|| crate::error::CatClawError::Social(format!("missing argument '{}'", key)))?;
    let urls = parse_string_or_array(v);
    if urls.is_empty() {
        return Err(crate::error::CatClawError::Social(format!("argument '{}' is empty", key)));
    }
    Ok(urls)
}

/// Parse an optional array-of-strings argument (tolerates stringified arrays).
fn opt_arr_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .map(parse_string_or_array)
        .unwrap_or_default()
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
