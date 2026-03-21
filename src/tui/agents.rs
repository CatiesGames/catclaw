use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::editor::{EditorAction, MdEditor};
use super::theme::Theme;
use super::{Action, Component};
use crate::agent::{AgentLoader, SkillSource};
use crate::config::{AgentConfig, Config};
use crate::ws_client::GatewayClient;

/// Claude Code built-in tools relevant to CatClaw agents.
/// Excluded tools (CatClaw has its own alternatives or they're irrelevant):
/// - CronCreate/CronDelete/CronList — CatClaw scheduler (`catclaw task add`)
/// - NotebookEdit — agents don't use Jupyter
/// - EnterWorktree — git worktree not needed for agents
/// - TodoWrite — agents use memory files (MEMORY.md, memory/*.md)
/// - EnterPlanMode/ExitPlanMode — plan mode is for interactive Claude Code, not subprocesses
const ALL_BUILTIN_TOOLS: &[(&str, &str)] = &[
    ("Bash",            "Execute shell commands"),
    ("Edit",            "Edit files with exact string replacement"),
    ("Glob",            "Find files by pattern (e.g. **/*.rs)"),
    ("Grep",            "Search file contents with regex"),
    ("Read",            "Read file contents"),
    ("Write",           "Write/create files"),
    ("WebFetch",        "Fetch content from a URL"),
    ("WebSearch",       "Search the web"),
    ("Task",            "Spawn a sub-agent for parallel tasks"),
    ("TaskOutput",      "Read output from a running sub-agent"),
    ("TaskStop",        "Stop a running sub-agent"),
    ("AskUserQuestion", "Ask the user a clarifying question"),
    ("Skill",           "Invoke a skill by name"),
    ("ToolSearch",      "Search available tools by keyword"),
    ("LSP",             "Language server protocol operations"),
];

// Tool names only, for compatibility
const ALL_BUILTIN_TOOL_NAMES: &[&str] = &[
    "Bash", "Edit", "Glob", "Grep", "Read", "Write",
    "WebFetch", "WebSearch", "Task", "TaskOutput", "TaskStop",
    "AskUserQuestion", "Skill", "ToolSearch", "LSP",
];

struct AgentInfo {
    id: String,
    workspace: PathBuf,
    is_default: bool,
    soul_preview: String,
    allowed: Vec<String>,
    denied: Vec<String>,
    model: Option<String>,
    fallback_model: Option<String>,
    /// Tools that require approval before execution
    approval_tools: Vec<String>,
}

fn load_skill_entries(agent_workspace: &std::path::Path, workspace_root: &std::path::Path) -> Vec<SkillEntry> {
    AgentLoader::list_skills(agent_workspace, workspace_root)
        .into_iter()
        .map(|info| SkillEntry { name: info.name, is_enabled: info.is_enabled, description: info.description })
        .collect()
}

struct SkillEntry {
    name: String,
    is_enabled: bool,
    description: String,
}

#[derive(Debug, Clone, PartialEq)]
enum SkillInputMode {
    Normal,
    Install,
    ConfirmUninstall,
}

const KNOWN_MODELS: &[(&str, &str)] = &[
    ("claude-opus-4-6",        "Opus 4.6 — most capable"),
    ("claude-sonnet-4-6",      "Sonnet 4.6 — balanced"),
    ("claude-haiku-4-5-20251001", "Haiku 4.5 — fastest"),
    ("opus",                   "alias → claude-opus-4-6"),
    ("sonnet",                 "alias → claude-sonnet-4-6"),
    ("haiku",                  "alias → claude-haiku-4-5-20251001"),
];

const AGENT_FILES: &[(&str, &str)] = &[
    ("SOUL.md", "Soul / Personality"),
    ("USER.md", "User profile"),
    ("IDENTITY.md", "Identity"),
    ("BOOT.md", "Boot prompt"),
    ("HEARTBEAT.md", "Heartbeat task"),
    ("MEMORY.md", "Long-term memory"),
    ("TOOLS.md", "Tool guidelines"),
    ("AGENTS.md", "Agent roster"),
];

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    /// Browsing agent list
    Normal,
    /// Editing tool permissions for selected agent
    Tools,
    /// Selecting which file to edit
    SelectFile,
    /// Full-screen markdown editor
    EditMd,
    /// Confirm agent deletion
    ConfirmDelete,
    /// Editing model or fallback_model for selected agent
    EditModel,
    /// Typing new agent name
    CreateAgent,
    /// Managing skills for selected agent
    Skills,
}

/// Which section a tool belongs to in the tools editor
#[derive(Debug, Clone, PartialEq)]
enum ToolSection {
    /// Claude Code built-in tool (managed via --tools / --disallowedTools)
    BuiltIn,
    /// CatClaw built-in MCP server tool (mcp__catclaw__*)
    McpCatclaw,
    /// User-defined MCP server tool from agent .mcp.json (mcp__{server}__*)
    McpUser,
}

/// A tool entry for the tools editor
struct ToolEntry {
    name: String,
    /// Display name (short form for MCP tools)
    display: String,
    /// Which section this tool belongs to
    section: ToolSection,
    /// true = in allowed list (whitelist, built-in only)
    allowed: bool,
    /// true = in denied list (blacklist, works for both built-in and MCP)
    denied: bool,
    /// true = requires user approval before each execution
    require_approval: bool,
}

pub struct AgentsPanel {
    agents: Vec<AgentInfo>,
    selected: usize,
    mode: Mode,
    /// Tool entries for the currently editing agent
    tool_entries: Vec<ToolEntry>,
    tool_selected: usize,
    status_msg: Option<String>,
    /// Whether allowed list is a whitelist (non-empty) or unrestricted (empty)
    whitelist_mode: bool,
    /// Path to catclaw.toml for saving config changes
    config_path: PathBuf,
    /// Full-screen SOUL.md editor
    editor: Option<MdEditor<'static>>,
    /// Path of the file being edited
    edit_path: Option<PathBuf>,
    /// Whether the editor content has been modified
    edit_modified: bool,
    /// Model edit: 0 = model, 1 = fallback_model
    model_edit_field: usize,
    /// Model edit buffer
    model_edit_buffer: String,
    /// Filtered model completions index
    model_completion_idx: usize,
    /// Selected file index in SelectFile mode
    file_selected: usize,
    /// Name input buffer for creating a new agent
    create_buf: String,
    /// Channel for async agent creation result: Ok(agent_id) or Err(msg)
    create_tx: mpsc::UnboundedSender<Result<String, String>>,
    create_rx: mpsc::UnboundedReceiver<Result<String, String>>,
    /// Workspace root (for creating agent dirs and shared skills pool)
    workspace: PathBuf,
    /// Skills for the currently selected agent (loaded on entry to Skills mode)
    skill_entries: Vec<SkillEntry>,
    skill_selected: usize,
    skill_install_buf: String,
    skill_mode: SkillInputMode,
    skill_status: Option<String>,
    /// Channel for async skill install results
    skill_tx: mpsc::UnboundedSender<Result<String, String>>,
    skill_rx: mpsc::UnboundedReceiver<Result<String, String>>,
    /// WS client for hot-reloading agent config in gateway
    client: Arc<GatewayClient>,
    /// Discovered MCP tools per server (fetched from gateway on Tools mode entry)
    mcp_discovered_tools: std::collections::HashMap<String, Vec<String>>,
    /// Receiver for async MCP tools discovery results
    mcp_tools_rx: mpsc::UnboundedReceiver<std::collections::HashMap<String, Vec<String>>>,
    mcp_tools_tx: mpsc::UnboundedSender<std::collections::HashMap<String, Vec<String>>>,
}

