use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{CatClawError, Result};

// ── Social config ────────────────────────────────────────────────────────────

/// Top-level social platforms config (Instagram + Threads).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SocialConfig {
    #[serde(default)]
    pub instagram: Option<InstagramConfig>,
    #[serde(default)]
    pub threads: Option<ThreadsConfig>,
}

fn default_social_mode() -> String { "off".to_string() }
fn default_poll_interval() -> u64 { 5 }
fn default_social_agent() -> String { "main".to_string() }
fn default_ig_subscribe() -> Vec<String> {
    vec!["comments".to_string(), "mentions".to_string()]
}
fn default_threads_subscribe() -> Vec<String> {
    vec!["replies".to_string(), "mentions".to_string()]
}

/// Instagram Graph API integration config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstagramConfig {
    /// Receive mode: "webhook" | "polling" | "off"
    #[serde(default = "default_social_mode")]
    pub mode: String,

    /// Polling interval in minutes (only when mode = "polling")
    #[serde(default = "default_poll_interval")]
    pub poll_interval_mins: u64,

    /// Env var holding the Instagram System User Access Token
    pub token_env: String,

    /// App ID (client_id) for short-lived → long-lived token exchange
    #[serde(default)]
    pub app_id: Option<String>,

    /// Env var holding the App Secret (for webhook HMAC-SHA256 signature verification)
    #[serde(default)]
    pub app_secret_env: Option<String>,

    /// Env var holding the webhook verify token (set when subscribing the webhook in Meta dashboard)
    #[serde(default)]
    pub webhook_verify_token_env: Option<String>,

    /// Instagram User ID (from Meta Business Manager)
    pub user_id: String,

    /// Admin channel for forward cards and draft review.
    /// Format: "discord:channel:<channel_id>" | "telegram:chat:<chat_id>" | "slack:channel:<channel_id>"
    pub admin_channel: String,

    /// Webhook event fields to subscribe to.
    /// Possible values: "comments", "mentions", "messages"
    #[serde(default = "default_ig_subscribe")]
    pub subscribe: Vec<String>,

    /// Agent to use for auto_reply sessions
    #[serde(default = "default_social_agent")]
    pub agent: String,

    /// Action rules — evaluated top-to-bottom, first match wins.
    /// If no rule matches, the event is ignored.
    #[serde(default)]
    pub rules: Vec<SocialRule>,

    /// Named reply templates used by auto_reply_template rules.
    #[serde(default)]
    pub templates: HashMap<String, String>,
}

/// Threads API integration config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadsConfig {
    /// Receive mode: "webhook" | "polling" | "off"
    #[serde(default = "default_social_mode")]
    pub mode: String,

    /// Polling interval in minutes (only when mode = "polling")
    #[serde(default = "default_poll_interval")]
    pub poll_interval_mins: u64,

    /// Env var holding the Threads OAuth access token (60-day long-lived)
    pub token_env: String,

    /// App ID (client_id) for short-lived → long-lived token exchange
    #[serde(default)]
    pub app_id: Option<String>,

    /// Env var holding the App Secret (for webhook HMAC-SHA256 signature verification)
    #[serde(default)]
    pub app_secret_env: Option<String>,

    /// Env var holding the webhook verify token (set when subscribing the webhook in Meta dashboard)
    #[serde(default)]
    pub webhook_verify_token_env: Option<String>,

    /// Threads User ID
    pub user_id: String,

    /// Admin channel for forward cards and draft review.
    /// Format: "discord:channel:<channel_id>" | "telegram:chat:<chat_id>" | "slack:channel:<channel_id>"
    pub admin_channel: String,

    /// Webhook event fields to subscribe to.
    /// Possible values: "replies", "mentions"
    #[serde(default = "default_threads_subscribe")]
    pub subscribe: Vec<String>,

    /// Agent to use for auto_reply sessions
    #[serde(default = "default_social_agent")]
    pub agent: String,

    /// Action rules — evaluated top-to-bottom, first match wins.
    #[serde(default)]
    pub rules: Vec<SocialRule>,

    /// Named reply templates used by auto_reply_template rules.
    #[serde(default)]
    pub templates: HashMap<String, String>,
}

/// A single action rule for social event routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialRule {
    /// Event type to match: "comments" | "mentions" | "messages" | "replies" | "*"
    #[serde(rename = "match")]
    pub match_type: String,

    /// Optional substring keyword filter (matches against event text, case-insensitive)
    #[serde(default)]
    pub keyword: Option<String>,

    /// Action to take: "forward" | "auto_reply" | "auto_reply_template" | "ignore"
    pub action: String,

    /// Template key (only for action = "auto_reply_template")
    #[serde(default)]
    pub template: Option<String>,

    /// Override the platform-level agent for this rule (only for action = "auto_reply")
    #[serde(default)]
    pub agent: Option<String>,
}

