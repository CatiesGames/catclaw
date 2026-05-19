//! Subscription status for the two runtimes catclaw drives.
//!
//! Two layers of truth:
//!
//! 1. **File-presence check** (fast, free): probe `claude auth status` and
//!    `codex login status` synchronously. Both commands are local-only
//!    (no API call, no token consumption) and return in <1 s.
//!
//! 2. **Real-failure record** (definitive, persisted): when an actual model
//!    call returns 401/403 / "Unauthorized" / "credit", `record_failure`
//!    writes a marker to `~/.catclaw/auth_status.json` that overrides the
//!    file-presence result. The next successful auth probe clears it.
//!
//! UI surfaces (TUI agent panel) call [`check_all`] to get the combined view.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::agent::Runtime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// `claude auth status` / `codex login status` reported a logged-in
    /// account AND no recent failure marker is on disk.
    Active,
    /// CLI reports not logged in. User needs `claude auth login` /
    /// `codex login` before this provider's models will work.
    NotLoggedIn,
    /// CLI says logged-in but a recent real model call failed with an
    /// auth-flavoured error (401 / "Unauthorized" / etc.). Token is likely
    /// expired or the plan was downgraded.
    Failed { reason: String, at_unix_ms: i64 },
    /// `claude` / `codex` binary not installed. The runtime is unreachable
    /// regardless of auth state.
    CliMissing,
    /// Auth probe itself errored unexpectedly (timeout, parse failure).
    Unknown { reason: String },
}

/// Subscription snapshot for one provider. Returned by [`check`] /
/// [`check_all`]. `account` / `plan` come from the CLI probe and are
/// surfaced in the TUI header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionStatus {
    pub provider: Runtime,
    pub state: AuthState,
    /// Display string for the logged-in account (e.g. email, "ChatGPT").
    /// None when not logged in or CLI doesn't expose it.
    pub account: Option<String>,
    /// Display string for the plan (e.g. "max", "pro"). None when unknown.
    pub plan: Option<String>,
    /// Unix millis when this check was performed.
    pub checked_at_unix_ms: i64,
}

/// On-disk record of recent real-call failures, keyed by provider.
///
/// Lives at `~/.catclaw/auth_status.json`. Idempotent — overwritten on
/// every write. Empty / missing file is fine and means "no recorded
/// failures".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FailureRecord {
    #[serde(default)]
    pub claude: Option<FailureEntry>,
    #[serde(default)]
    pub codex: Option<FailureEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureEntry {
    pub reason: String,
    pub at_unix_ms: i64,
}

fn auth_status_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".catclaw").join("auth_status.json")
}

fn now_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn read_failure_record() -> FailureRecord {
    let path = auth_status_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return FailureRecord::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn write_failure_record(rec: &FailureRecord) {
    let path = auth_status_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(rec) {
        let _ = std::fs::write(&path, json);
    }
}

/// Record an auth-style failure observed when a real model call ran (401 /
/// 403 / "Unauthorized" / "invalid api key" / "credit"). The TUI subscription
/// row will show this provider as `Failed` until the next successful probe.
#[allow(dead_code)]
pub fn record_failure(provider: Runtime, reason: impl Into<String>) {
    let mut rec = read_failure_record();
    let entry = FailureEntry {
        reason: reason.into(),
        at_unix_ms: now_unix_ms(),
    };
    match provider {
        Runtime::Claude => rec.claude = Some(entry),
        Runtime::Codex => rec.codex = Some(entry),
    }
    write_failure_record(&rec);
}

/// Clear a provider's failure marker (call after a successful real model
/// call so the TUI row goes back to `Active`).
#[allow(dead_code)]
pub fn clear_failure(provider: Runtime) {
    let mut rec = read_failure_record();
    match provider {
        Runtime::Claude => rec.claude = None,
        Runtime::Codex => rec.codex = None,
    }
    write_failure_record(&rec);
}

/// Probe one provider. Spawns the CLI sync (subprocess returns fast, < 1 s).
/// Combines the CLI result with any persisted failure marker.
pub fn check(provider: Runtime) -> SubscriptionStatus {
    let now = now_unix_ms();
    let (account, plan, state) = match provider {
        Runtime::Claude => probe_claude(),
        Runtime::Codex => probe_codex(),
    };

    // Layer 2: persisted failure marker overrides Active state.
    let state = if matches!(state, AuthState::Active) {
        let rec = read_failure_record();
        let failure = match provider {
            Runtime::Claude => rec.claude.as_ref(),
            Runtime::Codex => rec.codex.as_ref(),
        };
        match failure {
            Some(f) => AuthState::Failed {
                reason: f.reason.clone(),
                at_unix_ms: f.at_unix_ms,
            },
            None => state,
        }
    } else {
        state
    };

    SubscriptionStatus {
        provider,
        state,
        account,
        plan,
        checked_at_unix_ms: now,
    }
}

/// Probe both providers. Returns `(claude_status, codex_status)`.
pub fn check_all() -> (SubscriptionStatus, SubscriptionStatus) {
    (check(Runtime::Claude), check(Runtime::Codex))
}

fn probe_claude() -> (Option<String>, Option<String>, AuthState) {
    let output = std::process::Command::new("claude")
        .args(["auth", "status"])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (None, None, AuthState::CliMissing);
        }
        Err(e) => {
            return (
                None,
                None,
                AuthState::Unknown {
                    reason: format!("failed to run `claude auth status`: {}", e),
                },
            );
        }
    };
    if !output.status.success() {
        // Non-zero exit usually means "not logged in".
        return (None, None, AuthState::NotLoggedIn);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => {
            return (
                None,
                None,
                AuthState::Unknown {
                    reason: format!(
                        "could not parse `claude auth status` JSON (got: {})",
                        stdout.trim().chars().take(80).collect::<String>()
                    ),
                },
            );
        }
    };
    let logged_in = parsed
        .get("loggedIn")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !logged_in {
        return (None, None, AuthState::NotLoggedIn);
    }
    let account = parsed
        .get("email")
        .and_then(|v| v.as_str())
        .map(String::from);
    let plan = parsed
        .get("subscriptionType")
        .and_then(|v| v.as_str())
        .map(String::from);
    (account, plan, AuthState::Active)
}

fn probe_codex() -> (Option<String>, Option<String>, AuthState) {
    let output = std::process::Command::new("codex")
        .args(["login", "status"])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (None, None, AuthState::CliMissing);
        }
        Err(e) => {
            return (
                None,
                None,
                AuthState::Unknown {
                    reason: format!("failed to run `codex login status`: {}", e),
                },
            );
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Codex outputs free-form text — examples:
    //   "Logged in using ChatGPT"
    //   "Logged in with API key (env: OPENAI_API_KEY)"
    //   "Not logged in"
    if stdout.starts_with("Logged in") {
        // Best-effort account extraction: everything after "using ".
        let account = stdout
            .strip_prefix("Logged in")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // Codex doesn't expose a "plan" concept the way Claude does.
        return (account, None, AuthState::Active);
    }
    if stdout.contains("Not logged in") || !output.status.success() {
        return (None, None, AuthState::NotLoggedIn);
    }
    (
        None,
        None,
        AuthState::Unknown {
            reason: format!("unrecognised `codex login status` output: {}", stdout),
        },
    )
}
