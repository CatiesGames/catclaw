use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{CatClawError, Result};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    #[serde(rename = "type")]
    pub channel_type: String,

    pub token_env: String,

    #[serde(default)]
    pub guilds: Vec<String>,

    #[serde(default = "default_activation")]
    pub activation: String,

    #[serde(default)]
    pub overrides: Vec<ChannelOverride>,

    /// DM policy: "open" (default), "allowlist", or "disabled"
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,

    /// Sender IDs allowed to DM (only used when dm_policy = "allowlist")
    #[serde(default)]
    pub dm_allow: Vec<String>,

    /// Sender IDs denied from DM (checked before allow; works in any policy)
    #[serde(default)]
    pub dm_deny: Vec<String>,

    /// Group policy: "open" (default) or "allowlist"
    #[serde(default = "default_group_policy")]
    pub group_policy: String,

    /// Sender IDs allowed in groups (only used when group_policy = "allowlist")
    #[serde(default)]
    pub group_allow: Vec<String>,

    /// Sender IDs denied in groups (checked before allow; works in any policy)
    #[serde(default)]
    pub group_deny: Vec<String>,

    /// Environment variable name for the app-level token (Slack Socket Mode only).
    #[serde(default)]
    pub app_token_env: Option<String>,
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
fn default_streaming() -> bool {
    true
}
fn default_activation() -> String {
    "mention".to_string()
}
fn default_dm_policy() -> String {
    "open".to_string()
}
fn default_group_policy() -> String {
    "open".to_string()
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
            other => {
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
            other => {
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
                streaming: default_streaming(),
                default_model: None,
                default_fallback_model: None,
                ws_token: generate_token(),
                timezone: None,
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