impl AgentsPanel {
    pub fn new(config: &Config, config_path: PathBuf, client: Arc<GatewayClient>) -> Self {
        let agents = Self::load_agents(config);
        let (create_tx, create_rx) = mpsc::unbounded_channel();
        let (skill_tx, skill_rx) = mpsc::unbounded_channel();
        let (mcp_tools_tx, mcp_tools_rx) = mpsc::unbounded_channel();

        // Pre-fetch MCP discovered tools so first Tools mode entry has data
        {
            let client = client.clone();
            let tx = mcp_tools_tx.clone();
            tokio::spawn(async move {
                if let Ok(resp) = client.request("mcp.tools", serde_json::json!({})).await {
                    if let Some(obj) = resp.as_object() {
                        let mut map = std::collections::HashMap::new();
                        for (server, tools) in obj {
                            if let Some(arr) = tools.as_array() {
                                let names: Vec<String> = arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect();
                                map.insert(server.clone(), names);
                            }
                        }
                        let _ = tx.send(map);
                    }
                }
            });
        }

        AgentsPanel {
            agents,
            selected: 0,
            mode: Mode::Normal,
            tool_entries: Vec::new(),
            tool_selected: 0,
            status_msg: None,
            whitelist_mode: false,
            config_path,
            editor: None,
            edit_path: None,
            edit_modified: false,
            model_edit_field: 0,
            model_edit_buffer: String::new(),
            model_completion_idx: 0,
            file_selected: 0,
            create_buf: String::new(),
            create_tx,
            create_rx,
            workspace: config.general.workspace.clone(),
            skill_entries: Vec::new(),
            skill_selected: 0,
            skill_install_buf: String::new(),
            skill_mode: SkillInputMode::Normal,
            skill_status: None,
            skill_tx,
            skill_rx,
            client,
            mcp_discovered_tools: std::collections::HashMap::new(),
            mcp_tools_rx,
            mcp_tools_tx,
        }
    }

    fn load_agents(config: &Config) -> Vec<AgentInfo> {
        config
            .agents
            .iter()
            .map(|a| {
                let soul = std::fs::read_to_string(a.workspace.join("SOUL.md"))
                    .unwrap_or_default();
                let soul_preview: String = soul.lines().take(3).collect::<Vec<_>>().join(" ");

                let (allowed, denied) = Self::read_tools_toml(&a.workspace);

                AgentInfo {
                    id: a.id.clone(),
                    workspace: a.workspace.clone(),
                    is_default: a.default,
                    soul_preview,
                    allowed,
                    denied,
                    model: a.model.clone(),
                    fallback_model: a.fallback_model.clone(),
                    approval_tools: {
                        let content = std::fs::read_to_string(a.workspace.join("tools.toml")).unwrap_or_default();
                        toml::from_str::<toml::Value>(&content).ok()
                            .and_then(|v| v.get("require_approval")?.as_array().map(|arr| {
                                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                            }))
                            .unwrap_or_default()
                    },
                }
            })
            .collect()
    }

    fn read_tools_toml(workspace: &std::path::Path) -> (Vec<String>, Vec<String>) {
        let content =
            std::fs::read_to_string(workspace.join("tools.toml")).unwrap_or_default();
        if let Ok(parsed) = toml::from_str::<toml::Value>(&content) {
            let allowed = parsed
                .get("allowed")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let denied = parsed
                .get("denied")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            (allowed, denied)
        } else {
            (vec![], vec![])
        }
    }

    fn enter_tools_mode(&mut self) {
        // Drain any pending MCP tools discovery results
        while let Ok(tools) = self.mcp_tools_rx.try_recv() {
            self.mcp_discovered_tools = tools;
        }

        // Trigger a fresh fetch for next time
        {
            let client = self.client.clone();
            let tx = self.mcp_tools_tx.clone();
            tokio::spawn(async move {
                if let Ok(resp) = client.request("mcp.tools", serde_json::json!({})).await {
                    if let Some(obj) = resp.as_object() {
                        let mut map = std::collections::HashMap::new();
                        for (server, tools) in obj {
                            if let Some(arr) = tools.as_array() {
                                let names: Vec<String> = arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect();
                                map.insert(server.clone(), names);
                            }
                        }
                        let _ = tx.send(map);
                    }
                }
            });
        }

        if let Some(agent) = self.agents.get(self.selected) {
            self.whitelist_mode = !agent.allowed.is_empty();

            // Load approval list from agent's tools.toml (same file as allowed/denied)
            let approval_list: Vec<String> = {
                let content = std::fs::read_to_string(agent.workspace.join("tools.toml")).unwrap_or_default();
                toml::from_str::<toml::Value>(&content).ok()
                    .and_then(|v| v.get("require_approval")?.as_array().map(|arr| {
                        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                    }))
                    .unwrap_or_default()
            };

            let mut entries: Vec<ToolEntry> = Vec::new();

            // 1. Built-in tools
            for &name in ALL_BUILTIN_TOOL_NAMES {
                entries.push(ToolEntry {
                    name: name.to_string(),
                    display: name.to_string(),
                    section: ToolSection::BuiltIn,
                    allowed: agent.allowed.is_empty() || agent.allowed.iter().any(|a| a == name),
                    denied: agent.denied.iter().any(|d| d == name),
                    require_approval: approval_list.iter().any(|p| crate::config::ApprovalConfig::matches_pattern(p, name)),
                });
            }

            // 2. CatClaw built-in MCP tools (from configured channel adapters)
            let config = Config::load(&self.config_path).ok();
            let catclaw_mcp_tools = Self::list_catclaw_mcp_tools(config.as_ref());
            for tool_name in &catclaw_mcp_tools {
                let display = tool_name.strip_prefix("mcp__catclaw__").unwrap_or(tool_name).to_string();
                entries.push(ToolEntry {
                    name: tool_name.clone(),
                    display,
                    section: ToolSection::McpCatclaw,
                    allowed: true, // MCP tools don't use --tools whitelist
                    denied: agent.denied.iter().any(|d| {
                        crate::config::ApprovalConfig::matches_pattern(d, tool_name)
                    }),
                    require_approval: approval_list.iter().any(|p| crate::config::ApprovalConfig::matches_pattern(p, tool_name)),
                });
            }

            // 3. User MCP tools (from shared workspace .mcp.json)
            // If we have discovered individual tools, show them; otherwise fallback to wildcard
            let user_mcp_servers = Self::list_user_mcp_servers(&self.workspace);
            for server_name in &user_mcp_servers {
                if let Some(discovered) = self.mcp_discovered_tools.get(server_name) {
                    // Show individual discovered tools
                    for tool_name_raw in discovered {
                        let full_name = format!("mcp__{}__{}", server_name, tool_name_raw);
                        entries.push(ToolEntry {
                            name: full_name.clone(),
                            display: tool_name_raw.clone(),
                            section: ToolSection::McpUser,
                            allowed: true,
                            denied: agent.denied.iter().any(|d| {
                                crate::config::ApprovalConfig::matches_pattern(d, &full_name)
                            }),
                            require_approval: approval_list.iter().any(|p| {
                                crate::config::ApprovalConfig::matches_pattern(p, &full_name)
                            }),
                        });
                    }
                } else {
                    // Fallback: show wildcard
                    let wildcard = format!("mcp__{}__*", server_name);
                    let display = format!("{}  (all tools)", server_name);
                    entries.push(ToolEntry {
                        name: wildcard.clone(),
                        display,
                        section: ToolSection::McpUser,
                        allowed: true,
                        denied: agent.denied.iter().any(|d| {
                            d == &wildcard || d == &format!("mcp__{}__*", server_name)
                        }),
                        require_approval: approval_list.iter().any(|p| {
                            crate::config::ApprovalConfig::matches_pattern(p, &wildcard)
                        }),
                    });
                }
            }

            self.tool_entries = entries;
            self.tool_selected = 0;
            self.mode = Mode::Tools;
            self.status_msg = None;
        }
    }