/// Parse an admin_channel string into (adapter_name, channel_id).
/// Format: "<adapter>:channel:<id>" or "<adapter>:chat:<id>"
/// e.g. "discord:channel:123456" -> ("discord", "123456")
///      "telegram:chat:987654"   -> ("telegram", "987654")
///      "slack:channel:CABCDEF"  -> ("slack", "CABCDEF")
pub fn parse_admin_channel(s: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() == 3 {
        Some((parts[0].to_string(), parts[2].to_string()))
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,

    #[serde(default)]
    pub channels: Vec<ChannelConfig>,

    #[serde(default)]
    pub agents: Vec<AgentConfig>,

    #[serde(default)]
    pub bindings: Vec<BindingConfig>,

    #[serde(default)]
    pub collaboration: CollaborationConfig,

    #[serde(default)]
    pub embedding: Option<EmbeddingConfig>,

    #[serde(default)]
    pub memory: Option<MemoryConfig>,

    #[serde(default)]
    pub heartbeat: Option<HeartbeatConfig>,

    #[serde(default)]
    pub logging: LoggingConfig,

    /// Per-MCP-server environment variables (secrets).
    /// Keys are server names from .mcp.json, values are key=value env vars.
    /// Example in TOML:
    /// ```toml
    /// [mcp_env.dotdot]
    /// DOTDOT_API_KEY = "sk-xxx"
    /// ```
    #[serde(default)]
    pub mcp_env: HashMap<String, HashMap<String, String>>,

    /// Environment variables injected into claude subprocesses.
    /// These are set as OS-level env vars on the spawned process,
    /// accessible by any tool (Bash, etc.) the agent uses.
    /// Example in TOML:
    /// ```toml
    /// [env]
    /// OP_SERVICE_ACCOUNT_TOKEN = "ops_xxx"
    /// ```
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Social inbox integration (Instagram + Threads).
    #[serde(default)]
    pub social: SocialConfig,

    /// Contacts subsystem (cross-platform identity, forward/approval pipeline).
    /// Disabled by default — when off, contacts_* MCP tools are not advertised
    /// to agents (saves ~3-4KB tokens per conversation start). Inbound
    /// contact-binding lookup + manual-reply detection still works because
    /// they're cheap, but the schema/CRUD remain available for explicit CLI/TUI
    /// use. Flip on when the user actually manages clients via the bot.
    #[serde(default)]
    pub contacts: ContactsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactsConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Forward channel for messages from unknown / unclassified contacts
    /// (e.g. LINE users who just added the OA but haven't been promoted to
    /// client/admin yet). Format same as contact.forward_channel:
    /// "platform:channel_id" or "platform:guild_id/channel_id".
    /// When unset, unknown inbound is only logged (`info!`); the sender is
    /// still auto-registered in the contacts table for later review via
    /// TUI Contacts (role=unknown filter) or `catclaw contact list --role unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_inbox_channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,

    #[serde(default = "default_state_db")]
    pub state_db: PathBuf,

    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sessions: usize,

    #[serde(default = "default_idle_timeout")]
    pub session_idle_timeout_mins: u64,

    #[serde(default = "default_archive_timeout")]
    pub session_archive_timeout_hours: u64,

    /// Port for the gateway server (WS + MCP share this port, default: 21130).
    #[serde(default = "default_ws_port", alias = "ws_port")]
    pub port: u16,

    /// Bind address for the gateway server (default: "0.0.0.0").
    /// Use "127.0.0.1" to restrict access to localhost only.
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    /// Enable streaming mode for TUI/WebUI (default: true).
    /// When false, TUI waits for complete response before displaying.
    #[serde(default = "default_streaming")]
    pub streaming: bool,

    /// Default model for all agents (short name or full ID).
    #[serde(default)]
    pub default_model: Option<String>,

    /// Default fallback model when primary is overloaded.
    #[serde(default)]
    pub default_fallback_model: Option<String>,

    /// Token for authenticating internal WS connections (TUI/CLI → gateway).
    /// Auto-generated on first run; stored in config so all internal clients share it.
    /// External processes do not have access to this file, providing local auth.
    #[serde(default)]
    pub ws_token: String,

    /// IANA timezone for interpreting naive times in `--at` (e.g. "Asia/Taipei").
    /// Falls back to system local timezone if not set.
    #[serde(default)]
    pub timezone: Option<String>,

    /// Public base URL for webhook callbacks (e.g. "https://myserver.com").
    /// Used to display the full webhook URL in TUI/CLI after setting mode=webhook.
    /// If not set, falls back to "http://localhost:{port}".
    #[serde(default)]
    pub webhook_base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(rename = "type")]
    pub channel_type: String,

    pub token_env: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guilds: Vec<String>,

    #[serde(default = "default_activation", skip_serializing_if = "is_default_activation")]
    pub activation: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ChannelOverride>,

    /// DM policy: "open" (default), "allowlist", or "disabled"
    #[serde(default = "default_dm_policy", skip_serializing_if = "is_default_dm_policy")]
    pub dm_policy: String,

    /// Sender IDs allowed to DM (only used when dm_policy = "allowlist")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dm_allow: Vec<String>,

    /// Sender IDs denied from DM (checked before allow; works in any policy)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dm_deny: Vec<String>,

    /// Group policy: "open" (default) or "allowlist"
    #[serde(default = "default_group_policy", skip_serializing_if = "is_default_group_policy")]
    pub group_policy: String,

    /// Sender IDs allowed in groups (only used when group_policy = "allowlist")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_allow: Vec<String>,

    /// Sender IDs denied in groups (checked before allow; works in any policy)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_deny: Vec<String>,

    /// Environment variable name for the app-level token (Slack Socket Mode only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_token_env: Option<String>,

    /// Environment variable name for the channel signing secret.
    /// Currently used by LINE adapter for webhook HMAC verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelOverride {
    pub pattern: String,
    pub activation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub workspace: PathBuf,

    #[serde(default)]
    pub default: bool,

    /// Model for this agent (overrides general default_model).
    #[serde(default)]
    pub model: Option<String>,

    /// Fallback model for this agent.
    #[serde(default)]
    pub fallback_model: Option<String>,

    /// Tool approval rules for this agent.
    #[serde(default)]
    pub approval: ApprovalConfig,
}

