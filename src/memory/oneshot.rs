//! One-shot inference helper used by diary generation + diary post-analysis.
//!
//! These are catclaw-internal "fast / cheap" model calls — not agent
//! conversations. The shared shape is: build a fully-formed prompt, spawn
//! the appropriate CLI with `--max-turns 1` (Claude) or one `codex exec`
//! turn (Codex), no MCP servers, no tools, no skills. Returns the model's
//! text reply as a `String`.
//!
//! `model_str` is a canonical `provider/model` (e.g. `claude/haiku-4-5`,
//! `codex/gpt-5.4-mini`). The Runtime is read from the parsed provider so
//! a single config key (`general.diary_model`) selects both the binary
//! and the model.

use crate::agent::models::{parse_model_string, ProviderModel};
use crate::agent::Runtime;
use crate::error::{CatClawError, Result};

/// Default model used when no `general.diary_model` is configured. Picks the
/// cheapest Claude tier so existing installs (without the new key) keep
/// behaving as they always did.
const DEFAULT_DIARY_MODEL: &str = "claude/haiku-4-5";

/// Process-wide snapshot of the diary model, installed by gateway startup via
/// [`install_diary_model`]. Read by [`current_diary_model`] from
/// `memory::analyze::call_haiku` and `scheduler::run_diary` so they don't
/// have to thread a `Config` reference through every caller in between.
///
/// Hot-reload: `config.set general.diary_model = ...` calls `install_diary_model`
/// again on the change-apply path, so the next diary extraction tick picks
/// up the new value without a restart.
static CURRENT_DIARY_MODEL: std::sync::RwLock<Option<ProviderModel>> =
    std::sync::RwLock::new(None);

/// Install (or re-install) the diary model snapshot. Idempotent.
#[allow(dead_code)] // wired in gateway::start + config.set hot-reload
pub fn install_diary_model(config_diary_model: Option<&str>) {
    let model = resolve_diary_model(config_diary_model);
    *CURRENT_DIARY_MODEL.write().unwrap() = Some(model);
}

/// Snapshot of the currently-active diary model. Returns the hard-coded
/// default when [`install_diary_model`] was never called (which happens
/// in non-gateway code paths and unit tests).
pub fn current_diary_model() -> ProviderModel {
    if let Some(model) = CURRENT_DIARY_MODEL.read().unwrap().clone() {
        return model;
    }
    resolve_diary_model(None)
}

/// Resolve which model to use for catclaw-internal background analysis.
/// Reads `general.diary_model` from config; falls back to claude haiku.
/// Returns a parsed [`ProviderModel`] so callers can dispatch by provider.
fn resolve_diary_model(config_diary_model: Option<&str>) -> ProviderModel {
    let raw = config_diary_model
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_DIARY_MODEL);
    parse_model_string(raw).unwrap_or_else(|_| {
        // Unparseable config — fall back to the hard-coded default rather
        // than break diary extraction. Caller already validated input on
        // `config.set`; this is defence in depth.
        parse_model_string(DEFAULT_DIARY_MODEL).expect("DEFAULT_DIARY_MODEL must parse")
    })
}

/// Run a single-turn inference call against the chosen provider and return
/// the model's text response (already trimmed).
///
/// On Claude this spawns `claude -p ... --model X --max-turns 1` with no
/// MCP / no tools. On Codex it spawns `codex exec --json -c model=X ...`
/// in a fully isolated CODEX_HOME (no project AGENTS.md, no MCP), then
/// extracts the final `agent_message` from the NDJSON stream.
///
/// Errors are surfaced as `CatClawError::Memory` so existing diary call
/// sites keep their error type unchanged.
pub async fn run_oneshot_inference(
    model: &ProviderModel,
    prompt: &str,
) -> Result<String> {
    match model.provider {
        Runtime::Claude => run_claude(prompt, &model.model).await,
        Runtime::Codex => run_codex(prompt, &model.model).await,
    }
}

async fn run_claude(prompt: &str, model: &str) -> Result<String> {
    let result = tokio::process::Command::new("claude")
        .args([
            "-p",
            prompt,
            "--model",
            model,
            "--max-turns",
            "1",
            "--output-format",
            "text",
            "--dangerously-skip-permissions",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CLAUDECODE")
        .output()
        .await;

    let output = result.map_err(|e| {
        CatClawError::Memory(format!("failed to spawn claude (model={}): {}", model, e))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Surface auth-style failures so subscription.rs can mark this
        // provider as Failed (TUI flips to ⚠️).
        if is_auth_failure(&stderr) {
            crate::subscription::record_failure(
                Runtime::Claude,
                format!("oneshot inference returned auth error: {}", trim_ascii(&stderr, 200)),
            );
        }
        return Err(CatClawError::Memory(format!(
            "claude exited with {}: {}",
            output.status, stderr
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Successful call clears any prior failure marker for this provider so
    // the TUI subscription row recovers to ✓ automatically.
    crate::subscription::clear_failure(Runtime::Claude);
    Ok(text)
}

async fn run_codex(prompt: &str, model: &str) -> Result<String> {
    // codex exec --json prints NDJSON; we need to extract the final
    // `agent_message` text. Build an isolated invocation: no workspace MCP,
    // no AGENTS.md, no rules.
    let result = tokio::process::Command::new("codex")
        .args([
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--ignore-rules",
            "-c",
            "project_doc_max_bytes=0",
            "-c",
            &format!("model={}", toml_quote(model)),
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "sandbox_mode=\"read-only\"",
            prompt,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("CODEX_API_KEY")
        .output()
        .await;

    let output = result.map_err(|e| {
        CatClawError::Memory(format!("failed to spawn codex (model={}): {}", model, e))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_auth_failure(&stderr) {
            crate::subscription::record_failure(
                Runtime::Codex,
                format!("oneshot inference returned auth error: {}", trim_ascii(&stderr, 200)),
            );
        }
        return Err(CatClawError::Memory(format!(
            "codex exited with {}: {}",
            output.status, stderr
        )));
    }

    // Parse NDJSON, accumulate the last `agent_message` text from
    // `item.completed` events.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut last_message = String::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("item.completed") {
            continue;
        }
        let Some(item) = v.get("item") else {
            continue;
        };
        if item.get("type").and_then(|t| t.as_str()) != Some("agent_message") {
            continue;
        }
        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
            last_message = text.to_string();
        }
    }
    if last_message.is_empty() {
        return Err(CatClawError::Memory(
            "codex returned no agent_message in NDJSON output".to_string(),
        ));
    }
    crate::subscription::clear_failure(Runtime::Codex);
    Ok(last_message.trim().to_string())
}

/// Heuristic: does this stderr / stdout fragment look like an
/// authentication-related failure (vs a transient API error)?
///
/// Used both by [`run_oneshot_inference`] (catches its own subprocess'
/// stderr) and by `session::claude` / `session::codex` stderr readers
/// (catches running-agent subprocesses) so a real 401/403 anywhere in
/// catclaw lands a `Failed` marker for the TUI subscription row.
pub(crate) fn is_auth_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("403")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("auth")
            && (lower.contains("expired") || lower.contains("failed"))
}

fn trim_ascii(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn toml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}
