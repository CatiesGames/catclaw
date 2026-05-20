//! Build `codex exec` CLI args for an agent + [`SpawnParams`].
//!
//! Codex shares zero CLI conventions with `claude -p`, so this builder is
//! independent from `claude_args_with_mcp`. The two builders MUST stay in
//! sync on the "what catclaw exposes to the model" front though — same
//! system prompt, same MCP servers, same model — only the wire format
//! differs.
//!
//! ## Isolation strategy
//!
//! Each codex agent points `CODEX_HOME` at a per-agent `.codex-home/`
//! directory inside its workspace. With `--ignore-user-config` +
//! `--ignore-rules` + `project_doc_max_bytes=0`, codex is fully detached
//! from the user's `~/.codex/` config, AGENTS.md auto-scan, and any
//! pre-existing project rules.
//!
//! ## First-turn vs resume
//!
//! First turn (`is_resume = false`) passes the system prompt via
//! `-c developer_instructions=...`. Codex binds the prompt to the thread —
//! [PoC verified][1] subsequent `codex exec resume` invocations ignore any
//! new `developer_instructions` value (the original sticks). So on resume
//! we omit it entirely, saving ~5–8K tokens of CLI plumbing per turn.
//!
//! ## Third-party MCP servers
//!
//! User's workspace `.mcp.json` is read the same way Claude reads it, and
//! each server (except the literal `catclaw` key, which we own) is
//! injected as a per-server `-c mcp_servers.NAME.*` flag set. Both stdio
//! and streamable_http transports are supported.
//!
//! [1]: ../../tasks/codex-runtime-plan.md "Codex runtime plan"

use std::collections::HashMap;
use std::path::PathBuf;

use crate::session::runtime::SpawnParams;

/// Where the per-agent isolated CODEX_HOME lives inside the workspace.
/// Caller must ensure this directory exists with the right `auth.json`
/// symlink before spawning (see `agent/loader.rs` Phase B.2.3).
#[allow(dead_code)]
pub fn codex_home_for(workspace: &std::path::Path) -> PathBuf {
    workspace.join(".codex-home")
}

/// Build the codex CLI args for this agent + spawn parameters.
///
/// Does NOT include the prompt itself — the caller passes it via
/// `CodexHandle::spawn_with_prompt` (first turn, as the trailing positional)
/// or `spawn_resume_with_prompt` (resume, via stdin).
#[allow(dead_code)]
pub fn codex_args_from(agent: &super::Agent, params: &SpawnParams<'_>) -> Vec<String> {
    let mut args: Vec<String> = vec!["exec".to_string()];

    if params.is_resume {
        // codex exec resume <thread_id> --json [-c flags...] -
        // The trailing `-` tells codex to read the new prompt from stdin.
        let thread_id = params
            .resume_thread_id
            .or(Some(params.session_id))
            .unwrap_or("");
        args.push("resume".to_string());
        args.push(thread_id.to_string());
    }

    args.push("--json".to_string());
    args.push("--skip-git-repo-check".to_string());
    args.push("--ignore-user-config".to_string());
    args.push("--ignore-rules".to_string());

    // Workspace = -C <agent.workspace>
    args.push("-C".to_string());
    args.push(agent.workspace.display().to_string());

    // Disable AGENTS.md auto-scan (Phase B verified — only way to keep codex
    // from injecting whatever AGENTS.md it finds upward of the workspace).
    args.push("-c".to_string());
    args.push("project_doc_max_bytes=0".to_string());

    // Model — codex `model` config key, matching Claude's `--model`.
    let effective_model = params.model_override.or(agent.model.as_deref());
    if let Some(model) = effective_model {
        let resolved = super::models::resolve_model(model);
        args.push("-c".to_string());
        args.push(format!("model={}", toml_quote(&resolved)));
    }

    // Sandbox + approval policy. catclaw approval-gate fires at the MCP
    // intercept layer; we explicitly tell codex never to ask for in-process
    // approvals (those would hang an `exec` run since it has no UI).
    args.push("-c".to_string());
    args.push("approval_policy=\"never\"".to_string());
    args.push("-c".to_string());
    args.push("sandbox_mode=\"workspace-write\"".to_string());

    // Image generation (gpt-image-2) — auto-on for every codex agent, the same
    // way codex's built-in `imagegen` skill is always available. ChatGPT login
    // covers it (no OPENAI_API_KEY needed). Equivalent to `--enable
    // image_generation`. Output lands in `.codex-home/generated_images/`; the
    // agent moves it into the workspace and uploads via `{platform}_upload_file`
    // (see the "Image generation" note in CODEX_RUNTIME_OVERRIDES). Inline `-c`
    // overrides are honoured even under `--ignore-user-config`.
    args.push("-c".to_string());
    args.push("features.image_generation=true".to_string());

    // Developer instructions only on first turn — codex thread-binds them
    // (PoC verified resume can't change the original).
    if !params.is_resume {
        let system_prompt = agent.build_system_prompt(params.state_db);
        if !system_prompt.is_empty() {
            args.push("-c".to_string());
            args.push(format!(
                "developer_instructions={}",
                toml_quote(&system_prompt)
            ));
        }
    }

    // catclaw built-in MCP server (HTTP transport) — shares the same Axum
    // endpoint that Claude uses, just over a different MCP client.
    if let Some(port) = params.mcp_port {
        args.push("-c".to_string());
        args.push(format!(
            "mcp_servers.catclaw.url={}",
            toml_quote(&format!("http://127.0.0.1:{}/mcp", port))
        ));
        // We never want codex's own "ask before each MCP tool call" prompt
        // to fire — catclaw's MCP intercept handles approval itself.
        args.push("-c".to_string());
        args.push("mcp_servers.catclaw.default_tools_approval_mode=\"approve\"".to_string());
    }

    // Third-party MCP servers from workspace .mcp.json (gap A).
    inject_user_mcp_servers(&mut args, &agent.workspace_root, params.mcp_env);

    // Resume invocations end with `-` so codex reads the new prompt from stdin.
    // CodexHandle::spawn_resume_with_prompt pipes the prompt into stdin and
    // closes it; codex sees EOF and finalises the turn. First-turn spawns
    // pass the prompt as a positional argument (appended by spawn_with_prompt
    // itself), so no `-` is needed there.
    if params.is_resume {
        args.push("-".to_string());
    }

    args
}