/// Tool approval rules for an agent.
/// Controls which tool calls require user confirmation before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalConfig {
    /// Tools that require explicit user approval before each execution.
    /// Supports simple name ("Bash") or wildcard patterns ("Bash*", "*").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_approval: Vec<String>,

    /// Tools that are unconditionally blocked — agent will be told it lacks permission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<String>,

    /// Seconds to wait for a user approval before auto-denying (default: 120).
    #[serde(default = "default_approval_timeout")]
    #[serde(skip_serializing_if = "is_default_approval_timeout")]
    pub timeout_secs: u64,
}

fn is_default_approval_timeout(v: &u64) -> bool { *v == 120 || *v == 0 }

impl Default for ApprovalConfig {
    fn default() -> Self {
        ApprovalConfig {
            require_approval: Vec::new(),
            blocked: Vec::new(),
            timeout_secs: default_approval_timeout(),
        }
    }
}

fn default_approval_timeout() -> u64 { 120 }

impl ApprovalConfig {
    pub fn is_empty(&self) -> bool {
        self.require_approval.is_empty() && self.blocked.is_empty()
    }

    /// Check if a tool name matches a pattern (supports "*" wildcard).
    pub fn matches_pattern(pattern: &str, tool: &str) -> bool {
        if pattern == "*" { return true; }
        if let Some(prefix) = pattern.strip_suffix('*') {
            return tool.starts_with(prefix);
        }
        pattern == tool
    }

    pub fn is_blocked(&self, tool: &str) -> bool {
        self.blocked.iter().any(|p| Self::matches_pattern(p, tool))
    }

    pub fn requires_approval(&self, tool: &str) -> bool {
        self.require_approval.iter().any(|p| Self::matches_pattern(p, tool))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingConfig {
    pub pattern: String,
    pub agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollaborationConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_provider")]
    pub provider: String,

    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    #[serde(default = "default_embedding_model")]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_chunk_size")]
    pub chunk_size_tokens: usize,

    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap_tokens: usize,

    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,

    #[serde(default = "default_bm25_weight")]
    pub bm25_weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_heartbeat_interval")]
    pub interval_mins: u64,
}

// Default functions
fn catclaw_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".catclaw")
}
fn default_workspace() -> PathBuf {
    catclaw_home().join("workspace")
}
fn default_state_db() -> PathBuf {
    catclaw_home().join("state.sqlite")
}
fn default_max_concurrent() -> usize {
    3
}
fn default_idle_timeout() -> u64 {
    30
}
fn default_archive_timeout() -> u64 {
    168 // 7 days
}
fn default_ws_port() -> u16 {
    21130
}
fn default_bind_addr() -> String {
    "0.0.0.0".to_string()
}
fn default_streaming() -> bool {
    true
}
fn default_activation() -> String {
    "mention".to_string()
}
fn is_default_activation(v: &str) -> bool {
    v == "mention"
}
fn default_dm_policy() -> String {
    "open".to_string()
}
fn is_default_dm_policy(v: &str) -> bool {
    v == "open"
}
fn default_group_policy() -> String {
    "open".to_string()
}
fn is_default_group_policy(v: &str) -> bool {
    v == "open"
}
fn default_embedding_provider() -> String {
    "ollama".to_string()
}
fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}
fn default_embedding_model() -> String {
    "nomic-embed-text".to_string()
}
fn default_chunk_size() -> usize {
    400
}
fn default_chunk_overlap() -> usize {
    80
}
fn default_vector_weight() -> f64 {
    0.7
}
fn default_bm25_weight() -> f64 {
    0.3
}
fn default_heartbeat_interval() -> u64 {
    30
}
fn default_log_level() -> String {
    "debug".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level for file output: error, warn, info, debug
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Directory for log files (default: workspace/logs/)
    /// Files are named catclaw-YYYY-MM-DD.jsonl
    #[serde(default)]
    pub log_dir: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: default_log_level(),
            log_dir: None,
        }
    }
}

impl LoggingConfig {
    /// Resolve the log directory (defaults to workspace/logs/)
    pub fn resolve_log_dir(&self, workspace: &Path) -> PathBuf {
        self.log_dir
            .clone()
            .unwrap_or_else(|| workspace.join("logs"))
    }
}

