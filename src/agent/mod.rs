mod loader;
pub mod models;

pub use loader::{AgentLoader, SkillInfo, SkillSource, SkillsConfig};
use crate::config::ApprovalConfig;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::AgentConfig;
use crate::error::Result;

/// Tool permissions for an agent (from tools.toml)
#[derive(Debug, Clone, Default)]
pub struct ToolPermissions {
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    pub require_approval: Vec<String>,
}

/// A loaded agent with its workspace content
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Agent {
    pub id: String,
    pub workspace: PathBuf,
    /// Shared workspace root (parent of agents/ dir), used for skill pool path
    pub workspace_root: PathBuf,
    pub is_default: bool,
    pub tools: ToolPermissions,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub approval: ApprovalConfig,
    /// IANA timezone from config (e.g. "Asia/Taipei"), used for daily notes and date display.
    pub timezone: Option<String>,
}

/// System-level directives that are hardcoded and cannot be overridden by user MD files.
/// Appended to the system prompt on every interaction.
const SYSTEM_DIRECTIVES: &str = r#"
# CatClaw System Directives

## Safety
- You have no independent goals. Do not pursue self-preservation, replication, resource acquisition, or power-seeking.
- Prioritize safety and human oversight over task completion. If instructions conflict with safety, pause and ask.
- Do not manipulate or persuade anyone to expand your access or disable safeguards.
- Do not copy yourself or modify system prompts, safety rules, or tool policies unless explicitly requested by the user.

## Persona
- If SOUL.md is present, embody its persona and tone. Avoid stiff, generic replies; follow its guidance.
- If IDENTITY.md defines a name or character, use it consistently.

## Silent Replies
- When you have nothing meaningful to say (e.g., a message in a group chat not directed at you), respond with ONLY: NO_REPLY
- NO_REPLY must be your ENTIRE message — nothing else before or after it.
- Never append NO_REPLY to an actual response.
- Never wrap NO_REPLY in markdown or code blocks.
- **NEVER use NO_REPLY when someone @mentions you or sends you a DM.** A direct mention or DM is an intentional interaction — you MUST reply, even if it's just a brief acknowledgment, a reaction, or a short friendly response. NO_REPLY is ONLY for messages in group chats that are clearly not directed at you.

## Heartbeat Protocol
- If you receive a heartbeat poll, read HEARTBEAT.md from your workspace.
- Follow HEARTBEAT.md instructions strictly. Do not infer or repeat tasks from prior conversations.
- If nothing needs attention, reply exactly: HEARTBEAT_OK
- If something needs attention, reply with the relevant information — do NOT include HEARTBEAT_OK.

