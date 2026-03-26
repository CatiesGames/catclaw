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

    // Social publish tools: submit draft for human review then exit immediately
    // so the agent session is released (no blocking wait).
    const SOCIAL_PUBLISH_TOOLS: &[&str] = &[
        "mcp__catclaw__instagram_reply_comment",
        "mcp__catclaw__instagram_create_post",
        "mcp__catclaw__instagram_send_dm",
        "mcp__catclaw__threads_reply",
        "mcp__catclaw__threads_create_post",
    ];
    if SOCIAL_PUBLISH_TOOLS.contains(&tool.as_str()) {
        submit_draft_for_approval(&config, session_key, &hook_input).await;
        // never returns
    }

    // Tool requires approval — connect to gateway and wait
    let approved = request_approval(&config, session_key, &hook_input, approval.timeout_secs).await;

    if approved {
        // hookSpecificOutput JSON: additionalContext is added to Claude's context
        // See: https://docs.anthropic.com/en/docs/claude-code/hooks
        println!(
            "{}",
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "permissionDecisionReason": format!("Approved by user (CatClaw approval system)"),
                    "additionalContext": format!(
                        "Tool '{}' was approved by the user through CatClaw's approval system. The user reviewed the tool call and explicitly approved it.",
                        tool
                    )
                }
            })
        );
        std::process::exit(0);
    } else {
        // hookSpecificOutput with deny: permissionDecisionReason is shown to Claude
        println!(
            "{}",
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": format!(
                        "Tool '{}' was explicitly denied by the user through CatClaw's approval system. Do not retry this tool call without asking the user first.",
                        tool
                    )
                }
            })
        );
        std::process::exit(0);
    }
}

/// Submit a social draft for human approval, then exit(0) with a structured deny.
///
/// Uses `permissionDecision: "deny"` so Claude Code respects the decision at the
/// runtime level and does not retry. `exit(2)` would discard stdout and only feed
/// stderr as a raw error string — Claude may treat that as a transient failure.
async fn submit_draft_for_approval(
    config: &Config,
    session_key: &str,
    input: &HookInput,
) -> ! {
    let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);
    let token = &config.general.ws_token;

    let draft_id = match GatewayClient::connect(&ws_url, token).await {
        Ok((client, _rx)) => {
            let params = serde_json::json!({
                "tool_name": input.tool_name,
                "tool_input": input.tool_input,
                "session_key": session_key,
            });
            match client.request("social.draft.submit_for_approval", params).await {
                Ok(resp) => resp.get("draft_id").and_then(|v| v.as_i64()),
                Err(e) => {
                    eprintln!("catclaw hook: social.draft.submit_for_approval failed: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("catclaw hook: could not connect to gateway ({})", e);
            None
        }
    };

    let reason = if let Some(id) = draft_id {
        format!(
            "This social publish tool call has been intercepted by CatClaw. \
             Your draft (draft_id: {}) is now queued for human approval. \
             A human will review and publish it via the admin channel. \
             Do NOT retry this tool call or call any further publish tools for this content. \
             The task is complete — move on to the next task.",
            id
        )
    } else {
        "This social publish tool call has been intercepted by CatClaw. \
         Your draft is now queued for human approval. \
         A human will review and publish it via the admin channel. \
         Do NOT retry this tool call or call any further publish tools for this content. \
         The task is complete — move on to the next task."
            .to_string()
    };

    println!(
        "{}",
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason
            }
        })
    );
    std::process::exit(0);
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

    let request_id = uuid::Uuid::new_v4().to_string();

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