impl Config {
    /// Load config from a TOML file.
    /// Relative paths (workspace, state_db, agent workspaces) are resolved
    /// relative to the config file's parent directory, so the gateway works
    /// correctly regardless of the process's working directory (e.g., launchd).
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            CatClawError::Config(format!("failed to read {}: {}", path.display(), e))
        })?;
        let mut config: Config = toml::from_str(&content)?;

        // Resolve relative paths against config file's directory
        let base = path.parent().unwrap_or(Path::new("."));
        let resolve = |p: &PathBuf| -> PathBuf {
            if p.is_relative() {
                base.join(p)
            } else {
                p.clone()
            }
        };
        config.general.workspace = resolve(&config.general.workspace);
        config.general.state_db = resolve(&config.general.state_db);
        for agent in &mut config.agents {
            agent.workspace = resolve(&agent.workspace);
        }

        // Auto-generate ws_token if missing (upgrade existing configs)
        if config.general.ws_token.is_empty() {
            config.general.ws_token = generate_token();
            // Persist immediately so all clients share the same token
            let _ = config.save(path);
        }
        Ok(config)
    }

    /// Save config to a TOML file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Return the effective webhook base URL (falls back to localhost:{port}).
    pub fn webhook_base_url(&self) -> String {
        self.general.webhook_base_url.clone()
            .unwrap_or_else(|| format!("http://localhost:{}", self.general.port))
    }

    /// Get the default agent ID
    pub fn default_agent_id(&self) -> Option<&str> {
        self.agents
            .iter()
            .find(|a| a.default)
            .or(self.agents.first())
            .map(|a| a.id.as_str())
    }

    /// Get a configuration value by key path.
    pub fn config_get(&self, key: &str) -> Result<String> {
        match key {
            "workspace" => Ok(self.general.workspace.display().to_string()),
            "state_db" => Ok(self.general.state_db.display().to_string()),
            "max_concurrent_sessions" => Ok(self.general.max_concurrent_sessions.to_string()),
            "session_idle_timeout_mins" => Ok(self.general.session_idle_timeout_mins.to_string()),
            "session_archive_timeout_hours" => Ok(self.general.session_archive_timeout_hours.to_string()),
            "port" | "ws_port" => Ok(self.general.port.to_string()),
            "bind_addr" => Ok(self.general.bind_addr.clone()),
            "streaming" => Ok(self.general.streaming.to_string()),
            "default_model" => Ok(self.general.default_model.clone().unwrap_or_default()),
            "default_fallback_model" => Ok(self.general.default_fallback_model.clone().unwrap_or_default()),
            "timezone" => Ok(self.general.timezone.clone().unwrap_or_default()),
            "logging.level" => Ok(self.logging.level.clone()),
            "heartbeat.enabled" => Ok(self.heartbeat.as_ref().is_some_and(|h| h.enabled).to_string()),
            "heartbeat.interval_mins" => Ok(self.heartbeat.as_ref().map_or(30, |h| h.interval_mins).to_string()),
            "approval.timeout_secs" => {
                let t = self.agents.first().map(|a| a.approval.timeout_secs).unwrap_or(120);
                Ok(if t == 0 { "120".to_string() } else { t.to_string() })
            }
            "contacts.enabled" => Ok(self.contacts.enabled.to_string()),
            "contacts.unknown_inbox_channel" => {
                Ok(self.contacts.unknown_inbox_channel.clone().unwrap_or_default())
            }
            "webhook_base_url" => Ok(self.general.webhook_base_url.clone().unwrap_or_else(|| format!("http://localhost:{}", self.general.port))),
            "social.instagram.mode" => Ok(self.social.instagram.as_ref().map_or_else(|| "off".to_string(), |c| c.mode.clone())),
            "social.instagram.poll_interval_mins" => Ok(self.social.instagram.as_ref().map_or(5, |c| c.poll_interval_mins).to_string()),
            "social.instagram.user_id" => Ok(self.social.instagram.as_ref().map(|c| c.user_id.clone()).unwrap_or_default()),
            "social.instagram.admin_channel" => Ok(self.social.instagram.as_ref().map(|c| c.admin_channel.clone()).unwrap_or_default()),
            "social.instagram.token_env" => Ok(self.social.instagram.as_ref().map(|c| c.token_env.clone()).unwrap_or_default()),
            "social.instagram.app_id" => Ok(self.social.instagram.as_ref().and_then(|c| c.app_id.clone()).unwrap_or_default()),
            "social.instagram.app_secret_env" => Ok(self.social.instagram.as_ref().and_then(|c| c.app_secret_env.clone()).unwrap_or_default()),
            "social.instagram.webhook_verify_token_env" => Ok(self.social.instagram.as_ref().and_then(|c| c.webhook_verify_token_env.clone()).unwrap_or_default()),
            "social.instagram.subscribe" => Ok(self.social.instagram.as_ref().map(|c| c.subscribe.join(",")).unwrap_or_default()),
            "social.instagram.agent" => Ok(self.social.instagram.as_ref().map(|c| c.agent.clone()).unwrap_or_default()),
            "social.instagram.rules.count" => Ok(self.social.instagram.as_ref().map_or(0, |c| c.rules.len()).to_string()),
            "social.threads.mode" => Ok(self.social.threads.as_ref().map_or_else(|| "off".to_string(), |c| c.mode.clone())),
            "social.threads.poll_interval_mins" => Ok(self.social.threads.as_ref().map_or(5, |c| c.poll_interval_mins).to_string()),
            "social.threads.user_id" => Ok(self.social.threads.as_ref().map(|c| c.user_id.clone()).unwrap_or_default()),
            "social.threads.admin_channel" => Ok(self.social.threads.as_ref().map(|c| c.admin_channel.clone()).unwrap_or_default()),
            "social.threads.token_env" => Ok(self.social.threads.as_ref().map(|c| c.token_env.clone()).unwrap_or_default()),
            "social.threads.app_id" => Ok(self.social.threads.as_ref().and_then(|c| c.app_id.clone()).unwrap_or_default()),
            "social.threads.app_secret_env" => Ok(self.social.threads.as_ref().and_then(|c| c.app_secret_env.clone()).unwrap_or_default()),
            "social.threads.webhook_verify_token_env" => Ok(self.social.threads.as_ref().and_then(|c| c.webhook_verify_token_env.clone()).unwrap_or_default()),
            "social.threads.subscribe" => Ok(self.social.threads.as_ref().map(|c| c.subscribe.join(",")).unwrap_or_default()),
            "social.threads.agent" => Ok(self.social.threads.as_ref().map(|c| c.agent.clone()).unwrap_or_default()),
            "social.threads.rules.count" => Ok(self.social.threads.as_ref().map_or(0, |c| c.rules.len()).to_string()),
            other => {
                // env.{KEY}
                if let Some(env_key) = other.strip_prefix("env.") {
                    return Ok(self.env.get(env_key).cloned().unwrap_or_default());
                }
                // social.{platform}.rules[N].{field}
                for platform in ["instagram", "threads"] {
                    let prefix = format!("social.{}.rules[", platform);
                    if let Some(rest) = other.strip_prefix(&prefix) {
                        if let Some((idx_str, field)) = rest.split_once("].") {
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                let rules = match platform {
                                    "instagram" => self.social.instagram.as_ref().map(|c| &c.rules),
                                    _ => self.social.threads.as_ref().map(|c| &c.rules),
                                };
                                if let Some(rules) = rules {
                                    if let Some(rule) = rules.get(idx) {
                                        return match field {
                                            "match" => Ok(rule.match_type.clone()),
                                            "action" => Ok(rule.action.clone()),
                                            "keyword" => Ok(rule.keyword.clone().unwrap_or_default()),
                                            "template" => Ok(rule.template.clone().unwrap_or_default()),
                                            "agent" => Ok(rule.agent.clone().unwrap_or_default()),
                                            _ => Err(CatClawError::Config(format!("unknown rule field: {}", field))),
                                        };
                                    }
                                    return Err(CatClawError::Config(format!("rule index {} out of range", idx)));
                                }
                                return Err(CatClawError::Config(format!("social.{} is not configured", platform)));
                            }
                        }
                    }
                }
                if let Some(rest) = other.strip_prefix("channels[") {
                    if let Some((idx_str, field)) = rest.split_once("].") {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            if let Some(ch) = self.channels.get(idx) {
                                return match field {
                                    "type" => Ok(ch.channel_type.clone()),
                                    "token_env" => Ok(ch.token_env.clone()),
                                    "activation" => Ok(ch.activation.clone()),
                                    "guilds" => Ok(ch.guilds.join(",")),
                                    "dm_policy" => Ok(ch.dm_policy.clone()),
                                    "dm_allow" => Ok(ch.dm_allow.join(",")),
                                    "dm_deny" => Ok(ch.dm_deny.join(",")),
                                    "group_policy" => Ok(ch.group_policy.clone()),
                                    "group_allow" => Ok(ch.group_allow.join(",")),
                                    "group_deny" => Ok(ch.group_deny.join(",")),
                                    "app_token_env" => Ok(ch.app_token_env.clone().unwrap_or_default()),
                                    _ => Err(CatClawError::Config(format!("unknown channel field: {}", field))),
                                };
                            }
                            return Err(CatClawError::Config(format!("channel index {} out of range", idx)));
                        }
                    }
                }
                Err(CatClawError::Config(format!("unknown config key: {}", other)))
            }
        }
    }

    /// Apply a key=value config change. Returns Ok(needs_restart: bool).
    /// - false: change takes effect immediately (hot-reloadable)
    /// - true: requires gateway restart
    pub fn apply_config_set(&mut self, key: &str, value: &str) -> Result<bool> {
        match key {
            "streaming" => {
                self.general.streaming = parse_bool(value)?;
                Ok(false)
            }
            "default_model" => {
                self.general.default_model = if value.is_empty() { None } else { Some(value.to_string()) };
                Ok(false)
            }
            "default_fallback_model" => {
                self.general.default_fallback_model = if value.is_empty() { None } else { Some(value.to_string()) };
                Ok(false)
            }
            "max_concurrent_sessions" => {
                self.general.max_concurrent_sessions = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                Ok(true)
            }
            "session_idle_timeout_mins" => {
                self.general.session_idle_timeout_mins = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                Ok(true)
            }
            "session_archive_timeout_hours" => {
                self.general.session_archive_timeout_hours = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                Ok(true)
            }
            "port" | "ws_port" => {
                self.general.port = value.parse().map_err(|_| CatClawError::Config("invalid port".into()))?;
                Ok(true)
            }
            "bind_addr" => {
                self.general.bind_addr = value.to_string();
                Ok(true)
            }
            "contacts.enabled" => {
                self.contacts.enabled = parse_bool(value)?;
                // tools/list reads config live each call, so no restart needed.
                Ok(false)
            }
            "contacts.unknown_inbox_channel" => {
                self.contacts.unknown_inbox_channel =
                    if value.is_empty() { None } else { Some(value.to_string()) };
                Ok(false)
            }
            "heartbeat.enabled" => {
                let enabled = parse_bool(value)?;
                if let Some(h) = &mut self.heartbeat {
                    h.enabled = enabled;
                } else {
                    self.heartbeat = Some(HeartbeatConfig { enabled, interval_mins: default_heartbeat_interval() });
                }
                Ok(true)
            }
            "heartbeat.interval_mins" => {
                let mins: u64 = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                if let Some(h) = &mut self.heartbeat {
                    h.interval_mins = mins;
                } else {
                    self.heartbeat = Some(HeartbeatConfig { enabled: false, interval_mins: mins });
                }
                Ok(true)
            }
            "logging.level" => {
                match value {
                    "error" | "warn" | "info" | "debug" | "trace" => {
                        self.logging.level = value.to_string();
                        Ok(false)
                    }
                    _ => Err(CatClawError::Config("level must be error/warn/info/debug/trace".into())),
                }
            }
            "approval.timeout_secs" => {
                let secs: u64 = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                for agent in &mut self.agents {
                    agent.approval.timeout_secs = secs;
                }
                Ok(false)
            }
            "workspace" => {
                self.general.workspace = value.into();
                Ok(true)
            }
            "state_db" => {
                self.general.state_db = value.into();
                Ok(true)
            }
            "timezone" => {
                if value.is_empty() {
                    self.general.timezone = None;
                } else {
                    // Validate IANA timezone name
                    value.parse::<chrono_tz::Tz>().map_err(|_| {
                        CatClawError::Config(format!("unknown timezone '{}'. Use IANA name like 'Asia/Taipei'", value))
                    })?;
                    self.general.timezone = Some(value.to_string());
                }
                Ok(false)
            }
            "webhook_base_url" => {
                self.general.webhook_base_url = if value.is_empty() { None } else { Some(value.trim_end_matches('/').to_string()) };
                Ok(false)
            }
            "social.instagram.mode" => {
                match value {
                    "webhook" | "polling" | "off" => {
                        if let Some(ref mut ig) = self.social.instagram {
                            ig.mode = value.to_string();
                            Ok(false)
                        } else {
                            Err(CatClawError::Config("social.instagram is not configured".into()))
                        }
                    }
                    _ => Err(CatClawError::Config("social.instagram.mode must be 'webhook', 'polling', or 'off'".into())),
                }
            }
            "social.instagram.poll_interval_mins" => {
                let mins: u64 = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                if let Some(ref mut ig) = self.social.instagram {
                    ig.poll_interval_mins = mins;
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.threads.mode" => {
                match value {
                    "webhook" | "polling" | "off" => {
                        if let Some(ref mut th) = self.social.threads {
                            th.mode = value.to_string();
                            Ok(false)
                        } else {
                            Err(CatClawError::Config("social.threads is not configured".into()))
                        }
                    }
                    _ => Err(CatClawError::Config("social.threads.mode must be 'webhook', 'polling', or 'off'".into())),
                }
            }
            "social.threads.poll_interval_mins" => {
                let mins: u64 = value.parse().map_err(|_| CatClawError::Config("invalid number".into()))?;
                if let Some(ref mut th) = self.social.threads {
                    th.poll_interval_mins = mins;
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.instagram.admin_channel" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.admin_channel = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.threads.admin_channel" => {
                if let Some(ref mut th) = self.social.threads {
                    th.admin_channel = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            // ── Social: additional writable fields ────────────────────────────
            "social.instagram.token_env" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.token_env = value.to_string();
                    Ok(true)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.app_id" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.app_id = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.app_secret_env" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.app_secret_env = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.webhook_verify_token_env" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.webhook_verify_token_env = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.user_id" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.user_id = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.subscribe" => {
                let vals = split_csv(value);
                for v in &vals {
                    if !["comments", "mentions", "messages"].contains(&v.as_str()) {
                        return Err(CatClawError::Config(format!("invalid subscribe value '{}'. Valid: comments, mentions, messages", v)));
                    }
                }
                if let Some(ref mut ig) = self.social.instagram {
                    ig.subscribe = vals;
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.agent" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.agent = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.rules.add" => {
                if let Some(ref mut ig) = self.social.instagram {
                    ig.rules.push(SocialRule {
                        match_type: "*".to_string(),
                        keyword: None,
                        action: "forward".to_string(),
                        template: None,
                        agent: None,
                    });
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.instagram is not configured".into()))
                }
            }
            "social.instagram.init" => {
                if self.social.instagram.is_none() {
                    self.social.instagram = Some(InstagramConfig {
                        mode: "off".to_string(),
                        poll_interval_mins: 5,
                        token_env: "CATCLAW_INSTAGRAM_TOKEN".to_string(),
                        app_id: None,
                        app_secret_env: Some("CATCLAW_INSTAGRAM_APP_SECRET".to_string()),
                        webhook_verify_token_env: Some("CATCLAW_INSTAGRAM_WEBHOOK_VERIFY_TOKEN".to_string()),
                        user_id: String::new(),
                        admin_channel: String::new(),
                        subscribe: vec!["comments".to_string(), "mentions".to_string()],
                        agent: "main".to_string(),
                        rules: vec![SocialRule {
                            match_type: "*".to_string(),
                            keyword: None,
                            action: "forward".to_string(),
                            template: None,
                            agent: None,
                        }],
                        templates: HashMap::new(),
                    });
                }
                Ok(true)
            }
            "social.threads.token_env" => {
                if let Some(ref mut th) = self.social.threads {
                    th.token_env = value.to_string();
                    Ok(true)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.app_id" => {
                if let Some(ref mut th) = self.social.threads {
                    th.app_id = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.app_secret_env" => {
                if let Some(ref mut th) = self.social.threads {
                    th.app_secret_env = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.webhook_verify_token_env" => {
                if let Some(ref mut th) = self.social.threads {
                    th.webhook_verify_token_env = if value.is_empty() { None } else { Some(value.to_string()) };
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.user_id" => {
                if let Some(ref mut th) = self.social.threads {
                    th.user_id = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.subscribe" => {
                let vals = split_csv(value);
                for v in &vals {
                    if !["replies", "mentions"].contains(&v.as_str()) {
                        return Err(CatClawError::Config(format!("invalid subscribe value '{}'. Valid: replies, mentions", v)));
                    }
                }
                if let Some(ref mut th) = self.social.threads {
                    th.subscribe = vals;
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.agent" => {
                if let Some(ref mut th) = self.social.threads {
                    th.agent = value.to_string();
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.rules.add" => {
                if let Some(ref mut th) = self.social.threads {
                    th.rules.push(SocialRule {
                        match_type: "*".to_string(),
                        keyword: None,
                        action: "forward".to_string(),
                        template: None,
                        agent: None,
                    });
                    Ok(false)
                } else {
                    Err(CatClawError::Config("social.threads is not configured".into()))
                }
            }
            "social.threads.init" => {
                if self.social.threads.is_none() {
                    self.social.threads = Some(ThreadsConfig {
                        mode: "off".to_string(),
                        poll_interval_mins: 5,
                        token_env: "CATCLAW_THREADS_TOKEN".to_string(),
                        app_id: None,
                        app_secret_env: Some("CATCLAW_THREADS_APP_SECRET".to_string()),
                        webhook_verify_token_env: Some("CATCLAW_THREADS_WEBHOOK_VERIFY_TOKEN".to_string()),
                        user_id: String::new(),
                        admin_channel: String::new(),
                        subscribe: vec!["replies".to_string(), "mentions".to_string()],
                        agent: "main".to_string(),
                        rules: vec![SocialRule {
                            match_type: "*".to_string(),
                            keyword: None,
                            action: "forward".to_string(),
                            template: None,
                            agent: None,
                        }],
                        templates: HashMap::new(),
                    });
                }
                Ok(true)
            }
            other => {
                // env.{KEY}
                if let Some(env_key) = other.strip_prefix("env.") {
                    if value.is_empty() {
                        self.env.remove(env_key);
                    } else {
                        self.env.insert(env_key.to_string(), value.to_string());
                    }
                    return Ok(false);
                }
                // social.{platform}.rules[N].{field} and social.{platform}.rules[N].delete
                for platform in ["instagram", "threads"] {
                    let prefix = format!("social.{}.rules[", platform);
                    if let Some(rest) = other.strip_prefix(&prefix) {
                        if let Some((idx_str, field)) = rest.split_once("].") {
                            let idx: usize = idx_str.parse().map_err(|_| CatClawError::Config("invalid rule index".into()))?;
                            let rules = match platform {
                                "instagram" => self.social.instagram.as_mut().map(|c| &mut c.rules),
                                _ => self.social.threads.as_mut().map(|c| &mut c.rules),
                            };
                            let rules = rules.ok_or_else(|| CatClawError::Config(format!("social.{} is not configured", platform)))?;
                            if field == "delete" {
                                if idx >= rules.len() {
                                    return Err(CatClawError::Config(format!("rule index {} out of range", idx)));
                                }
                                rules.remove(idx);
                                return Ok(false);
                            }
                            if idx >= rules.len() {
                                return Err(CatClawError::Config(format!("rule index {} out of range", idx)));
                            }
                            let rule = &mut rules[idx];
                            return match field {
                                "match" => { rule.match_type = value.to_string(); Ok(false) }
                                "action" => {
                                    match value {
                                        "forward" | "auto_reply" | "auto_reply_template" | "ignore" => {
                                            rule.action = value.to_string();
                                            Ok(false)
                                        }
                                        _ => Err(CatClawError::Config("action must be forward, auto_reply, auto_reply_template, or ignore".into())),
                                    }
                                }
                                "keyword" => { rule.keyword = if value.is_empty() { None } else { Some(value.to_string()) }; Ok(false) }
                                "template" => { rule.template = if value.is_empty() { None } else { Some(value.to_string()) }; Ok(false) }
                                "agent" => { rule.agent = if value.is_empty() { None } else { Some(value.to_string()) }; Ok(false) }
                                _ => Err(CatClawError::Config(format!("unknown rule field: {}", field))),
                            };
                        }
                    }
                }
                if let Some(rest) = other.strip_prefix("channels[") {
                    if let Some((idx_str, field)) = rest.split_once("].") {
                        let idx: usize = idx_str.parse().map_err(|_| CatClawError::Config("invalid channel index".into()))?;
                        if idx >= self.channels.len() {
                            return Err(CatClawError::Config(format!("channel index {} out of range", idx)));
                        }
                        let ch = &mut self.channels[idx];
                        match field {
                            "activation" => {
                                match value {
                                    "mention" | "all" => ch.activation = value.to_string(),
                                    _ => return Err(CatClawError::Config("activation must be 'mention' or 'all'".into())),
                                }
                                Ok(false)
                            }
                            "guilds" => {
                                ch.guilds = split_csv(value);
                                Ok(false)
                            }
                            "dm_policy" => {
                                match value {
                                    "open" | "allowlist" | "disabled" => ch.dm_policy = value.to_string(),
                                    _ => return Err(CatClawError::Config("dm_policy must be 'open', 'allowlist', or 'disabled'".into())),
                                }
                                Ok(false)
                            }
                            "dm_allow" => { ch.dm_allow = split_csv(value); Ok(false) }
                            "dm_deny" => { ch.dm_deny = split_csv(value); Ok(false) }
                            "group_policy" => {
                                match value {
                                    "open" | "allowlist" => ch.group_policy = value.to_string(),
                                    _ => return Err(CatClawError::Config("group_policy must be 'open' or 'allowlist'".into())),
                                }
                                Ok(false)
                            }
                            "group_allow" => { ch.group_allow = split_csv(value); Ok(false) }
                            "group_deny" => { ch.group_deny = split_csv(value); Ok(false) }
                            "token_env" => {
                                ch.token_env = value.to_string();
                                Ok(true)
                            }
                            "app_token_env" => {
                                ch.app_token_env = if value.is_empty() { None } else { Some(value.to_string()) };
                                Ok(true)
                            }
                            _ => Err(CatClawError::Config(format!("unknown channel field: {}", field))),
                        }
                    } else {
                        Err(CatClawError::Config(format!("invalid key format: {}", other)))
                    }
                } else {
                    Err(CatClawError::Config(format!("unknown config key: {}", other)))
                }
            }
        }
    }

    /// Create a default config for `catclaw onboard`
    pub fn default_init() -> Self {
        Config {
            general: GeneralConfig {
                workspace: default_workspace(),
                state_db: default_state_db(),
                max_concurrent_sessions: default_max_concurrent(),
                session_idle_timeout_mins: default_idle_timeout(),
                session_archive_timeout_hours: default_archive_timeout(),
                port: default_ws_port(),
                bind_addr: default_bind_addr(),
                streaming: default_streaming(),
                default_model: None,
                default_fallback_model: None,
                ws_token: generate_token(),
                timezone: None,
                webhook_base_url: None,
            },
            channels: vec![],
            agents: vec![AgentConfig {
                id: "main".to_string(),
                workspace: default_workspace().join("agents/main"),
                default: true,
                model: None,
                fallback_model: None,
                approval: ApprovalConfig::default(),
            }],
            bindings: vec![],
            collaboration: CollaborationConfig::default(),
            embedding: Some(EmbeddingConfig {
                provider: default_embedding_provider(),
                ollama_url: default_ollama_url(),
                model: default_embedding_model(),
            }),
            memory: Some(MemoryConfig {
                chunk_size_tokens: default_chunk_size(),
                chunk_overlap_tokens: default_chunk_overlap(),
                vector_weight: default_vector_weight(),
                bm25_weight: default_bm25_weight(),
            }),
            heartbeat: Some(HeartbeatConfig {
                enabled: true,
                interval_mins: default_heartbeat_interval(),
            }),
            logging: LoggingConfig::default(),
            mcp_env: HashMap::new(),
            env: HashMap::new(),
            social: SocialConfig::default(),
            contacts: ContactsConfig::default(),
        }
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(CatClawError::Config("invalid boolean (use true/false)".into())),
    }
}

/// Write or update a single `KEY=value` line in `~/.catclaw/.env`.
/// Also updates the current process environment.
pub fn write_env_var(env_var: &str, value: &str) {
    let env_path = {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(".catclaw").join(".env")
    };
    let mut lines: Vec<String> = if env_path.exists() {
        std::fs::read_to_string(&env_path).unwrap_or_default().lines().map(String::from).collect()
    } else {
        Vec::new()
    };
    let prefix = format!("{}=", env_var);
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = format!("{}={}", env_var, value);
    } else {
        lines.push(format!("{}={}", env_var, value));
    }
    if let Some(parent) = env_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&env_path, lines.join("\n") + "\n");
    std::env::set_var(env_var, value);
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Generate a random 32-byte hex token for WS authentication.
fn generate_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Mix time + process ID + random-ish data for a unique token
    let mut h = DefaultHasher::new();
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().hash(&mut h);
    std::process::id().hash(&mut h);
    let h1 = h.finish();

    // Second hash with different seed
    let mut h2 = DefaultHasher::new();
    h1.hash(&mut h2);
    std::thread::current().id().hash(&mut h2);
    let h2 = h2.finish();

    let mut h3 = DefaultHasher::new();
    h2.hash(&mut h3);
    (h1 ^ h2).hash(&mut h3);
    let h3 = h3.finish();

    let mut h4 = DefaultHasher::new();
    h3.hash(&mut h4);
    (h1.wrapping_add(h3)).hash(&mut h4);
    let h4 = h4.finish();

    format!("{:016x}{:016x}{:016x}{:016x}", h1, h2, h3, h4)
}
