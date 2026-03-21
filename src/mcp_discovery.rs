//! MCP server tool discovery.
//!
//! At gateway startup, probes each user-defined MCP server (from `.mcp.json`)
//! for its `tools/list` to learn which individual tools are available.
//! Results are stored in-memory and exposed via WS `mcp.tools`.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};
use tracing::{debug, info, warn};

/// Result of discovering tools from a single MCP server.
pub struct McpServerTools {
    pub server_name: String,
    pub tools: Vec<String>,
}

/// Discover tools from all servers defined in `.mcp.json`.
/// Each server is probed in parallel with a 10-second timeout.
/// Failures are logged as warnings and skipped.
pub async fn discover_all(
    mcp_json_path: &Path,
    mcp_env: &HashMap<String, HashMap<String, String>>,
) -> Vec<McpServerTools> {
    debug!(path = %mcp_json_path.display(), "MCP discovery: reading .mcp.json");
    let content = match std::fs::read_to_string(mcp_json_path) {
        Ok(c) => c,
        Err(e) => {
            debug!(path = %mcp_json_path.display(), error = %e, "MCP discovery: no .mcp.json found");
            return vec![];
        }
    };
    let parsed: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "MCP discovery: failed to parse .mcp.json");
            return vec![];
        }
    };
    let servers = match parsed.get("mcpServers").and_then(|v| v.as_object()) {
        Some(s) => s.clone(),
        None => {
            debug!("MCP discovery: no mcpServers key in .mcp.json");
            return vec![];
        }
    };
    let server_count = servers.keys().filter(|k| *k != "catclaw").count();
    info!(servers = server_count, "MCP discovery: probing servers");

    let mut handles = Vec::new();

    for (name, def) in &servers {
        if name == "catclaw" {
            continue; // skip built-in
        }
        let name = name.clone();
        let def = def.clone();
        let env = mcp_env.get(&name).cloned().unwrap_or_default();

        handles.push(tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                probe_server(&name, &def, &env),
            )
            .await;

            match result {
                Ok(Ok(tools)) => Some(McpServerTools {
                    server_name: name,
                    tools,
                }),
                Ok(Err(e)) => {
                    warn!(server = %name, error = %e, "MCP discovery failed");
                    None
                }
                Err(_) => {
                    warn!(server = %name, "MCP discovery timed out (10s)");
                    None
                }
            }
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(Some(entry)) = handle.await {
            results.push(entry);
        }
    }
    results
}

/// Probe a single MCP server for its tool list.
async fn probe_server(
    name: &str,
    def: &Value,
    env: &HashMap<String, String>,
) -> Result<Vec<String>, String> {
    let server_type = def
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("stdio");

    match server_type {
        "http" | "sse" => probe_http(name, def, env).await,
        _ => probe_stdio(name, def, env).await,
    }
}

