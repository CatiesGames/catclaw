use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;
use serde_json::json;
use tokio::sync::mpsc;

use super::theme::Theme;
use super::{Action, Component};
use crate::config::Config;
use crate::ws_client::GatewayClient;

#[derive(Debug, Clone, PartialEq)]
enum ConfigMode {
    Normal,
    Editing,
    /// Multi-step input for adding a new MCP env var: server → key → value
    AddingMcpEnv { step: McpEnvStep },
}

#[derive(Debug, Clone, PartialEq)]
enum McpEnvStep {
    Server,
    Key { server: String },
    Value { server: String, key: String },
}

struct ConfigEntry {
    key: String,
    value: String,
    section: String,
    editable: bool,
}

enum ConfigEvent {
    SetResult { key: String, value: String, needs_restart: bool },
    SetError(String),
}

pub struct ConfigPanel {
    config: Config,
    config_path: PathBuf,
    client: Arc<GatewayClient>,
    event_tx: mpsc::UnboundedSender<ConfigEvent>,
    event_rx: mpsc::UnboundedReceiver<ConfigEvent>,
    entries: Vec<ConfigEntry>,
    selected: usize,
    mode: ConfigMode,
    edit_buffer: String,
    status_msg: Option<String>,
    completions: Vec<String>,
    completion_idx: usize,
    pending_action: Option<Action>,
}

impl ConfigPanel {
    pub fn new(config: &Config, config_path: PathBuf, client: Arc<GatewayClient>) -> Self {
        let entries = Self::build_entries(config);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        ConfigPanel {
            config: config.clone(),
            config_path,
            client,
            event_tx,
            event_rx,
            entries,
            selected: 0,
            mode: ConfigMode::Normal,
            edit_buffer: String::new(),
            status_msg: None,
            completions: Vec::new(),
            completion_idx: 0,
            pending_action: None,
        }
    }