    /// List CatClaw built-in MCP tool names based on configured channel adapters.
    fn list_catclaw_mcp_tools(config: Option<&Config>) -> Vec<String> {
        let config = match config {
            Some(c) => c,
            None => return vec![],
        };
        let mut tools = Vec::new();
        for ch in &config.channels {
            let adapter_name = &ch.channel_type;
            // Known adapter actions — we keep a static list here since we can't
            // query the actual adapter (it requires a running gateway).
            let actions: &[&str] = match adapter_name.as_str() {
                "discord" => &[
                    "get_messages", "send_message", "edit_message", "delete_message",
                    "react", "get_reactions", "delete_reaction",
                    "pin_message", "unpin_message", "list_pins",
                    "create_thread", "list_threads",
                    "get_channels", "channel_info", "create_channel", "create_category",
                    "edit_channel", "delete_channel", "edit_permissions",
                    "get_guilds", "get_guild_info", "member_info",
                    "get_roles", "create_role", "edit_role", "delete_role", "assign_role", "remove_role",
                    "timeout_member", "kick_member", "ban_member", "unban_member",
                    "list_emojis", "list_stickers",
                    "list_events", "get_event",
                ],
                "telegram" => &[
                    "send_message", "edit_message", "delete_message",
                    "forward_message", "copy_message",
                    "pin_message", "unpin_message", "unpin_all",
                    "get_chat", "get_chat_member_count", "get_chat_member", "get_chat_administrators",
                    "set_chat_title", "set_chat_description",
                    "ban_member", "unban_member", "restrict_member", "promote_member",
                    "send_poll", "stop_poll",
                    "create_forum_topic", "close_forum_topic", "reopen_forum_topic", "delete_forum_topic",
                    "set_chat_permissions",
                    "create_invite_link",
                ],
                "slack" => &[
                    "send_message", "edit_message", "delete_message", "get_messages",
                    "react", "delete_reaction", "get_reactions",
                    "pin_message", "unpin_message", "list_pins",
                    "get_channels", "channel_info", "create_channel", "archive_channel",
                    "get_thread_replies",
                    "user_info", "list_users",
                ],
                _ => &[],
            };
            for action in actions {
                tools.push(format!("mcp__catclaw__{}_{}", adapter_name, action));
            }
        }
        tools
    }

