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
            let tools = build_tool_list(adapters);
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