    fn build_entries(config: &Config) -> Vec<ConfigEntry> {
        let mut entries = vec![
            ConfigEntry {
                key: "workspace".to_string(),
                value: config.general.workspace.display().to_string(),
                section: "General".to_string(),
                editable: false,
            },
            ConfigEntry {
                key: "state_db".to_string(),
                value: config.general.state_db.display().to_string(),
                section: "General".to_string(),
                editable: false,
            },
            ConfigEntry {
                key: "max_concurrent_sessions".to_string(),
                value: config.general.max_concurrent_sessions.to_string(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "session_idle_timeout_mins".to_string(),
                value: config.general.session_idle_timeout_mins.to_string(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "session_archive_timeout_hours".to_string(),
                value: config.general.session_archive_timeout_hours.to_string(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "streaming".to_string(),
                value: config.general.streaming.to_string(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "default_model".to_string(),
                value: config.general.default_model.clone().unwrap_or_default(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "default_fallback_model".to_string(),
                value: config.general.default_fallback_model.clone().unwrap_or_default(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "timezone".to_string(),
                value: config.general.timezone.clone().unwrap_or_default(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "logging.level".to_string(),
                value: config.logging.level.clone(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "port".to_string(),
                value: config.general.port.to_string(),
                section: "General".to_string(),
                editable: true,
            },
            ConfigEntry {
                key: "webhook_base_url".to_string(),
                value: config.general.webhook_base_url.clone().unwrap_or_default(),
                section: "General".to_string(),
                editable: true,
            },
        ];

    // Heartbeat section
    {
        let (enabled, interval) = config.heartbeat.as_ref()
            .map_or((false, 30u64), |h| (h.enabled, h.interval_mins));
        entries.push(ConfigEntry {
            key: "heartbeat.enabled".to_string(),
            value: enabled.to_string(),
            section: "Heartbeat".to_string(),
            editable: true,
        });
        entries.push(ConfigEntry {
            key: "heartbeat.interval_mins".to_string(),
            value: interval.to_string(),
            section: "Heartbeat".to_string(),
            editable: true,
        });
    }

        // Approval (global default — per-agent timeout uses this as fallback)
        {
            // Find the "common" timeout: use the first agent's value or default 120
            let timeout = config.agents.first()
                .map(|a| a.approval.timeout_secs)
                .unwrap_or(120);
            let timeout = if timeout == 0 { 120 } else { timeout };
            entries.push(ConfigEntry {
                key: "approval.timeout_secs".to_string(),
                value: timeout.to_string(),
                section: "Approval".to_string(),
                editable: true,
            });
        }

        // Channels
        for (i, ch) in config.channels.iter().enumerate() {
            entries.push(ConfigEntry {
                key: format!("channels[{}].token_env", i),
                value: ch.token_env.clone(),
                section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                editable: false,
            });
            entries.push(ConfigEntry {
                key: format!("channels[{}].activation", i),
                value: ch.activation.clone(),
                section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                editable: true,
            });
            if !ch.guilds.is_empty() {
                entries.push(ConfigEntry {
                    key: format!("channels[{}].guilds", i),
                    value: ch.guilds.join(", "),
                    section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                    editable: true,
                });
            }
            entries.push(ConfigEntry {
                key: format!("channels[{}].dm_policy", i),
                value: ch.dm_policy.clone(),
                section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                editable: true,
            });
            if !ch.dm_allow.is_empty() || ch.dm_policy == "allowlist" {
                entries.push(ConfigEntry {
                    key: format!("channels[{}].dm_allow", i),
                    value: ch.dm_allow.join(", "),
                    section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                    editable: true,
                });
            }
            if !ch.dm_deny.is_empty() {
                entries.push(ConfigEntry {
                    key: format!("channels[{}].dm_deny", i),
                    value: ch.dm_deny.join(", "),
                    section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                    editable: true,
                });
            }
            entries.push(ConfigEntry {
                key: format!("channels[{}].group_policy", i),
                value: ch.group_policy.clone(),
                section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                editable: true,
            });
            if !ch.group_allow.is_empty() || ch.group_policy == "allowlist" {
                entries.push(ConfigEntry {
                    key: format!("channels[{}].group_allow", i),
                    value: ch.group_allow.join(", "),
                    section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                    editable: true,
                });
            }
            if !ch.group_deny.is_empty() {
                entries.push(ConfigEntry {
                    key: format!("channels[{}].group_deny", i),
                    value: ch.group_deny.join(", "),
                    section: format!("Channel: {} ({})", ch.channel_type, ch.activation),
                    editable: true,
                });
            }
        }

        // Embedding
        if let Some(emb) = &config.embedding {
            entries.push(ConfigEntry {
                key: "provider".to_string(),
                value: emb.provider.clone(),
                section: "Embedding".to_string(),
                editable: false,
            });
            entries.push(ConfigEntry {
                key: "model".to_string(),
                value: emb.model.clone(),
                section: "Embedding".to_string(),
                editable: false,
            });
        }

        // Social Inbox
        if let Some(ig) = &config.social.instagram {
            let base = config.webhook_base_url();
            entries.push(ConfigEntry {
                key: "social.instagram.mode".to_string(),
                value: ig.mode.clone(),
                section: "Social: Instagram".to_string(),
                editable: true,
            });
            entries.push(ConfigEntry {
                key: "social.instagram.webhook_url".to_string(),
                value: format!("{}/webhook/instagram", base),
                section: "Social: Instagram".to_string(),
                editable: false,
            });
            entries.push(ConfigEntry {
                key: "social.instagram.poll_interval_mins".to_string(),
                value: ig.poll_interval_mins.to_string(),
                section: "Social: Instagram".to_string(),
                editable: true,
            });
            entries.push(ConfigEntry {
                key: "social.instagram.admin_channel".to_string(),
                value: ig.admin_channel.clone(),
                section: "Social: Instagram".to_string(),
                editable: true,
            });
        }
        if let Some(th) = &config.social.threads {
            let base = config.webhook_base_url();
            entries.push(ConfigEntry {
                key: "social.threads.mode".to_string(),
                value: th.mode.clone(),
                section: "Social: Threads".to_string(),
                editable: true,
            });
            entries.push(ConfigEntry {
                key: "social.threads.webhook_url".to_string(),
                value: format!("{}/webhook/threads", base),
                section: "Social: Threads".to_string(),
                editable: false,
            });
            entries.push(ConfigEntry {
                key: "social.threads.poll_interval_mins".to_string(),
                value: th.poll_interval_mins.to_string(),
                section: "Social: Threads".to_string(),
                editable: true,
            });
            entries.push(ConfigEntry {
                key: "social.threads.admin_channel".to_string(),
                value: th.admin_channel.clone(),
                section: "Social: Threads".to_string(),
                editable: true,
            });
        }

        // MCP Env
        for (server, vars) in &config.mcp_env {
            for (k, v) in vars {
                let masked = Self::mask_value(v);
                entries.push(ConfigEntry {
                    key: format!("mcp_env.{}.{}", server, k),
                    value: masked,
                    section: format!("MCP Env: {}", server),
                    editable: true,
                });
            }
        }

        entries
    }

    fn mask_value(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() <= 6 {
            "***".to_string()
        } else {
            let prefix: String = chars[..3].iter().collect();
            let suffix: String = chars[chars.len()-3..].iter().collect();
            format!("{}...{}", prefix, suffix)
        }
    }

    fn reload_config(&mut self) {
        match Config::load(&self.config_path) {
            Ok(config) => {
                self.entries = Self::build_entries(&config);
                self.config = config;
                if self.selected >= self.entries.len() && !self.entries.is_empty() {
                    self.selected = self.entries.len() - 1;
                }
                self.status_msg = Some("Reloaded config from disk".to_string());
            }
            Err(e) => {
                self.status_msg = Some(format!("Failed to reload: {}", e));
            }
        }
    }

    fn completions_for_key(key: &str) -> Vec<String> {
        if key == "streaming" {
            return vec!["true".into(), "false".into()];
        }
        if key == "default_model" || key == "default_fallback_model" {
            return vec!["opus".into(), "sonnet".into(), "haiku".into(), "".into()];
        }
        if key == "logging.level" {
            return vec!["error".into(), "warn".into(), "info".into(), "debug".into()];
        }
        if key == "heartbeat.enabled" {
            return vec!["true".into(), "false".into()];
        }
        // channels[N].activation
        if key.ends_with(".activation") && key.starts_with("channels[") {
            return vec!["mention".into(), "all".into()];
        }
        // channels[N].dm_policy
        if key.ends_with(".dm_policy") && key.starts_with("channels[") {
            return vec!["open".into(), "allowlist".into(), "disabled".into()];
        }
        // channels[N].group_policy
        if key.ends_with(".group_policy") && key.starts_with("channels[") {
            return vec!["open".into(), "allowlist".into()];
        }
        if key == "social.instagram.mode" || key == "social.threads.mode" {
            return vec!["webhook".into(), "polling".into(), "off".into()];
        }
        vec![]
    }

    fn filtered_completions(&self) -> Vec<&str> {
        let query = self.edit_buffer.to_lowercase();
        if query.is_empty() {
            self.completions.iter().map(|s| s.as_str()).collect()
        } else {
            self.completions
                .iter()
                .filter(|c| c.to_lowercase().contains(&query) || c.is_empty())
                .map(|s| s.as_str())
                .collect()
        }
    }

    fn accept_completion(&mut self) {
        let filtered = self.filtered_completions();
        if let Some(&val) = filtered.get(self.completion_idx) {
            self.edit_buffer = val.to_string();
        }
    }

    fn apply_edit(&mut self) {
        let value = self.edit_buffer.trim().to_string();

        let Some(entry) = self.entries.get(self.selected) else {
            self.mode = ConfigMode::Normal;
            return;
        };
        let key = entry.key.clone();

        // Model fields and list fields (allow/deny) allow empty (= clear)
        let is_model_field = key == "default_model" || key == "default_fallback_model";
        let is_list_field = key.ends_with("_allow") || key.ends_with("_deny") || key.ends_with(".guilds");
        if value.is_empty() && !is_model_field && !is_list_field {
            self.status_msg = Some("Value cannot be empty".to_string());
            self.mode = ConfigMode::Normal;
            return;
        }

        // Route MCP env keys to mcp_env.set, everything else to config.set
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        let key_clone = key.clone();
        let value_clone = value.clone();

        if let Some(rest) = key.strip_prefix("mcp_env.") {
            // Parse "mcp_env.{server}.{env_key}"
            if let Some((server, env_key)) = rest.split_once('.') {
                let server = server.to_string();
                let env_key = env_key.to_string();
                tokio::spawn(async move {
                    match client.request("mcp_env.set", json!({"server": server, "key": env_key, "value": value_clone})).await {
                        Ok(_) => {
                            let _ = tx.send(ConfigEvent::SetResult {
                                key: key_clone,
                                value: "(updated)".to_string(),
                                needs_restart: false,
                            });
                        }
                        Err(e) => { let _ = tx.send(ConfigEvent::SetError(e)); }
                    }
                });
            }
            self.status_msg = Some(format!("Setting {} ...", key));
            self.mode = ConfigMode::Normal;
            return;
        }

        // Route social mode changes to social.mode (returns webhook_url in response)
        if key == "social.instagram.mode" || key == "social.threads.mode" {
            let platform = if key.contains("instagram") { "instagram" } else { "threads" }.to_string();
            tokio::spawn(async move {
                match client.request("social.mode", json!({"platform": platform, "mode": value_clone})).await {
                    Ok(resp) => {
                        let mode_str = resp.get("mode").and_then(|v| v.as_str()).unwrap_or("");
                        let requires_restart = resp.get("requires_restart").and_then(|v| v.as_bool()).unwrap_or(false);
                        let display = if let Some(url) = resp.get("webhook_url").and_then(|v| v.as_str()) {
                            format!("{} — webhook URL: {} (immediate)", mode_str, url)
                        } else if requires_restart {
                            format!("{} — restart gateway to apply", mode_str)
                        } else {
                            mode_str.to_string()
                        };
                        let _ = tx.send(ConfigEvent::SetResult {
                            key: key_clone,
                            value: display,
                            needs_restart: requires_restart,
                        });
                    }
                    Err(e) => { let _ = tx.send(ConfigEvent::SetError(e)); }
                }
            });
            self.status_msg = Some(format!("Setting {} ...", key));
            self.mode = ConfigMode::Normal;
            return;
        }

        tokio::spawn(async move {
            match client.request("config.set", json!({"key": key_clone, "value": value_clone})).await {
                Ok(resp) => {
                    let needs_restart = resp.get("needs_restart").and_then(|v| v.as_bool()).unwrap_or(false);
                    let _ = tx.send(ConfigEvent::SetResult {
                        key: key_clone,
                        value: value_clone,
                        needs_restart,
                    });
                }
                Err(e) => {
                    let _ = tx.send(ConfigEvent::SetError(e));
                }
            }
        });

        self.status_msg = Some(format!("Setting {} ...", key));
        self.mode = ConfigMode::Normal;
    }

    /// Poll for async WS results. Called from handle_event or render.
    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                ConfigEvent::SetResult { key, value, needs_restart } => {
                    if needs_restart {
                        self.status_msg = Some(format!("Set {} = {} (requires restart)", key, value));
                    } else {
                        self.status_msg = Some(format!("Set {} = {} (applied)", key, value));
                    }
                    // Propagate log level change to LogsPanel
                    if key == "logging.level" {
                        self.pending_action = Some(Action::SetLogLevel(value.clone()));
                    }
                    // Reload from disk to stay in sync
                    self.reload_config();
                }
                ConfigEvent::SetError(e) => {
                    self.status_msg = Some(format!("Error: {}", e));
                }
            }
        }
    }
}

impl Component for ConfigPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        let action = match &self.mode {
            ConfigMode::Normal => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.entries.is_empty() {
                        self.selected = (self.selected + 1).min(self.entries.len() - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Enter => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        if entry.editable {
                            // MCP env entries display masked values — start with empty buffer
                            // to force the user to type the new value (not edit the mask)
                            if entry.key.starts_with("mcp_env.") {
                                self.edit_buffer.clear();
                            } else {
                                self.edit_buffer = entry.value.clone();
                            }
                            self.completions = Self::completions_for_key(&entry.key);
                            self.completion_idx = 0;
                            self.mode = ConfigMode::Editing;
                            self.status_msg = None;
                        } else {
                            self.status_msg = Some("This field is not editable".to_string());
                        }
                    }
                    Action::None
                }
                KeyCode::Char('r') => {
                    self.reload_config();
                    Action::None
                }
                KeyCode::Char('a') => {
                    // Start adding a new MCP env var
                    self.edit_buffer.clear();
                    self.mode = ConfigMode::AddingMcpEnv {
                        step: McpEnvStep::Server,
                    };
                    self.status_msg = None;
                    Action::None
                }
                KeyCode::Char('d') => {
                    // Delete selected MCP env entry
                    if let Some(entry) = self.entries.get(self.selected) {
                        if let Some(rest) = entry.key.strip_prefix("mcp_env.") {
                            if let Some((server, env_key)) = rest.split_once('.') {
                                let client = self.client.clone();
                                let tx = self.event_tx.clone();
                                let server = server.to_string();
                                let env_key = env_key.to_string();
                                let key_full = entry.key.clone();
                                tokio::spawn(async move {
                                    match client.request("mcp_env.remove", json!({"server": server, "key": env_key})).await {
                                        Ok(_) => {
                                            let _ = tx.send(ConfigEvent::SetResult {
                                                key: key_full,
                                                value: "(removed)".to_string(),
                                                needs_restart: false,
                                            });
                                        }
                                        Err(e) => { let _ = tx.send(ConfigEvent::SetError(e)); }
                                    }
                                });
                                self.status_msg = Some("Removing...".to_string());
                            }
                        }
                    }
                    Action::None
                }
                _ => Action::None,
            },

            ConfigMode::Editing => match event.code {
                KeyCode::Enter => {
                    if self.edit_buffer.is_empty() && !self.completions.is_empty() {
                        self.accept_completion();
                    }
                    self.apply_edit();
                    Action::None
                }
                KeyCode::Tab if !self.completions.is_empty() => {
                    self.accept_completion();
                    Action::None
                }
                KeyCode::Down if !self.completions.is_empty() => {
                    let count = self.filtered_completions().len();
                    if count > 0 {
                        self.completion_idx = (self.completion_idx + 1).min(count - 1);
                    }
                    Action::None
                }
                KeyCode::Up if !self.completions.is_empty() => {
                    self.completion_idx = self.completion_idx.saturating_sub(1);
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = ConfigMode::Normal;
                    self.status_msg = Some("Cancelled".to_string());
                    Action::None
                }
                KeyCode::Backspace => {
                    self.edit_buffer.pop();
                    self.completion_idx = 0;
                    Action::None
                }
                KeyCode::Char(c) => {
                    self.edit_buffer.push(c);
                    self.completion_idx = 0;
                    Action::None
                }
                _ => Action::None,
            },

            ConfigMode::AddingMcpEnv { step } => {
                let step = step.clone();
                match event.code {
                    KeyCode::Enter => {
                        let input = self.edit_buffer.trim().to_string();
                        if input.is_empty() {
                            self.status_msg = Some("Cannot be empty".to_string());
                            self.mode = ConfigMode::Normal;
                        } else {
                            match step {
                                McpEnvStep::Server => {
                                    self.edit_buffer.clear();
                                    self.mode = ConfigMode::AddingMcpEnv {
                                        step: McpEnvStep::Key { server: input },
                                    };
                                }
                                McpEnvStep::Key { server } => {
                                    self.edit_buffer.clear();
                                    self.mode = ConfigMode::AddingMcpEnv {
                                        step: McpEnvStep::Value { server, key: input },
                                    };
                                }
                                McpEnvStep::Value { server, key } => {
                                    // Submit via WS
                                    let client = self.client.clone();
                                    let tx = self.event_tx.clone();
                                    let value = input;
                                    let key_display = format!("mcp_env.{}.{}", server, key);
                                    let s = server.clone();
                                    let k = key.clone();
                                    let v = value.clone();
                                    tokio::spawn(async move {
                                        match client.request("mcp_env.set", json!({"server": s, "key": k, "value": v})).await {
                                            Ok(_) => {
                                                let _ = tx.send(ConfigEvent::SetResult {
                                                    key: key_display,
                                                    value: "(set)".to_string(),
                                                    needs_restart: false,
                                                });
                                            }
                                            Err(e) => { let _ = tx.send(ConfigEvent::SetError(e)); }
                                        }
                                    });
                                    self.status_msg = Some(format!("Setting {}.{} ...", server, key));
                                    self.mode = ConfigMode::Normal;
                                }
                            }
                        }
                        Action::None
                    }
                    KeyCode::Esc => {
                        self.mode = ConfigMode::Normal;
                        self.status_msg = Some("Cancelled".to_string());
                        Action::None
                    }
                    KeyCode::Backspace => {
                        self.edit_buffer.pop();
                        Action::None
                    }
                    KeyCode::Char(c) => {
                        self.edit_buffer.push(c);
                        Action::None
                    }
                    _ => Action::None,
                }
            }
        };

        // Drain async events; if a pending action was set, return it instead
        self.drain_events();
        if let Some(action) = self.pending_action.take() {
            return action;
        }
        action
    }

    fn captures_input(&self) -> bool {
        matches!(self.mode, ConfigMode::Editing | ConfigMode::AddingMcpEnv { .. })
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Process any pending WS responses
        self.drain_events();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        // Split main area for autocomplete popup when editing with completions
        let filtered = if self.mode == ConfigMode::Editing { self.filtered_completions() } else { vec![] };
        let has_completions = !filtered.is_empty();
        let filtered_count = filtered.len().min(5) as u16;
        let (entries_area, autocomplete_area) = if has_completions && chunks[0].height > filtered_count + 6 {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(0),
                    Constraint::Length(filtered_count + 2),
                ])
                .split(chunks[0]);
            (split[0], Some(split[1]))
        } else {
            (chunks[0], None)
        };

        let mut lines: Vec<Line> = Vec::new();
        let mut current_section = String::new();

        for (i, entry) in self.entries.iter().enumerate() {
            if entry.section != current_section {
                if !current_section.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(vec![Span::styled(
                    format!("  {}", entry.section),
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    "  ────────",
                    Style::default().fg(Theme::SURFACE1),
                )]));
                current_section = entry.section.clone();
            }

            let style = if i == self.selected {
                Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };

            let editable_marker = if entry.editable { "" } else { " 🔒" };

            lines.push(Line::from(vec![
                Span::styled(format!("  {:<30}", entry.key), style),
                Span::styled(&entry.value, Style::default().fg(Theme::TEXT)),
                Span::styled(editable_marker, Style::default().fg(Theme::SURFACE2)),
            ]));
        }