    /// List user-defined MCP server names from shared workspace .mcp.json.
    fn list_user_mcp_servers(workspace_root: &std::path::Path) -> Vec<String> {
        let mcp_path = workspace_root.join(".mcp.json");
        let mut servers = std::collections::BTreeSet::new();
        if let Ok(content) = std::fs::read_to_string(mcp_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(srv) = parsed.get("mcpServers").and_then(|v| v.as_object()) {
                    servers.extend(srv.keys().cloned());
                }
            }
        }
        // Exclude "catclaw" — that's the built-in MCP, already shown in CatClaw MCP section
        servers.remove("catclaw");
        servers.into_iter().collect()
    }

    /// Scroll the active list up by `n` items (for mouse wheel).
    pub fn scroll_up(&mut self, n: usize) {
        match self.mode {
            Mode::Tools => {
                self.tool_selected = self.tool_selected.saturating_sub(n);
            }
            Mode::Normal => {
                self.selected = self.selected.saturating_sub(n);
            }
            Mode::Skills => {
                self.skill_selected = self.skill_selected.saturating_sub(n);
            }
            _ => {}
        }
    }

    /// Scroll the active list down by `n` items (for mouse wheel).
    pub fn scroll_down(&mut self, n: usize) {
        match self.mode {
            Mode::Tools => {
                if !self.tool_entries.is_empty() {
                    self.tool_selected = (self.tool_selected + n).min(self.tool_entries.len() - 1);
                }
            }
            Mode::Normal => {
                if !self.agents.is_empty() {
                    self.selected = (self.selected + n).min(self.agents.len() - 1);
                }
            }
            Mode::Skills => {
                if !self.skill_entries.is_empty() {
                    self.skill_selected = (self.skill_selected + n).min(self.skill_entries.len() - 1);
                }
            }
            _ => {}
        }
    }

    fn toggle_tool(&mut self) {
        if let Some(entry) = self.tool_entries.get_mut(self.tool_selected) {
            if entry.denied {
                // denied → allowed
                entry.denied = false;
                entry.allowed = true;
                entry.require_approval = false;
            } else if entry.require_approval {
                // approval → denied
                entry.require_approval = false;
                entry.denied = true;
                entry.allowed = false;
            } else if entry.allowed {
                // allowed → approval (stays in allowed list but needs approval)
                entry.require_approval = true;
            } else {
                // neither → allowed
                entry.allowed = true;
            }
        }
    }

    fn save_tools(&mut self) {
        if let Some(agent) = self.agents.get_mut(self.selected) {
            // Built-in tools: allowed (whitelist for --tools) and denied (blacklist)
            let builtin_allowed: Vec<String> = self.tool_entries.iter()
                .filter(|e| e.section == ToolSection::BuiltIn && e.allowed && !e.denied)
                .map(|e| e.name.clone())
                .collect();
            let builtin_denied: Vec<String> = self.tool_entries.iter()
                .filter(|e| e.section == ToolSection::BuiltIn && e.denied)
                .map(|e| e.name.clone())
                .collect();

            // MCP tools: only denied (--disallowedTools) since --tools doesn't affect MCP
            let mcp_denied: Vec<String> = self.tool_entries.iter()
                .filter(|e| e.section != ToolSection::BuiltIn && e.denied)
                .map(|e| e.name.clone())
                .collect();

            // All denied = built-in denied + MCP denied
            let all_denied: Vec<String> = builtin_denied.iter()
                .chain(mcp_denied.iter())
                .cloned()
                .collect();

            // All approval tools (built-in + MCP)
            let approval_tools: Vec<String> = self.tool_entries.iter()
                .filter(|e| e.require_approval)
                .map(|e| e.name.clone())
                .collect();

            // If all built-in tools are allowed and none denied, write empty allowed (= unrestricted)
            let all_builtin_allowed = builtin_allowed.len() == ALL_BUILTIN_TOOL_NAMES.len();
            let final_allowed = if all_builtin_allowed && builtin_denied.is_empty() {
                vec![]
            } else {
                builtin_allowed
            };

            // Write everything to tools.toml (allowed, denied, require_approval)
            let mut content = format!(
                "allowed = [{}]\ndenied = [{}]\n",
                final_allowed.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(", "),
                all_denied.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(", "),
            );
            if !approval_tools.is_empty() {
                content.push_str(&format!(
                    "require_approval = [{}]\n",
                    approval_tools.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(", "),
                ));
            }

            let tools_path = agent.workspace.join("tools.toml");
            if std::fs::write(&tools_path, &content).is_ok() {
                agent.allowed = final_allowed;
                agent.denied = all_denied;
                agent.approval_tools = approval_tools;
                self.status_msg = Some(format!("Tools saved for '{}'", agent.id));
            } else {
                self.status_msg = Some("Failed to save tools.toml".into());
                self.mode = Mode::Normal;
                return;
            }

            // Hot-reload in gateway via WS
            let client = self.client.clone();
            let aid = agent.id.clone();
            tokio::spawn(async move {
                let _ = client.request(
                    "agents.reload_tools",
                    serde_json::json!({"agent_id": aid}),
                ).await;
            });
        }
        self.mode = Mode::Normal;
    }

    fn enter_edit_mode(&mut self) {
        if self.agents.get(self.selected).is_none() {
            return;
        }
        self.file_selected = 0;
        self.mode = Mode::SelectFile;
        self.status_msg = None;
    }

    fn enter_edit_for_file(&mut self) {
        let Some(agent) = self.agents.get(self.selected) else {
            return;
        };
        let (filename, _) = AGENT_FILES[self.file_selected];
        let file_path = agent.workspace.join(filename);
        let content = std::fs::read_to_string(&file_path).unwrap_or_default();
        let file_display = file_path.display().to_string();

        let editor = MdEditor::new(&agent.id, &file_display, &content);
        self.editor = Some(editor);
        self.edit_path = Some(file_path);
        self.edit_modified = false;
        self.mode = Mode::EditMd;
        self.status_msg = None;
    }

    fn save_editor(&mut self) {
        let Some(editor) = &self.editor else { return };
        let Some(path) = &self.edit_path else { return };

        let content = editor.content();
        if std::fs::write(path, &content).is_ok() {
            self.edit_modified = false;
            self.status_msg = Some(format!("Saved {}", path.display()));

            // Update the soul preview if we edited SOUL.md
            if path.file_name().is_some_and(|n| n == "SOUL.md") {
                if let Some(agent) = self.agents.get_mut(self.selected) {
                    agent.soul_preview = content.lines().take(3).collect::<Vec<_>>().join(" ");
                }
            }
        } else {
            self.status_msg = Some(format!("Failed to save {}", path.display()));
        }
    }

    fn close_editor(&mut self) {
        self.editor = None;
        self.edit_path = None;
        self.edit_modified = false;
        self.mode = Mode::Normal;
    }

    fn filtered_models(&self) -> Vec<&(&'static str, &'static str)> {
        let q = self.model_edit_buffer.to_lowercase();
        KNOWN_MODELS
            .iter()
            .filter(|(id, desc)| {
                q.is_empty() || id.contains(q.as_str()) || desc.to_lowercase().contains(q.as_str())
            })
            .collect()
    }

    fn accept_model_completion(&mut self) {
        let models = self.filtered_models();
        if let Some(&(id, _)) = models.get(self.model_completion_idx) {
            self.model_edit_buffer = id.to_string();
            self.model_completion_idx = 0;
        }
    }

    fn enter_model_edit(&mut self) {
        if let Some(agent) = self.agents.get(self.selected) {
            self.model_edit_field = 0;
            self.model_edit_buffer = agent.model.clone().unwrap_or_default();
            self.model_completion_idx = 0;
            self.mode = Mode::EditModel;
            self.status_msg = None;
        }
    }

    fn save_model_edit(&mut self) {
        let value = self.model_edit_buffer.trim().to_string();
        let Some(agent) = self.agents.get_mut(self.selected) else {
            self.mode = Mode::Normal;
            return;
        };
        let agent_id = agent.id.clone();

        let new_value = if value.is_empty() { None } else { Some(value.clone()) };

        if self.model_edit_field == 0 {
            agent.model = new_value.clone();
        } else {
            agent.fallback_model = new_value.clone();
        }

        // Save to catclaw.toml
        match Config::load(&self.config_path) {
            Ok(mut config) => {
                if let Some(ac) = config.agents.iter_mut().find(|a| a.id == agent_id) {
                    if self.model_edit_field == 0 {
                        ac.model = new_value;
                    } else {
                        ac.fallback_model = new_value;
                    }
                }
                match config.save(&self.config_path) {
                    Ok(()) => {
                        let field_name = if self.model_edit_field == 0 { "model" } else { "fallback_model" };
                        let display = if value.is_empty() { "(cleared)" } else { &value };
                        self.status_msg = Some(format!("{} {} = {}", agent_id, field_name, display));

                        // Hot-reload in gateway via WS
                        let client = self.client.clone();
                        let aid = agent_id.clone();
                        tokio::spawn(async move {
                            let _ = client.request(
                                "agents.reload_tools",
                                serde_json::json!({"agent_id": aid}),
                            ).await;
                        });
                    }
                    Err(e) => {
                        self.status_msg = Some(format!("Failed to save: {}", e));
                    }
                }
            }
            Err(e) => {
                self.status_msg = Some(format!("Failed to load config: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }

    fn start_create_agent(&mut self) {
        self.create_buf.clear();
        self.mode = Mode::CreateAgent;
        self.status_msg = None;
    }

    fn submit_create_agent(&mut self) {
        let name = self.create_buf.trim().to_string();
        if name.is_empty() {
            self.mode = Mode::Normal;
            return;
        }
        if self.agents.iter().any(|a| a.id == name) {
            self.status_msg = Some(format!("Agent '{}' already exists", name));
            self.mode = Mode::Normal;
            return;
        }

        let workspace = self.workspace.join("agents").join(&name);
        let workspace_root = self.workspace.clone();
        let config_path = self.config_path.clone();
        let tx = self.create_tx.clone();

        self.status_msg = Some(format!("Creating agent '{}' ...", name));
        self.mode = Mode::Normal;

        tokio::spawn(async move {
            let result: Result<String, String> = async {
                AgentLoader::create_workspace(&workspace, &workspace_root, &name)
                    .map_err(|e| e.to_string())?;
                AgentLoader::install_remote_skills(&workspace_root).await
                    .map_err(|e: crate::error::CatClawError| e.to_string())?;

                let mut config = Config::load(&config_path)
                    .map_err(|e| e.to_string())?;
                if !config.agents.iter().any(|a| a.id == name) {
                    config.agents.push(AgentConfig {
                        id: name.clone(),
                        workspace,
                        default: false,
                        model: None,
                        fallback_model: None,
                        approval: crate::config::ApprovalConfig::default(),
                    });
                    config.save(&config_path).map_err(|e| e.to_string())?;
                }
                Ok(name)
            }.await;
            let _ = tx.send(result);
        });
    }

    fn poll_create(&mut self) {
        while let Ok(result) = self.create_rx.try_recv() {
            match result {
                Ok(name) => {
                    // Reload agent list from config
                    if let Ok(config) = Config::load(&self.config_path) {
                        self.agents = Self::load_agents(&config);
                        self.workspace = config.general.workspace.clone();
                        // Select the newly created agent
                        if let Some(idx) = self.agents.iter().position(|a| a.id == name) {
                            self.selected = idx;
                        }
                    }
                    self.status_msg = Some(format!("Agent '{}' created", name));
                }
                Err(e) => {
                    self.status_msg = Some(format!("Failed to create agent: {}", e));
                }
            }
        }
    }

    fn enter_skills_mode(&mut self) {
        if let Some(agent) = self.agents.get(self.selected) {
            self.skill_entries = load_skill_entries(&agent.workspace, &self.workspace);
            self.skill_selected = 0;
            self.skill_mode = SkillInputMode::Normal;
            self.skill_status = None;
            self.mode = Mode::Skills;
        }
    }

    fn skill_toggle(&mut self) {
        if let Some(agent) = self.agents.get(self.selected) {
            if let Some(skill) = self.skill_entries.get_mut(self.skill_selected) {
                let new_state = !skill.is_enabled;
                if AgentLoader::set_skill_enabled(&agent.workspace, &self.workspace, &skill.name, new_state).is_ok() {
                    skill.is_enabled = new_state;
                    self.skill_status = Some(format!(
                        "'{}' {}",
                        skill.name,
                        if new_state { "enabled" } else { "disabled" }
                    ));
                }
            }
        }
    }

    fn skill_start_install(&mut self) {
        self.skill_install_buf.clear();
        self.skill_mode = SkillInputMode::Install;
        self.skill_status = None;
    }

    fn skill_confirm_install(&mut self) {
        let src_str = self.skill_install_buf.trim().to_string();
        if src_str.is_empty() {
            self.skill_mode = SkillInputMode::Normal;
            return;
        }
        if self.agents.get(self.selected).is_some() {
            let source = match SkillSource::parse(&src_str) {
                Ok(s) => s,
                Err(e) => {
                    self.skill_status = Some(format!("Invalid source: {}", e));
                    self.skill_mode = SkillInputMode::Normal;
                    return;
                }
            };
            let workspace_root = self.workspace.clone();
            let tx = self.skill_tx.clone();
            tokio::spawn(async move {
                match AgentLoader::install_skill(&workspace_root, &source).await {
                    Ok(()) => {
                        let name = match &source {
                            SkillSource::Anthropic(n) => n.clone(),
                            SkillSource::GitHub { path, .. } => {
                                std::path::Path::new(path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(path)
                                    .to_string()
                            }
                            SkillSource::Local(p) => {
                                std::path::Path::new(p)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("skill")
                                    .to_string()
                            }
                        };
                        let _ = tx.send(Ok(name));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            });
            self.skill_status = Some("Installing…".to_string());
        }
        self.skill_mode = SkillInputMode::Normal;
    }

    fn skill_start_uninstall(&mut self) {
        if !self.skill_entries.is_empty() {
            self.skill_mode = SkillInputMode::ConfirmUninstall;
        }
    }

    fn skill_confirm_uninstall(&mut self) {
        // Extract needed data before mutable borrows
        let info = self.agents.get(self.selected).and_then(|agent| {
            self.skill_entries.get(self.skill_selected).map(|skill| {
                (agent.workspace.clone(), skill.name.clone())
            })
        });
        if let Some((agent_workspace, skill_name)) = info {
            match AgentLoader::uninstall_skill(&self.workspace, &skill_name) {
                Ok(()) => {
                    self.skill_entries = load_skill_entries(&agent_workspace, &self.workspace);
                    if self.skill_selected >= self.skill_entries.len() && !self.skill_entries.is_empty() {
                        self.skill_selected = self.skill_entries.len() - 1;
                    }
                    self.skill_status = Some(format!("Uninstalled '{}'", skill_name));
                }
                Err(e) => {
                    self.skill_status = Some(format!("Uninstall failed: {}", e));
                }
            }
        }
        self.skill_mode = SkillInputMode::Normal;
    }

    fn poll_skills(&mut self) {
        while let Ok(result) = self.skill_rx.try_recv() {
            match result {
                Ok(name) => {
                    if let Some(agent) = self.agents.get(self.selected) {
                        self.skill_entries = load_skill_entries(&agent.workspace, &self.workspace);
                    }
                    self.skill_status = Some(format!("Installed '{}'", name));
                }
                Err(e) => {
                    self.skill_status = Some(format!("Install failed: {}", e));
                }
            }
        }
    }

    fn delete_selected(&mut self) {
        let Some(agent) = self.agents.get(self.selected) else {
            self.mode = Mode::Normal;
            return;
        };

        if agent.is_default {
            self.status_msg = Some("Cannot delete the default agent.".to_string());
            self.mode = Mode::Normal;
            return;
        }

        let agent_id = agent.id.clone();

        let config_result = Config::load(&self.config_path);
        match config_result {
            Ok(mut config) => {
                config.agents.retain(|a| a.id != agent_id);
                match config.save(&self.config_path) {
                    Ok(()) => {
                        self.agents.retain(|a| a.id != agent_id);
                        if self.selected >= self.agents.len() && !self.agents.is_empty() {
                            self.selected = self.agents.len() - 1;
                        }
                        self.status_msg =
                            Some(format!("Deleted agent '{}' from config", agent_id));
                    }
                    Err(e) => {
                        self.status_msg = Some(format!("Failed to save config: {}", e));
                    }
                }
            }
            Err(e) => {
                self.status_msg = Some(format!("Failed to load config: {}", e));
            }
        }

        self.mode = Mode::Normal;
    }
}

impl Component for AgentsPanel {
    fn handle_event(&mut self, event: &KeyEvent) -> Action {
        match &self.mode {
            Mode::Normal => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.agents.is_empty() {
                        self.selected = (self.selected + 1).min(self.agents.len() - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Char('t') => {
                    self.enter_tools_mode();
                    Action::None
                }
                KeyCode::Char('e') => {
                    self.enter_edit_mode();
                    Action::None
                }
                KeyCode::Char('m') => {
                    self.enter_model_edit();
                    Action::None
                }
                KeyCode::Char('s') => {
                    self.enter_skills_mode();
                    Action::None
                }
                KeyCode::Char('n') => {
                    self.start_create_agent();
                    Action::None
                }
                KeyCode::Char('d') => {
                    if let Some(a) = self.agents.get(self.selected) {
                        if a.is_default {
                            self.status_msg = Some("Cannot delete the default agent.".to_string());
                        } else {
                            self.mode = Mode::ConfirmDelete;
                            self.status_msg = None;
                        }
                    } else if !self.agents.is_empty() {
                        self.mode = Mode::ConfirmDelete;
                        self.status_msg = None;
                    }
                    Action::None
                }
                _ => Action::None,
            },
            Mode::SelectFile => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.file_selected = (self.file_selected + 1).min(AGENT_FILES.len() - 1);
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.file_selected = self.file_selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::Enter => {
                    self.enter_edit_for_file();
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.status_msg = None;
                    Action::None
                }
                _ => Action::None,
            },
            Mode::Tools => match event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.tool_entries.is_empty() {
                        self.tool_selected =
                            (self.tool_selected + 1).min(self.tool_entries.len() - 1);
                    }
                    Action::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.tool_selected = self.tool_selected.saturating_sub(1);
                    Action::None
                }
                KeyCode::PageDown | KeyCode::Char('d') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    if !self.tool_entries.is_empty() {
                        self.tool_selected =
                            (self.tool_selected + 15).min(self.tool_entries.len() - 1);
                    }
                    Action::None
                }
                KeyCode::PageUp | KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.tool_selected = self.tool_selected.saturating_sub(15);
                    Action::None
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    self.tool_selected = 0;
                    Action::None
                }
                KeyCode::End | KeyCode::Char('G') => {
                    if !self.tool_entries.is_empty() {
                        self.tool_selected = self.tool_entries.len() - 1;
                    }
                    Action::None
                }
                KeyCode::Char(' ') => {
                    self.toggle_tool();
                    Action::None
                }
                KeyCode::Enter => {
                    self.save_tools();
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.status_msg = Some("Cancelled".into());
                    Action::None
                }
                _ => Action::None,
            },
            Mode::EditMd => {
                match (event.modifiers, event.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                        self.save_editor();
                    }
                    (KeyModifiers::CONTROL, KeyCode::Char('q')) | (_, KeyCode::Esc) => {
                        self.close_editor();
                    }
                    _ => {
                        if let Some(editor) = &mut self.editor {
                            let action = editor.handle_event(event);
                            match action {
                                EditorAction::Save => self.save_editor(),
                                EditorAction::Quit => self.close_editor(),
                                EditorAction::None => {
                                    if editor.is_modified() {
                                        self.edit_modified = true;
                                    }
                                }
                            }
                        }
                    }
                }
                Action::None
            }
            Mode::EditModel => {
                let models = self.filtered_models();
                let model_count = models.len();
                match event.code {
                    KeyCode::Enter => {
                        // If completions visible, accept current selection first
                        if model_count > 0 && (!self.model_edit_buffer.is_empty()
                            || self.model_completion_idx < model_count)
                        {
                            // Check if buffer exactly matches a model id — if not, accept completion
                            let exact = models.iter().any(|(id, _)| *id == self.model_edit_buffer);
                            if !exact && model_count > 0 {
                                self.accept_model_completion();
                                return Action::None;
                            }
                        }
                        self.save_model_edit();
                        Action::None
                    }
                    KeyCode::Tab => {
                        if model_count > 0 {
                            // Tab: accept autocomplete
                            self.accept_model_completion();
                        } else {
                            // Tab with no completions: toggle model/fallback field
                            if let Some(agent) = self.agents.get(self.selected) {
                                if self.model_edit_field == 0 {
                                    self.model_edit_field = 1;
                                    self.model_edit_buffer = agent.fallback_model.clone().unwrap_or_default();
                                } else {
                                    self.model_edit_field = 0;
                                    self.model_edit_buffer = agent.model.clone().unwrap_or_default();
                                }
                                self.model_completion_idx = 0;
                            }
                        }
                        Action::None
                    }
                    KeyCode::Down => {
                        if model_count > 0 {
                            self.model_completion_idx = (self.model_completion_idx + 1).min(model_count - 1);
                        }
                        Action::None
                    }
                    KeyCode::Up => {
                        self.model_completion_idx = self.model_completion_idx.saturating_sub(1);
                        Action::None
                    }
                    KeyCode::Char('f') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Ctrl+F: toggle field (model ↔ fallback)
                        if let Some(agent) = self.agents.get(self.selected) {
                            if self.model_edit_field == 0 {
                                self.model_edit_field = 1;
                                self.model_edit_buffer = agent.fallback_model.clone().unwrap_or_default();
                            } else {
                                self.model_edit_field = 0;
                                self.model_edit_buffer = agent.model.clone().unwrap_or_default();
                            }
                            self.model_completion_idx = 0;
                        }
                        Action::None
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        self.status_msg = Some("Cancelled".to_string());
                        Action::None
                    }
                    KeyCode::Backspace => {
                        self.model_edit_buffer.pop();
                        self.model_completion_idx = 0;
                        Action::None
                    }
                    KeyCode::Char(c) => {
                        self.model_edit_buffer.push(c);
                        self.model_completion_idx = 0;
                        Action::None
                    }
                    _ => Action::None,
                }
            }
            Mode::CreateAgent => match event.code {
                KeyCode::Enter => {
                    self.submit_create_agent();
                    Action::None
                }
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.status_msg = None;
                    Action::None
                }
                KeyCode::Backspace => {
                    self.create_buf.pop();
                    Action::None
                }
                KeyCode::Char(c) if c.is_alphanumeric() || c == '-' || c == '_' => {
                    self.create_buf.push(c);
                    Action::None
                }
                _ => Action::None,
            },
            Mode::Skills => match &self.skill_mode {
                SkillInputMode::Install => match event.code {
                    KeyCode::Enter => { self.skill_confirm_install(); Action::None }
                    KeyCode::Esc => { self.skill_mode = SkillInputMode::Normal; Action::None }
                    KeyCode::Backspace => { self.skill_install_buf.pop(); Action::None }
                    KeyCode::Char(c) => { self.skill_install_buf.push(c); Action::None }
                    _ => Action::None,
                },
                SkillInputMode::ConfirmUninstall => match event.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => { self.skill_confirm_uninstall(); Action::None }
                    _ => { self.skill_mode = SkillInputMode::Normal; Action::None }
                },
                SkillInputMode::Normal => match event.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        if !self.skill_entries.is_empty() {
                            self.skill_selected = (self.skill_selected + 1).min(self.skill_entries.len() - 1);
                        }
                        Action::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.skill_selected = self.skill_selected.saturating_sub(1);
                        Action::None
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => { self.skill_toggle(); Action::None }
                    KeyCode::Char('i') => { self.skill_start_install(); Action::None }
                    KeyCode::Char('x') => { self.skill_start_uninstall(); Action::None }
                    KeyCode::Char('r') => {
                        if let Some(agent) = self.agents.get(self.selected) {
                            self.skill_entries = load_skill_entries(&agent.workspace, &self.workspace);
                        }
                        Action::None
                    }
                    KeyCode::Esc => { self.mode = Mode::Normal; Action::None }
                    _ => Action::None,
                },
            },
            Mode::ConfirmDelete => match event.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.delete_selected();
                    Action::None
                }
                _ => {
                    self.mode = Mode::Normal;
                    self.status_msg = Some("Cancelled".to_string());
                    Action::None
                }
            },
        }
    }

    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::EditMd | Mode::ConfirmDelete | Mode::EditModel | Mode::SelectFile | Mode::CreateAgent | Mode::Skills)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.poll_create();
        self.poll_skills();
        match &self.mode {
            Mode::Normal | Mode::ConfirmDelete | Mode::EditModel | Mode::CreateAgent => self.render_normal(frame, area),
            Mode::SelectFile => self.render_select_file(frame, area),
            Mode::Tools => self.render_tools(frame, area),
            Mode::Skills => self.render_skills(frame, area),
            Mode::EditMd => self.render_editor(frame, area),
        }
    }
}