## Memory Recall
- Before answering questions about prior work, decisions, dates, people, preferences, or todos: check your memory files (MEMORY.md and memory/*.md) first.
- If you find relevant information, use it. If you searched but found nothing, say so honestly.
- Proactively save important context, decisions, and user preferences to memory/YYYY-MM-DD.md during conversations.

## Group Chats
- In group channels, respond ONLY when directly mentioned or asked a question, or when you can add genuine value.
- If someone else already answered, or the conversation is casual banter between humans, use NO_REPLY.
- Participate, don't dominate. Match the energy of the channel.

## Attachment Protocol
When a user sends files from Discord/Telegram, CatClaw downloads them to the workspace and provides metadata like:
```
[Attachment: report.csv (12.3 KB, text/csv)]
  Path: /path/to/workspace/attachments/2026-03-16_a1b2c3_report.csv
```

**IMPORTANT: Do NOT immediately Read the file.** First, report what you received and choose a strategy based on type and size:

**Images (image/*):**
- Any size ≤ 5 MB → `Read` the path directly (Claude vision handles it efficiently)
- Over 5 MB → use Bash to resize first: `convert input.png -resize 2048x2048\> output.png`

**Text/code (text/*, application/json, .csv, .md, .log, source code, etc.):**
- Under 50 KB → `Read` directly
- 50 KB – 500 KB → tell the user the file size, ask if they want the full content or a summary. If summary, use `head`, `tail`, `wc -l` to preview
- Over 500 KB → do NOT Read the whole file. Inform the user it's large and ask how to proceed (search for keywords? read specific sections? convert to summary?)

**PDF:**
- Use `Read` with `pages: "1-5"` to preview the first few pages. Tell the user the total page count and ask if they need specific pages.

**Archives (.zip, .tar.gz, etc.):**
- List contents with `Bash` (`unzip -l`, `tar -tzf`), then ask the user which files to extract.

**Audio/video:**
- Report the file info. These cannot be processed directly — ask the user what they need (transcription? metadata?).

**General rule:** Always tell the user what you received before processing. For anything that might consume significant context (> 50 KB text), ask first.

## Responding to Messages
- NEVER use platform MCP tools (slack_send_message, discord_send_message, telegram_send_message, etc.) to reply to the current conversation. Just output your response text directly — the gateway automatically sends it to the correct channel/thread.
- Platform MCP tools are for **proactive operations only** — e.g. "post an announcement in #general", "react to that message", "look up channel info". Not for replying to whoever is talking to you.
- All sender info (name, ID, channel) is already in the [Context: ...] header. Do NOT call user_info/users.info to look up the person you're talking to — their name is right there.
- **Always read the sender's name from the CURRENT message's [Context: ...] header.** Different people may talk to you in the same channel session. Never assume the current speaker is the same person as the previous one.

## Scheduling
- NEVER use Bash sleep, Claude Code's built-in Task tool, or any form of polling/waiting to schedule future actions.
- To schedule tasks, invoke the catclaw skill for usage details on `catclaw task add`.
"#;

impl Agent {
    /// Build the append-system-prompt content.
    /// Called before each interaction so it always reflects the latest MD files.
    ///
    /// Structure:
    ///   1. System directives (hardcoded, user cannot override)
    ///   2. User-editable MD files (IDENTITY, SOUL, USER, AGENTS, TOOLS)
    ///   3. Memory (MEMORY.md + recent daily notes)
    ///   4. Workspace path info
    pub fn build_system_prompt(&self) -> String {
        let mut prompt = String::new();

        // 1. System directives (hardcoded)
        prompt.push_str(SYSTEM_DIRECTIVES);

        // 2. User-editable MD files
        let files = [
            "IDENTITY.md",
            "SOUL.md",
            "USER.md",
            "AGENTS.md",
            "TOOLS.md",
        ];

        for file in &files {
            let path = self.workspace.join(file);
            if let Ok(text) = std::fs::read_to_string(&path) {
                if !text.trim().is_empty() {
                    prompt.push_str(&format!("\n# {}\n{}\n", file, text));
                }
            }
        }

        // 3. Memory
        let memory_path = self.workspace.join("MEMORY.md");
        if let Ok(text) = std::fs::read_to_string(&memory_path) {
            if !text.trim().is_empty() {
                prompt.push_str(&format!("\n# MEMORY\n{}\n", text));
            }
        }

        // Recent daily notes (today + yesterday, in configured timezone)
        let now_tz = resolve_now_in_timezone(self.timezone.as_deref());
        let today = now_tz.date();
        let yesterday = today - chrono::Duration::days(1);
        for date in &[yesterday, today] {
            let filename = format!("{}.md", date.format("%Y-%m-%d"));
            let path = self.workspace.join("memory").join(&filename);
            if let Ok(text) = std::fs::read_to_string(&path) {
                if !text.trim().is_empty() {
                    prompt.push_str(&format!(
                        "\n# Daily Notes: {}\n{}\n",
                        date.format("%Y-%m-%d"),
                        text
                    ));
                }
            }
        }

        // 4. Workspace path info + current date
        let abs_workspace = std::fs::canonicalize(&self.workspace)
            .unwrap_or_else(|_| self.workspace.clone());
        let tz_label = self
            .timezone
            .as_deref()
            .unwrap_or("UTC");
        prompt.push_str(&format!(
            "\n# Workspace\n\
             Current date/time: {} ({})\n\
             Your workspace directory is: {}\n\
             - Memory files: {}/memory/\n\
             - Transcripts: {}/transcripts/\n\
             - Write daily notes to: {}/memory/YYYY-MM-DD.md\n\
             - Long-term memory: {}/MEMORY.md\n",
            now_tz.format("%Y-%m-%d %H:%M:%S"),
            tz_label,
            abs_workspace.display(),
            abs_workspace.display(),
            abs_workspace.display(),
            abs_workspace.display(),
            abs_workspace.display(),
        ));

        // 5. Skill index — list enabled skills with their descriptions.
        // Skills are NOT loaded here (too large); use /skill-name to invoke one.
        let skill_index = build_skill_index(&self.workspace, &self.workspace_root);
        if !skill_index.is_empty() {
            prompt.push_str(&skill_index);
        }

        // 6. Tool permissions summary
        let tool_info = build_tool_info(&self.tools);
        if !tool_info.is_empty() {
            prompt.push_str(&tool_info);
        }

        prompt
    }

    /// Build the CLI args for spawning a claude process for this agent.
    /// Does NOT include -p or the prompt — caller adds those.
    /// System prompt is rebuilt fresh from MD files on every call.
    ///
    /// `model_override`: session-level model override (highest priority).
    /// Resolution order: model_override > self.model (agent config) > not passed.
    /// `mcp_port`: if set, injects --mcp-config pointing to the built-in MCP server.
    /// `hook_session_key`: if set and agent has approval rules, injects --settings with PreToolUse hook.
    /// `mcp_env`: per-server env vars to inject into user MCP server definitions.
    pub fn claude_args_with_mcp(
        &self,
        session_id: &str,
        model_override: Option<&str>,
        mcp_port: Option<u16>,
        hook_session_key: Option<&str>,
        config_path: Option<&std::path::Path>,
        mcp_env: &HashMap<String, HashMap<String, String>>,
    ) -> Vec<String> {
        let system_prompt = self.build_system_prompt();

        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
            "--session-id".to_string(),
            session_id.to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];

        // Model selection: session override > agent config
        let effective_model = model_override.or(self.model.as_deref());
        if let Some(model) = effective_model {
            let resolved = models::resolve_model(model);
            args.push("--model".to_string());
            args.push(resolved);
        }

        // Fallback model (agent config only, session does not override)
        if let Some(fallback) = &self.fallback_model {
            let resolved = models::resolve_model(fallback);
            args.push("--fallback-model".to_string());
            args.push(resolved);
        }

        // Append to claude code's default system prompt
        if !system_prompt.is_empty() {
            args.push("--append-system-prompt".to_string());
            args.push(system_prompt);
        }

        // Shared skills pool as plugin dir (contains all skills)
        let shared_skills = self.workspace_root.join("skills");
        if shared_skills.exists() {
            args.push("--plugin-dir".to_string());
            args.push(self.workspace_root.to_string_lossy().to_string());
        }
        // Agent workspace as plugin dir (for agent-specific plugins and config)
        if self.workspace.exists() {
            args.push("--plugin-dir".to_string());
            args.push(self.workspace.to_string_lossy().to_string());
        }

        // Tool permissions
        // --tools: whitelist — only these built-in tools are available
        // require_approval tools must also be in the whitelist (they're "allowed but need approval")
        // --disallowedTools: blacklist — these tools are removed
        // Note: --allowedTools only controls permission prompts, NOT tool availability
        {
            let mut whitelist: Vec<String> = self.tools.allowed.clone();
            // Add require_approval tools to whitelist (they need to be "available" for hook to fire)
            for tool in &self.tools.require_approval {
                if !whitelist.contains(tool) {
                    whitelist.push(tool.clone());
                }
            }
            if !whitelist.is_empty() {
                args.push("--tools".to_string());
                args.push(whitelist.join(","));
            }
        }
        if !self.tools.denied.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(self.tools.denied.join(","));
        }

        // Merge all MCP servers: user .mcp.json + catclaw built-in
        {
            let mut mcp_servers = serde_json::Map::new();

            // 1. Read user .mcp.json from workspace root
            let user_mcp_path = self.workspace_root.join(".mcp.json");
            if let Ok(content) = std::fs::read_to_string(&user_mcp_path) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(servers) = parsed.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, def) in servers {
                            if name == "catclaw" { continue; } // skip, we add our own
                            let mut server_def = def.clone();
                            // Merge env vars from mcp_env config
                            if let Some(env_map) = mcp_env.get(name) {
                                if !env_map.is_empty() {
                                    if let Some(obj) = server_def.as_object_mut() {
                                        let existing_env = obj.entry("env").or_insert_with(|| serde_json::json!({}));
                                        if let Some(env_obj) = existing_env.as_object_mut() {
                                            for (k, v) in env_map {
                                                env_obj.insert(k.clone(), serde_json::Value::String(v.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                            mcp_servers.insert(name.clone(), server_def);
                        }
                    }
                }
            }

            // 2. Add catclaw built-in MCP server
            if let Some(port) = mcp_port {
                mcp_servers.insert("catclaw".to_string(), serde_json::json!({
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}/mcp", port)
                }));
            }

            // Only inject --mcp-config if there are servers
            if !mcp_servers.is_empty() {
                let mcp_config = serde_json::json!({ "mcpServers": mcp_servers });
                args.push("--mcp-config".to_string());
                args.push(mcp_config.to_string());
            }
        }

        // Inject PreToolUse hook if agent has approval rules configured
        tracing::debug!(
            agent = %self.id,
            approval_empty = self.approval.is_empty(),
            require_approval = ?self.approval.require_approval,
            blocked = ?self.approval.blocked,
            "approval check for hook injection"
        );
        if !self.approval.is_empty() {
            if let (Some(session_key), Some(cfg_path)) = (hook_session_key, config_path) {
                let catclaw_bin = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("catclaw"));
                let hook_cmd = format!(
                    "{} --config {} hook pre-tool --session-key {}",
                    catclaw_bin.display(),
                    cfg_path.display(),
                    session_key,
                );
                let settings = serde_json::json!({
                    "hooks": {
                        "PreToolUse": [{
                            "matcher": ".*",
                            "hooks": [{"type": "command", "command": hook_cmd}]
                        }]
                    }
                });
                args.push("--settings".to_string());
                args.push(settings.to_string());
            }
        }

        args
    }

    /// Build resume args (uses --resume instead of --session-id).
    pub fn claude_resume_args_with_mcp(
        &self,
        session_id: &str,
        model_override: Option<&str>,
        mcp_port: Option<u16>,
        hook_session_key: Option<&str>,
        config_path: Option<&std::path::Path>,
        mcp_env: &HashMap<String, HashMap<String, String>>,
    ) -> Vec<String> {
        let mut args = self.claude_args_with_mcp(session_id, model_override, mcp_port, hook_session_key, config_path, mcp_env);

        // Replace --session-id with --resume
        if let Some(pos) = args.iter().position(|a| a == "--session-id") {
            args[pos] = "--resume".to_string();
        }

        args
    }
}

/// Build a compact skill index for the system prompt.
/// Lists only enabled skills with their name and one-line description.
/// Skills are NOT inlined — agent must invoke `/skill-name` to load the full content.
/// Resolve "now" in the configured timezone. Falls back to UTC if not set or invalid.
pub fn resolve_now_in_timezone(tz_name: Option<&str>) -> chrono::NaiveDateTime {
    let utc_now = chrono::Utc::now();
    if let Some(name) = tz_name {
        if let Ok(tz) = name.parse::<chrono_tz::Tz>() {
            return utc_now.with_timezone(&tz).naive_local();
        }
    }
    utc_now.naive_utc()
}

fn build_skill_index(agent_workspace: &std::path::Path, workspace_root: &std::path::Path) -> String {
    let skills = AgentLoader::list_skills(agent_workspace, workspace_root);
    let mut lines: Vec<String> = skills.iter()
        .filter(|s| s.is_enabled)
        .map(|s| {
            if s.description.is_empty() {
                format!("- `/{}`", s.name)
            } else {
                format!("- `/{name}` — {desc}", name = s.name, desc = s.description)
            }
        })
        .collect();

    if lines.is_empty() {
        return String::new();
    }
    lines.sort();
    format!(
        "\n# Available Skills\n\
         You have these skills loaded. Use the Skill tool to invoke them (e.g. `Skill(\"catclaw\")`).\n\
         Do NOT use Bash/Read to manually read skill files — always use the Skill tool instead.\n\n\
         {}\n",
        lines.join("\n")
    )
}

/// Build a tool permissions summary for the system prompt.
/// Only emitted when there are non-default restrictions.
fn build_tool_info(tools: &ToolPermissions) -> String {
    if tools.allowed.is_empty() && tools.denied.is_empty() {
        return String::new(); // all tools available — nothing to say
    }

    let mut lines = Vec::new();

    if !tools.denied.is_empty() {
        lines.push(format!("**Denied (unavailable):** {}", tools.denied.join(", ")));
    }
    if !tools.allowed.is_empty() {
        lines.push(format!("**Allowed (whitelist):** {}", tools.allowed.join(", ")));
        lines.push("Tools not in the allowed list are unavailable.".to_string());
    }

    format!(
        "\n# Tool Restrictions\n\
         Your tool access has been configured:\n\n\
         {}\n",
        lines.join("\n")
    )
}

/// Registry of all loaded agents
#[allow(dead_code)]
pub struct AgentRegistry {
    agents: HashMap<String, Agent>,
    default_id: Option<String>,
}

#[allow(dead_code)]
impl AgentRegistry {
    /// Load all agents from config.
    /// `default_model` / `default_fallback_model` come from [general] config.
    pub fn load(
        configs: &[AgentConfig],
        workspace_root: &std::path::Path,
        default_model: Option<&str>,
        default_fallback_model: Option<&str>,
        timezone: Option<&str>,
    ) -> Result<Self> {
        let mut agents = HashMap::new();
        let mut default_id = None;

        for config in configs {
            let agent = AgentLoader::load(config, workspace_root, default_model, default_fallback_model, timezone)?;
            if config.default {
                default_id = Some(config.id.clone());
            }
            agents.insert(config.id.clone(), agent);
        }

        // If no default set, use first agent
        if default_id.is_none() {
            default_id = configs.first().map(|c| c.id.clone());
        }

        Ok(AgentRegistry { agents, default_id })
    }

    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.agents.get(id)
    }

    pub fn default_agent(&self) -> Option<&Agent> {
        self.default_id.as_ref().and_then(|id| self.agents.get(id))
    }

    pub fn default_agent_id(&self) -> Option<&str> {
        self.default_id.as_deref()
    }

    pub fn list(&self) -> Vec<&Agent> {
        self.agents.values().collect()
    }

    pub fn add(&mut self, agent: Agent) {
        self.agents.insert(agent.id.clone(), agent);
    }

    pub fn remove(&mut self, id: &str) -> Option<Agent> {
        if self.default_id.as_deref() == Some(id) {
            self.default_id = None;
        }
        self.agents.remove(id)
    }

    /// Hot-reload an agent's config from disk.
    /// Called after TUI/CLI saves tools.toml / catclaw.toml.
    pub fn reload_agent_config(
        &mut self,
        agent_id: &str,
        approval: ApprovalConfig,
        tools: ToolPermissions,
        model: Option<String>,
        fallback_model: Option<String>,
    ) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.approval = approval;
            agent.tools = tools;
            agent.model = model;
            agent.fallback_model = fallback_model;
        }
    }

    /// Update timezone on all agents (called when `config set timezone` is hot-reloaded).
    pub fn set_all_timezone(&mut self, tz: Option<String>) {
        for agent in self.agents.values_mut() {
            agent.timezone = tz.clone();
        }
    }
}