/// Read `<workspace_root>/.mcp.json` and emit `-c mcp_servers.NAME.*` flags
/// for every server (except the reserved `catclaw` name).
///
/// Supports two MCP transport shapes:
/// - `{"command": "...", "args": [...], "env": {...}}` → stdio
/// - `{"type": "http"|"streamable_http", "url": "..."}` → HTTP
///
/// Unknown shapes are skipped with a warn — codex can still launch with the
/// rest of the servers wired up.
#[allow(dead_code)]
fn inject_user_mcp_servers(
    args: &mut Vec<String>,
    workspace_root: &std::path::Path,
    mcp_env: &HashMap<String, HashMap<String, String>>,
) {
    let path = workspace_root.join(".mcp.json");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?path, %e, "invalid .mcp.json — skipping third-party MCP servers for codex");
            return;
        }
    };
    let servers = match parsed.get("mcpServers").and_then(|v| v.as_object()) {
        Some(s) => s,
        None => return,
    };

    for (name, def) in servers {
        if name == "catclaw" {
            // catclaw is already injected in codex_args_from above.
            continue;
        }
        let server_type = def.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let has_url = def.get("url").is_some();
        let has_command = def.get("command").is_some();

        if has_url || matches!(server_type, "http" | "streamable_http") {
            if let Some(url) = def.get("url").and_then(|u| u.as_str()) {
                args.push("-c".to_string());
                args.push(format!("mcp_servers.{}.url={}", name, toml_quote(url)));
            } else {
                tracing::warn!(server = %name, "http MCP server missing url; skipping for codex");
                continue;
            }
        } else if has_command {
            if let Some(cmd) = def.get("command").and_then(|c| c.as_str()) {
                args.push("-c".to_string());
                args.push(format!("mcp_servers.{}.command={}", name, toml_quote(cmd)));
            }
            if let Some(arr) = def.get("args").and_then(|a| a.as_array()) {
                let inline = arr
                    .iter()
                    .map(|v| toml_quote(v.as_str().unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join(", ");
                args.push("-c".to_string());
                args.push(format!("mcp_servers.{}.args=[{}]", name, inline));
            }
            // Merge env: .mcp.json "env" + per-server mcp_env override.
            let mut combined_env: HashMap<String, String> = HashMap::new();
            if let Some(env_obj) = def.get("env").and_then(|v| v.as_object()) {
                for (k, v) in env_obj {
                    if let Some(s) = v.as_str() {
                        combined_env.insert(k.clone(), s.to_string());
                    }
                }
            }
            if let Some(env_map) = mcp_env.get(name) {
                for (k, v) in env_map {
                    combined_env.insert(k.clone(), v.clone());
                }
            }
            for (k, v) in &combined_env {
                args.push("-c".to_string());
                args.push(format!(
                    "mcp_servers.{}.env.{}={}",
                    name,
                    k,
                    toml_quote(v)
                ));
            }
        } else {
            tracing::warn!(server = %name, "unknown MCP server shape in .mcp.json — skipping for codex");
            continue;
        }

        // Same opt-out as catclaw — never have codex's UI gate fire, our own
        // intercept handles approval where it's needed.
        args.push("-c".to_string());
        args.push(format!(
            "mcp_servers.{}.default_tools_approval_mode=\"approve\"",
            name
        ));
    }
}

/// Escape a string for use as a TOML literal value in a `-c key=VALUE` arg.
/// Codex parses `VALUE` as TOML; double-quotes + escape `"` and `\`.
fn toml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}