        let paragraph = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(" Config ")
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(paragraph, entries_area);

        // ── Autocomplete popup ──
        if let Some(ac_area) = autocomplete_area {
            let filtered = self.filtered_completions();
            let items: Vec<ListItem> = filtered
                .iter()
                .enumerate()
                .take(5)
                .map(|(i, &val)| {
                    let display = if val.is_empty() { "(clear)" } else { val };
                    let is_selected = i == self.completion_idx;
                    let (prefix, style) = if is_selected {
                        (
                            "  ▸ ",
                            Style::default()
                                .fg(Theme::MAUVE)
                                .bg(Theme::SURFACE0)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        ("    ", Style::default().fg(Theme::SUBTEXT0))
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::styled(display, style),
                    ]))
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Theme::MAUVE))
                    .title(" Options ")
                    .title_style(Style::default().fg(Theme::MAUVE)),
            );
            frame.render_widget(list, ac_area);
        }

        // Status / input line
        let status_line = match &self.mode {
            ConfigMode::Editing => {
                let key_name = self
                    .entries
                    .get(self.selected)
                    .map(|e| e.key.as_str())
                    .unwrap_or("?");
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" {}: ", key_name),
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}▌", self.edit_buffer),
                        Style::default().fg(Theme::TEXT),
                    ),
                ]))
                .style(Style::default().bg(Theme::SURFACE0))
            }
            ConfigMode::AddingMcpEnv { step } => {
                let label = match step {
                    McpEnvStep::Server => "Server name",
                    McpEnvStep::Key { .. } => "Env var name",
                    McpEnvStep::Value { .. } => "Env var value",
                };
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" {}: ", label),
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}▌", self.edit_buffer),
                        Style::default().fg(Theme::TEXT),
                    ),
                ]))
                .style(Style::default().bg(Theme::SURFACE0))
            }
            ConfigMode::Normal => {
                if let Some(msg) = &self.status_msg {
                    Paragraph::new(format!(" {}", msg))
                        .style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
                } else {
                    Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
                }
            }
        };
        frame.render_widget(status_line, chunks[1]);

        // Help bar
        let help_text = match &self.mode {
            ConfigMode::Editing => if has_completions {
                " Enter Save  Tab Complete  ↑↓ Select  Esc Cancel"
            } else {
                " Enter Save  Esc Cancel"
            },
            ConfigMode::AddingMcpEnv { .. } => " Enter Next  Esc Cancel",
            ConfigMode::Normal => " j/k Navigate  Enter Edit  a Add MCP Env  d Delete MCP Env  r Reload",
        };
        let help = Paragraph::new(help_text)
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[2]);
    }
}
