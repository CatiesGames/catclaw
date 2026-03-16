/// PreToolUse hook implementation.
///
/// Called by Claude Code as a subprocess before each tool execution when
/// the agent has approval rules configured. Reads tool info from stdin,
/// checks config rules, and either:
/// - Exits 0: allow the tool to execute
/// - Exits 2: block the tool (stderr message shown to agent as tool error)

use std::path::Path;

use crate::approval::HookInput;
use crate::config::Config;
use crate::ws_client::GatewayClient;

pub async fn run_pre_tool(config_path: &Path, session_key: &str) -> ! {
    // Read tool info from stdin (synchronous — Claude Code pipes JSON here)
    let hook_input: HookInput = {
        let stdin = std::io::stdin();
        match serde_json::from_reader(stdin.lock()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("catclaw hook: failed to parse stdin: {}", e);
                std::process::exit(0); // Don't block on parse error
            }
        }
    };

    // Load config for port + token + agent rules
    let config = match Config::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("catclaw hook: failed to load config: {}", e);
            std::process::exit(0);
        }
    };

    // Parse session key: catclaw:{agent_id}:{origin}:{ctx}
    let parts: Vec<&str> = session_key.splitn(4, ':').collect();
    let agent_id = parts.get(1).copied().unwrap_or("");
    let origin = parts.get(2).copied().unwrap_or("");

    // System-originated sessions (cron, heartbeat) auto-approve — no human is waiting
    if origin == "system" {
        std::process::exit(0);
    }

    // Build approval config: tool lists from agent's tools.toml, timeout from catclaw.toml
    let approval = {
        let agent_config = config.agents.iter().find(|a| a.id == agent_id);
        let timeout_secs = agent_config.map(|a| a.approval.timeout_secs).unwrap_or(120);

        // Read require_approval and denied from tools.toml
        let tools_path = agent_config.map(|a| a.workspace.join("tools.toml"));
        let (require_approval, blocked) = if let Some(path) = tools_path {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(parsed) = toml::from_str::<toml::Value>(&content) {
                let ra = parsed.get("require_approval")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let bl = parsed.get("denied")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                (ra, bl)
            } else {
                (vec![], vec![])
            }
        } else {
            (vec![], vec![])
        };

        crate::config::ApprovalConfig {
            require_approval,
            blocked,
            timeout_secs,
        }
    };

    let tool = &hook_input.tool_name;

    // Check blocked list first
    if approval.is_blocked(tool) {
        eprintln!(
            "Tool '{}' is blocked by CatClaw policy for agent '{}'. Ask the user to update the agent's tool permissions.",
            tool, agent_id
        );
        std::process::exit(2);
    }

    // If no approval required, allow immediately
    if !approval.requires_approval(tool) {
        std::process::exit(0);
    }

    // Tool requires approval — connect to gateway and wait
    let approved = request_approval(&config, session_key, &hook_input, approval.timeout_secs).await;

    if approved {
        std::process::exit(0);
    } else {
        eprintln!(
            "Tool '{}' was not approved. The user can approve this in the TUI (Ctrl+A) or the originating channel.",
            tool
        );
        std::process::exit(2);
    }
}

async fn request_approval(
    config: &Config,
    session_key: &str,
    input: &HookInput,
    timeout_secs: u64,
) -> bool {
    let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);
    let token = &config.general.ws_token;

    let (client, mut event_rx) = match GatewayClient::connect(&ws_url, token).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("catclaw hook: could not connect to gateway ({}): approving by default", e);
            return true; // Fail open if gateway unreachable
        }
    };

    let request_id = uuid_v4();

    let params = serde_json::json!({
        "request_id": request_id,
        "session_key": session_key,
        "tool_name": input.tool_name,
        "tool_input": input.tool_input,
    });

    if let Err(e) = client.request("approval.request", params).await {
        eprintln!("catclaw hook: approval.request failed: {}", e);
        return true; // Fail open
    }

    // Wait for approval.result event with matching request_id
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(timeout_secs),
        async {
            while let Some(event) = event_rx.recv().await {
                if event.event == "approval.result" {
                    let rid = event.data.get("request_id").and_then(|v| v.as_str());
                    if rid == Some(request_id.as_str()) {
                        return event.data.get("approved")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                    }
                }
            }
            false
        }
    ).await;

    match result {
        Ok(approved) => approved,
        Err(_) => {
            eprintln!("catclaw hook: approval timed out after {}s — blocking tool", timeout_secs);
            false
        }
    }
}

/// Generate a simple UUID v4 without external dependencies.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    std::process::id().hash(&mut h);
    let a = h.finish();
    let mut h2 = DefaultHasher::new();
    a.hash(&mut h2);
    std::thread::current().id().hash(&mut h2);
    let b = h2.finish();

    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        a as u16 & 0x0fff,
        (b >> 48) as u16 | 0x8000,
        b & 0x0000_ffff_ffff_ffff,
    )
}