impl AgentsPanel {
    fn render_normal(&mut self, frame: &mut Frame, area: Rect) {
        // Reserve space for model autocomplete popup when editing
        let model_completions = if self.mode == Mode::EditModel {
            self.filtered_models().len().min(6) as u16
        } else {
            0
        };
        let ac_height = if model_completions > 0 { model_completions + 2 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(ac_height),  // autocomplete popup (0 when not editing)
                Constraint::Length(1),          // status
                Constraint::Length(1),          // help
            ])
            .split(area);

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(chunks[0]);

        // Left: agent list
        let items: Vec<ListItem> = self
            .agents
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let default_marker = if a.is_default { " ⭐" } else { "" };
                let style = if i == self.selected {
                    Style::default()
                        .fg(Theme::TEXT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::SUBTEXT0)
                };
                ListItem::new(Line::from(vec![Span::styled(
                    format!("  {}{}", a.id, default_marker),
                    style,
                )]))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(" Agents ")
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(list, main_chunks[0]);

        // Right: agent detail
        let detail = if let Some(agent) = self.agents.get(self.selected) {
            let allowed_str = if agent.allowed.is_empty() {
                "all (unrestricted)".to_string()
            } else {
                agent.allowed.join(", ")
            };
            let denied_str = if agent.denied.is_empty() {
                "none".to_string()
            } else {
                agent.denied.join(", ")
            };

            let model_str = agent.model.as_deref().unwrap_or("(default)");
            let fallback_str = agent.fallback_model.as_deref().unwrap_or("(none)");

            let lines = vec![
                Line::from(vec![
                    Span::styled("ID: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(
                        &agent.id,
                        Style::default()
                            .fg(Theme::MAUVE)
                            .add_modifier(Modifier::BOLD),
                    ),
                    if agent.is_default {
                        Span::styled(" (default)", Style::default().fg(Theme::YELLOW))
                    } else {
                        Span::raw("")
                    },
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Workspace: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(
                        agent.workspace.display().to_string(),
                        Style::default().fg(Theme::SUBTEXT0),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Model:    ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(model_str, Style::default().fg(Theme::PEACH)),
                ]),
                Line::from(vec![
                    Span::styled("Fallback: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(fallback_str, Style::default().fg(Theme::PEACH)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("SOUL.md: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(&agent.soul_preview, Style::default().fg(Theme::TEXT)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Allowed: ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(allowed_str, Style::default().fg(Theme::GREEN)),
                ]),
                Line::from(vec![
                    Span::styled("Denied:  ", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(denied_str, Style::default().fg(Theme::RED)),
                ]),
                Line::from(vec![
                    Span::styled("Approval:", Style::default().fg(Theme::OVERLAY1)),
                    Span::styled(
                        if agent.approval_tools.is_empty() {
                            " none".to_string()
                        } else {
                            format!(" {}", agent.approval_tools.join(", "))
                        },
                        Style::default().fg(Theme::YELLOW),
                    ),
                ]),
            ];
            Paragraph::new(lines)
        } else {
            Paragraph::new("No agents. Run `catclaw agent new <name>` to create one.")
                .style(Style::default().fg(Theme::OVERLAY0))
        };

        let detail = detail.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::SURFACE1))
                .title(" Detail ")
                .title_style(Style::default().fg(Theme::MAUVE)),
        );

        frame.render_widget(detail, main_chunks[1]);

        // Model autocomplete popup
        if self.mode == Mode::EditModel && ac_height > 0 {
            let models = self.filtered_models();
            let items: Vec<ListItem> = models
                .iter()
                .enumerate()
                .map(|(i, (id, desc))| {
                    let is_sel = i == self.model_completion_idx;
                    let icon = if is_sel { "▸ " } else { "  " };
                    let style = if is_sel {
                        Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Theme::SUBTEXT0)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{}{:<30}", icon, id), style),
                        Span::styled(format!("  {}", desc), Style::default().fg(Theme::OVERLAY1)),
                    ]))
                })
                .collect();
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Theme::MAUVE))
                    .title(" Models ")
                    .title_style(Style::default().fg(Theme::MAUVE)),
            );
            frame.render_widget(list, chunks[1]);
        }

        // Status
        let status = if self.mode == Mode::CreateAgent {
            Paragraph::new(Line::from(vec![
                Span::styled(" New agent name: ", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{}▌", self.create_buf),
                    Style::default().fg(Theme::TEXT),
                ),
                Span::styled("  (a-z 0-9 - _)", Style::default().fg(Theme::OVERLAY0)),
            ]))
            .style(Style::default().bg(Theme::SURFACE0))
        } else if self.mode == Mode::ConfirmDelete {
            let name = self
                .agents
                .get(self.selected)
                .map(|a| a.id.as_str())
                .unwrap_or("?");
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" Delete agent '{}'? ", name),
                    Style::default()
                        .fg(Theme::RED)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("(y/N)", Style::default().fg(Theme::OVERLAY0)),
            ]))
            .style(Style::default().bg(Theme::SURFACE0))
        } else if self.mode == Mode::EditModel {
            let field_name = if self.model_edit_field == 0 { "Model" } else { "Fallback" };
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {}: ", field_name),
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}▌", self.model_edit_buffer),
                    Style::default().fg(Theme::TEXT),
                ),
                Span::styled(
                    "  (empty = clear)",
                    Style::default().fg(Theme::OVERLAY0),
                ),
            ]))
            .style(Style::default().bg(Theme::SURFACE0))
        } else if let Some(msg) = &self.status_msg {
            Paragraph::new(format!(" {}", msg))
                .style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
        } else {
            Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
        };
        frame.render_widget(status, chunks[2]);

        // Help bar — use styled spans so active keys are visible
        let help = if self.mode == Mode::ConfirmDelete {
            Paragraph::new(Line::from(vec![
                Span::styled(" y", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
                Span::styled(" Delete  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("any other key", Style::default().fg(Theme::OVERLAY0)),
                Span::styled(" Cancel", Style::default().fg(Theme::OVERLAY1)),
            ]))
        } else if self.mode == Mode::EditModel {
            Paragraph::new(Line::from(vec![
                Span::styled(" Enter", Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
                Span::styled(" Save  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("↑↓", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Select  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("Tab", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Accept  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("Ctrl+F", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Toggle model/fallback  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("Esc", Style::default().fg(Theme::OVERLAY0).add_modifier(Modifier::BOLD)),
                Span::styled(" Cancel", Style::default().fg(Theme::OVERLAY1)),
            ]))
        } else if self.mode == Mode::CreateAgent {
            Paragraph::new(Line::from(vec![
                Span::styled(" Enter", Style::default().fg(Theme::GREEN).add_modifier(Modifier::BOLD)),
                Span::styled(" Create  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("Esc", Style::default().fg(Theme::OVERLAY0).add_modifier(Modifier::BOLD)),
                Span::styled(" Cancel", Style::default().fg(Theme::OVERLAY1)),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(" j/k", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Navigate  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("e", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Edit  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("t", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Tools  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("s", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Skills  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("m", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" Model  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("n", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(" New  ", Style::default().fg(Theme::OVERLAY1)),
                Span::styled("d", Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
                Span::styled(" Delete", Style::default().fg(Theme::OVERLAY1)),
            ]))
        };
        frame.render_widget(help.style(Style::default().bg(Theme::MANTLE)), chunks[3]);
    }

    fn render_tools(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // header
                Constraint::Min(0),    // tool list
                Constraint::Length(1), // selected tool description
                Constraint::Length(1), // help
            ])
            .split(area);

        // Header
        let agent_name = self
            .agents
            .get(self.selected)
            .map(|a| a.id.as_str())
            .unwrap_or("?");

        let header = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    format!("  Tool Permissions for '{}'", agent_name),
                    Style::default()
                        .fg(Theme::MAUVE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  (Space to toggle, Enter to save, Esc to cancel)",
                    Style::default().fg(Theme::OVERLAY0),
                ),
            ]),
        ])
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Theme::SURFACE1)),
        );
        frame.render_widget(header, chunks[0]);

        // Build list items with section headers
        let mut lines: Vec<Line> = Vec::new();
        let mut current_section: Option<&ToolSection> = None;
        // Map from display line index to tool_entries index (for selection tracking)
        let mut line_to_entry: Vec<Option<usize>> = Vec::new();

        for (i, e) in self.tool_entries.iter().enumerate() {
            // Section header
            if current_section != Some(&e.section) {
                if current_section.is_some() {
                    lines.push(Line::from(""));
                    line_to_entry.push(None);
                }
                let section_label = match e.section {
                    ToolSection::BuiltIn => "  Built-in Tools",
                    ToolSection::McpCatclaw => "  CatClaw MCP Tools",
                    ToolSection::McpUser => "  User MCP Servers",
                };
                lines.push(Line::from(Span::styled(
                    section_label,
                    Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD),
                )));
                line_to_entry.push(None);
                current_section = Some(&e.section);
            }

            let is_selected = i == self.tool_selected;

            let (icon, icon_color) = if e.denied {
                ("🚫", Theme::RED)
            } else if e.require_approval {
                ("🔒", Theme::YELLOW)
            } else if e.allowed {
                ("✅", Theme::GREEN)
            } else {
                ("⬜", Theme::OVERLAY0)
            };

            let bg = if is_selected { Theme::SURFACE0 } else { Theme::BASE };

            let name_style = if e.denied {
                Style::default().fg(Theme::OVERLAY0).bg(bg)
            } else {
                Style::default().fg(Theme::TEXT).bg(bg)
            };

            let mut spans = vec![
                Span::styled(format!("    {} ", icon), Style::default().fg(icon_color).bg(bg)),
                Span::styled(&e.display, name_style),
            ];
            if e.denied {
                spans.push(Span::styled(" (denied)", Style::default().fg(Theme::RED).bg(bg)));
            } else if e.require_approval {
                spans.push(Span::styled(" (approval)", Style::default().fg(Theme::YELLOW).bg(bg)));
            }

            lines.push(Line::from(spans));
            line_to_entry.push(Some(i));
        }

        // Find scroll offset to keep selected item visible
        let selected_line = line_to_entry.iter().position(|e| *e == Some(self.tool_selected)).unwrap_or(0);
        let visible_height = chunks[1].height as usize;
        let scroll = if selected_line >= visible_height {
            (selected_line - visible_height + 1) as u16
        } else {
            0
        };

        let tool_list = Paragraph::new(lines).scroll((scroll, 0));
        frame.render_widget(tool_list, chunks[1]);

        // Selected tool description
        let selected_desc = self.tool_entries.get(self.tool_selected)
            .and_then(|e| ALL_BUILTIN_TOOLS.iter().find(|(n, _)| *n == e.name))
            .map(|(_, desc)| *desc)
            .unwrap_or_else(|| {
                // For MCP tools, show the full tool name as description
                self.tool_entries.get(self.tool_selected)
                    .map(|e| e.name.as_str())
                    .unwrap_or("")
            });
        let desc_line = Paragraph::new(Line::from(vec![
            Span::styled(" → ", Style::default().fg(Theme::MAUVE)),
            Span::styled(selected_desc, Style::default().fg(Theme::SUBTEXT0)),
        ])).style(Style::default().bg(Theme::MANTLE));
        frame.render_widget(desc_line, chunks[2]);

        // Help
        let help = Paragraph::new(
            " Space Toggle  j/k ↑↓  PgDn/PgUp  g/G Home/End  │  Enter Save  Esc Cancel",
        )
        .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[3]);
    }

    fn render_select_file(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);

        let agent_name = self.agents.get(self.selected).map(|a| a.id.as_str()).unwrap_or("?");

        let items: Vec<ListItem> = AGENT_FILES
            .iter()
            .enumerate()
            .map(|(i, (filename, desc))| {
                let is_sel = i == self.file_selected;
                let style = if is_sel {
                    Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Theme::SUBTEXT0)
                };
                let icon = if is_sel { "▸ " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{}{:<14}", icon, filename), style),
                    Span::styled(format!("  {}", desc), Style::default().fg(Theme::OVERLAY1)),
                ]))
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::MAUVE))
                .title(format!(" Edit file — {} ", agent_name))
                .title_style(Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(list, chunks[0]);

        let help = Paragraph::new(" j/k Navigate  Enter Open  Esc Back")
            .style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE));
        frame.render_widget(help, chunks[1]);
    }

    fn render_skills(&mut self, frame: &mut Frame, area: Rect) {
        let agent_name = self.agents.get(self.selected).map(|a| a.id.as_str()).unwrap_or("?");

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let items: Vec<ListItem> = self.skill_entries.iter().enumerate().map(|(i, sk)| {
            let is_sel = i == self.skill_selected;
            let (icon, icon_color) = if sk.is_enabled { ("✅", Theme::GREEN) } else { ("⛔", Theme::OVERLAY0) };
            let type_label = " shared  ";
            let type_color = Theme::SAPPHIRE;
            let name_style = if is_sel {
                Style::default().fg(Theme::TEXT).bg(Theme::SURFACE0).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Theme::SUBTEXT0)
            };
            let sel_icon = if is_sel { "▸ " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {} {} ", sel_icon, icon), Style::default().fg(icon_color)),
                Span::styled(format!("{:<20}", sk.name), name_style),
                Span::styled(format!("{:<9}", type_label), Style::default().fg(type_color)),
                Span::styled(sk.description.clone(), Style::default().fg(Theme::OVERLAY1)),
            ]))
        }).collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::MAUVE))
                .title(format!(" Skills — {} ", agent_name))
                .title_style(Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(list, chunks[0]);

        // Status / input
        let status = match &self.skill_mode {
            SkillInputMode::Install => Paragraph::new(Line::from(vec![
                Span::styled(" Install: ", Style::default().fg(Theme::MAUVE).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{}▌", self.skill_install_buf), Style::default().fg(Theme::TEXT)),
                Span::styled("  @anthropic/<name>  github:owner/repo/path  /local/path", Style::default().fg(Theme::OVERLAY0)),
            ])).style(Style::default().bg(Theme::SURFACE0)),
            SkillInputMode::ConfirmUninstall => {
                let name = self.skill_entries.get(self.skill_selected).map(|s| s.name.as_str()).unwrap_or("?");
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" Uninstall '{}'? ", name), Style::default().fg(Theme::RED).add_modifier(Modifier::BOLD)),
                    Span::styled("(y/N)", Style::default().fg(Theme::OVERLAY0)),
                ])).style(Style::default().bg(Theme::SURFACE0))
            }
            SkillInputMode::Normal => {
                if let Some(msg) = &self.skill_status {
                    Paragraph::new(format!(" {}", msg)).style(Style::default().fg(Theme::GREEN).bg(Theme::MANTLE))
                } else {
                    Paragraph::new("").style(Style::default().bg(Theme::MANTLE))
                }
            }
        };
        frame.render_widget(status, chunks[1]);

        let help_text = match &self.skill_mode {
            SkillInputMode::Install => " Enter Install  Esc Cancel",
            SkillInputMode::ConfirmUninstall => " y Confirm  any other key Cancel",
            SkillInputMode::Normal => " Space Toggle  i Install  x Uninstall  r Refresh  Esc Back",
        };
        frame.render_widget(
            Paragraph::new(help_text).style(Style::default().fg(Theme::OVERLAY0).bg(Theme::MANTLE)),
            chunks[2],
        );
    }

    fn render_editor(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(editor) = &mut self.editor {
            editor.render(frame, area);
        }
    }
}