/// Probe a stdio MCP server by spawning the process and sending JSON-RPC.
async fn probe_stdio(
    name: &str,
    def: &Value,
    extra_env: &HashMap<String, String>,
) -> Result<Vec<String>, String> {
    let command = def
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("server '{}': missing 'command'", name))?;

    let args: Vec<String> = def
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Merge env from .mcp.json definition + mcp_env config
    let mut env_vars: HashMap<String, String> = HashMap::new();
    if let Some(env_obj) = def.get("env").and_then(|v| v.as_object()) {
        for (k, v) in env_obj {
            if let Some(s) = v.as_str() {
                env_vars.insert(k.clone(), s.to_string());
            }
        }
    }
    for (k, v) in extra_env {
        env_vars.insert(k.clone(), v.clone());
    }

    debug!(server = %name, command = %command, args = ?args, "spawning MCP server for discovery");

    let mut child = tokio::process::Command::new(command)
        .args(&args)
        .envs(&env_vars)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("server '{}': spawn failed: {}", name, e))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("server '{}': no stdin", name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("server '{}': no stdout", name))?;

    // Send initialize → initialized notification → tools/list
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "catclaw-discovery", "version": "1.0" }
        }
    });
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let tools_list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });

    let msgs = format!(
        "{}\n{}\n{}\n",
        initialize, initialized, tools_list
    );

    stdin
        .write_all(msgs.as_bytes())
        .await
        .map_err(|e| format!("server '{}': write failed: {}", name, e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("server '{}': flush failed: {}", name, e))?;

    // Read responses line by line, looking for tools/list response (id=2)
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut tools = Vec::new();

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("server '{}': read failed: {}", name, e))?;
        if n == 0 {
            break; // EOF
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(resp) = serde_json::from_str::<Value>(trimmed) {
            // Check if this is the response to id=2 (tools/list)
            if resp.get("id").and_then(|v| v.as_u64()) == Some(2) {
                if let Some(tool_arr) = resp
                    .get("result")
                    .and_then(|r| r.get("tools"))
                    .and_then(|t| t.as_array())
                {
                    for tool in tool_arr {
                        if let Some(tool_name) = tool.get("name").and_then(|n| n.as_str()) {
                            tools.push(tool_name.to_string());
                        }
                    }
                }
                break;
            }
        }
    }

    // Kill the subprocess
    let _ = child.kill().await;

    debug!(server = %name, tool_count = tools.len(), "stdio discovery complete");
    Ok(tools)
}

/// Resolve `${VAR}` placeholders in a string using the combined env map.
/// Supports `${VAR}` and `${VAR:-default}` syntax.
fn resolve_vars(s: &str, env: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    // Simple ${VAR} replacement (no nested, no default for now)
    for (key, value) in env {
        let placeholder = format!("${{{}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

/// Probe an HTTP MCP server by POSTing JSON-RPC requests.
async fn probe_http(
    name: &str,
    def: &Value,
    extra_env: &HashMap<String, String>,
) -> Result<Vec<String>, String> {
    let url = def
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("server '{}': missing 'url'", name))?;

    // Build combined env: .mcp.json "env" + mcp_env config
    let mut env_vars: HashMap<String, String> = HashMap::new();
    if let Some(env_obj) = def.get("env").and_then(|v| v.as_object()) {
        for (k, v) in env_obj {
            if let Some(s) = v.as_str() {
                env_vars.insert(k.clone(), s.to_string());
            }
        }
    }
    for (k, v) in extra_env {
        env_vars.insert(k.clone(), v.clone());
    }

    // Build HTTP headers from .mcp.json "headers" with ${VAR} resolution
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(hdr_obj) = def.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in hdr_obj {
            if let Some(s) = v.as_str() {
                let resolved = resolve_vars(s, &env_vars);
                if let (Ok(name), Ok(value)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(&resolved),
                ) {
                    headers.insert(name, value);
                }
            }
        }
    }

    let client = reqwest::Client::new();

    // Initialize
    let init_resp = client
        .post(url)
        .headers(headers.clone())
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "catclaw-discovery", "version": "1.0" }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("server '{}': initialize failed: {}", name, e))?;

    if !init_resp.status().is_success() {
        return Err(format!(
            "server '{}': initialize returned {}",
            name,
            init_resp.status()
        ));
    }

    // Send initialized notification (fire and forget)
    let _ = client
        .post(url)
        .headers(headers.clone())
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .send()
        .await;

    // tools/list
    let resp = client
        .post(url)
        .headers(headers)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }))
        .send()
        .await
        .map_err(|e| format!("server '{}': tools/list failed: {}", name, e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("server '{}': parse tools/list response: {}", name, e))?;

    let mut tools = Vec::new();
    if let Some(tool_arr) = body
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
    {
        for tool in tool_arr {
            if let Some(tool_name) = tool.get("name").and_then(|n| n.as_str()) {
                tools.push(tool_name.to_string());
            }
        }
    }

    debug!(server = %name, tool_count = tools.len(), "http discovery complete");
    Ok(tools)
}
