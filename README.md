<p align="center">
  <img src="https://img.shields.io/badge/rust-1.75+-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/claude_code-CLI-blueviolet" alt="Claude Code">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
  <img src="https://img.shields.io/badge/status-alpha-yellow" alt="Status">
</p>

```
     ██████╗  █████╗ ████████╗ ██████╗██╗      █████╗ ██╗    ██╗
    ██╔════╝ ██╔══██╗╚══██╔══╝██╔════╝██║     ██╔══██╗██║    ██║
    ██║      ███████║   ██║   ██║     ██║     ███████║██║ █╗ ██║
    ██║      ██╔══██║   ██║   ██║     ██║     ██╔══██║██║███╗██║
    ╚██████╗ ██║  ██║   ██║   ╚██████╗███████╗██║  ██║╚███╔███╔╝
     ╚═════╝ ╚═╝  ╚═╝   ╚═╝    ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝
```

<p align="center">
  <strong>Personal AI Gateway powered by Claude Code</strong><br>
  <em>Multi-agent &bull; Multi-channel &bull; Always-on</em>
</p>

---

**English** | [繁體中文](README.zh-TW.md)

CatClaw is a Rust daemon that turns your **Claude Code subscription** into a personal AI assistant accessible from Discord, Telegram, and a beautiful terminal UI. Inspired by [OpenClaw](https://github.com/nicekid1/OpenClaw), built from scratch in Rust for performance, reliability, and full Anthropic compliance.

## Why CatClaw?

- **Use your Claude Code subscription** &mdash; no API keys, no surprise bills. CatClaw spawns `claude -p` subprocesses that use your existing Claude Code plan.
- **Multi-agent** &mdash; define multiple AI personas (main assistant, research expert, code reviewer), each with their own personality, memory, and tool permissions.
- **Multi-channel** &mdash; talk to your agents from Discord, Telegram, or the built-in TUI. All channels share the same session and memory system.
- **Tool approval system** &mdash; require user confirmation before agents execute sensitive tools (Bash, Edit, etc.) with inline approval UI in TUI and Discord/Telegram buttons.
- **Stateless gateway** &mdash; all state persisted to SQLite. Kill the daemon anytime, restart, and everything picks up where it left off.
- **Beautiful TUI** &mdash; Catppuccin Mocha themed terminal interface with 8 panels for managing everything.

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) 1.75+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and authenticated

### Install

```bash
git clone https://github.com/CatiesGames/catclaw.git
cd catclaw
cargo build --release
```

### Launch

```bash
./target/release/catclaw onboard
```

On first run, CatClaw will:
1. Show the splash logo
2. Run the interactive setup wizard (verify Claude Code CLI, create your agent, configure channels)
3. Start the gateway in the background
4. Launch the TUI

On subsequent runs, it skips setup and goes straight to gateway + TUI.

```bash
# Other ways to run:
catclaw onboard                      # Re-run the setup wizard
catclaw gateway start             # Start gateway in foreground
catclaw gateway start -d          # Start gateway as background daemon
catclaw gateway stop              # Stop the background gateway
catclaw gateway status            # Show gateway status
catclaw tui                       # Launch TUI only (connects to running gateway)
```

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                       CatClaw Gateway (Rust)                     │
│                                                                  │
│  ┌─────────────┐ ┌─────────────┐                                │
│  │  Discord     │ │  Telegram   │    Channel Adapters            │
│  │  Adapter     │ │  Adapter    │                                │
│  └──────┬───────┘ └──────┬──────┘                                │
│         └────────────────┘                                        │
│                  ▼                                                │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Message Router  →  Agent Registry  →  Session Manager    │  │
│  │  (binding table)    (SOUL/tools)       (claude -p spawn)  │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐                     │
│  │  State DB         │  │  Scheduler       │                     │
│  │  (SQLite WAL)     │  │  (cron/heartbeat)│                     │
│  └──────────────────┘  └──────────────────┘                     │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  WS Server (/ws)  +  MCP Server (/mcp)  — port 21130    │   │
│  └──────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┘
          ▲                              ▲
          │ WebSocket                    │ MCP JSON-RPC
    ┌─────┴─────┐                 ┌──────┴──────┐
    │  TUI      │                 │  Claude CLI │
    │ (ratatui) │                 │  (tool use) │
    └───────────┘                 └─────────────┘
```

**How it works**: When a message arrives from any channel, CatClaw resolves which agent should handle it (via binding table), finds or creates a session, spawns a `claude -p --output-format stream-json` subprocess, and streams the response back to the originating channel.

Each `claude -p` subprocess uses your Claude Code subscription &mdash; no API keys needed.

## CLI Reference

All configuration is managed through the CLI or TUI. No manual file editing required.

### Gateway

```bash
catclaw onboard                   # Onboarding: setup wizard → start gateway → launch TUI
catclaw gateway start             # Start gateway in foreground
catclaw gateway start -d          # Start as background daemon
catclaw gateway stop              # Stop background gateway
catclaw gateway restart           # Restart daemon
catclaw gateway status            # Show running status and PID
catclaw onboard                      # Re-run the setup wizard
catclaw tui                       # Launch TUI only
```

### Agent Management

```bash
catclaw agent new <name>                          # Create a new agent
catclaw agent list                                # List all agents
catclaw agent edit <name> <file>                  # Open file in $EDITOR
catclaw agent tools <name>                        # Show current tool permissions
catclaw agent tools <name> \
  --allow "Read,Grep,WebFetch" \
  --deny "Bash" \
  --approve "Edit,Write"                          # Configure tool permissions
catclaw agent delete <name>                       # Delete an agent
```

`<file>` values: `soul`, `user`, `identity`, `agents`, `tools`, `boot`, `heartbeat`, `memory`

### Channels

```bash
catclaw channel list                              # List configured channels
catclaw channel add discord \
  --token-env CATCLAW_DISCORD_TOKEN \
  --guilds "123456789" \
  --activation mention                            # Add Discord channel
catclaw channel add telegram \
  --token-env CATCLAW_TELEGRAM_TOKEN              # Add Telegram channel
```

### Bindings (Channel → Agent routing)

```bash
catclaw bind "discord:channel:222222" research    # Bind a channel to an agent
catclaw bind "telegram:*" main                    # Bind all Telegram to main
catclaw bind "*" main                             # Global fallback
```

### Sessions

```bash
catclaw session list                              # List all sessions
catclaw session delete <key>                      # Delete a session
```

### Skills

```bash
catclaw skill list <agent>                        # List skills (built-in + installed)
catclaw skill enable <agent> <skill>              # Enable a skill
catclaw skill disable <agent> <skill>             # Disable a skill
catclaw skill install <source>                    # Install from remote source
catclaw skill uninstall <skill>                   # Remove a skill
```

Install sources: `@anthropic/<name>`, `github:<owner>/<repo>/path`, `/local/path`

### Scheduled Tasks

```bash
catclaw task list                                 # List scheduled tasks
catclaw task add <name> --agent main \
  --prompt "Check inbox" --every 30               # Repeat every 30 minutes
catclaw task add <name> --agent main \
  --prompt "Morning briefing" --cron "0 9 * * *"  # Cron schedule
catclaw task enable <id>                          # Enable a task
catclaw task disable <id>                         # Disable a task
catclaw task delete <id>                          # Delete a task
```

### Configuration

```bash
catclaw config show                               # Show full config (TOML)
catclaw config get <key>                          # Get a specific value
catclaw config set <key> <value>                  # Set a value (hot-reload when possible)
```

### Logs

```bash
catclaw logs                                      # Show recent logs
catclaw logs -f                                   # Stream in real-time
catclaw logs --level debug                        # Filter by level
catclaw logs --grep "discord"                     # Search by pattern
catclaw logs --json                               # Raw JSON output
```

## TUI

The TUI provides a beautiful Catppuccin Mocha themed interface with 8 panels:

| Panel | Description |
|---|---|
| **Dashboard** | Overview with agent count, active sessions, uptime |
| **Sessions** | View all sessions, chat directly with agents, inline tool approval |
| **Agents** | Manage agents, edit personality files, configure tool permissions (allowed/denied/approval) |
| **Skills** | Enable/disable/install skills per agent |
| **Tasks** | View and manage scheduled tasks (heartbeat, cron) |
| **Bindings** | Map channels to agents |
| **Config** | View and edit gateway configuration with hot-reload |
| **Logs** | Live log viewer with search, level filter, structured fields |

**Keyboard shortcuts**: `Tab`/`Shift+Tab` cycle panels, `Alt+1-8` jump directly, `q` quit. Mouse scroll supported in Sessions and Logs.

**Chat commands**: `/new` start fresh session, `/model <name>` switch model, `/help` show help.

## Agent System

Each agent has its own workspace with personality, memory, skills, and tool permissions:

```
workspace/agents/main/
├── SOUL.md              # Personality, tone, values
├── USER.md              # Who the user is
├── IDENTITY.md          # Agent name, role
├── AGENTS.md            # Workspace conventions
├── TOOLS.md             # Tool usage guidelines
├── BOOT.md              # Startup instructions (prepended to first message)
├── HEARTBEAT.md         # Periodic check tasks
├── MEMORY.md            # Long-term memory (curated)
├── memory/              # Daily notes (YYYY-MM-DD.md)
├── transcripts/         # Session logs (JSONL)
└── tools.toml           # Tool permissions
```

### Tool Permissions

Each tool exists in exactly one of three states:

```toml
# workspace/agents/research/tools.toml
allowed = ["Read", "Grep", "Glob", "WebFetch", "WebSearch"]
denied = ["Bash"]
require_approval = ["Edit", "Write"]
```

- **allowed** &mdash; tool runs freely
- **denied** &mdash; tool is completely blocked
- **require_approval** &mdash; tool runs only after user approves (via TUI inline widget, Discord button, or Telegram keyboard)

Manage via TUI (Agents → `t` Tools → Space to cycle states) or CLI (`catclaw agent tools`).

### Skills

CatClaw ships with built-in skills and supports installing custom ones:

| Skill | Description |
|---|---|
| `catclaw` | CatClaw system administration (agent knows all CLI commands) |
| `discord` | Discord formatting and MCP tool usage |
| `telegram` | Telegram formatting and MCP tool usage |
| `sessions-history` | Query transcripts from other sessions |
| `injection-guard` | Defend against prompt injection from external content |

Skills are shared across agents via `workspace/skills/`. Each agent can enable/disable skills independently.

### User MCP Servers

Add custom MCP servers in `workspace/.mcp.json` (shared across all agents):

```json
{
  "mcpServers": {
    "my-api": {
      "type": "http",
      "url": "https://api.example.com/mcp",
      "headers": { "Authorization": "Bearer ${MY_API_KEY}" }
    }
  }
}
```

MCP tools appear in the TUI Tools panel under "User MCP Servers" and can be denied or set to require approval per agent.

## Channel Adapters

| Channel | Status | Features |
|---|---|---|
| **Discord** | ✅ | Threads, typing indicator, approval buttons, 32 MCP actions |
| **Telegram** | ✅ | Long polling, forum topics, approval keyboard, 26 MCP actions |
| **Slack** | Planned | — |
| **TUI** | ✅ | Direct chat with streaming, inline approval widget |

**Activation modes** (DMs always respond; this controls group/server channels):
- `mention` (default) &mdash; respond only when @mentioned
- `all` &mdash; respond to every message

### Built-in MCP Server

CatClaw exposes channel adapter operations as MCP tools, so agents can autonomously perform platform actions:

```
Agent wants to list Discord channels
  → Claude calls mcp__catclaw__discord_get_channels
  → CatClaw MCP server → serenity → Discord REST API
  → JSON result back to agent
```

**Discord** (32 tools): messages, reactions, pins, threads, channels, categories, permissions, guilds, members, roles, emojis, moderation, events, stickers.

**Telegram** (26 tools): messages, pins, chat info/management, moderation, polls, forum topics, permissions, invite links.

## Session Management

```
SessionKey = catclaw:{agent_id}:{origin}:{context_id}
```

**Lifecycle**:
```
New → Active (claude -p subprocess alive)
       ↓ idle 30 min
     Idle (subprocess exited, session_id preserved for --resume)
       ↓ idle 7 days
     Archived (summary written to memory, start fresh)
```

**Concurrency**: configurable max (default 3) with priority queue (DM > mention > cron). Excess requests queued.

**Stateless restart**: all state in SQLite. Kill and restart — sessions resume via `--resume`.

## Configuration

```toml
[general]
workspace = "./workspace"
state_db = "./state.sqlite"
max_concurrent_sessions = 3
session_idle_timeout_mins = 30
session_archive_timeout_hours = 168
port = 21130                        # WS + MCP on single port
streaming = true
default_model = "opus"              # optional: opus, sonnet, haiku

[[channels]]
type = "discord"
token_env = "CATCLAW_DISCORD_TOKEN"
guilds = ["123456789"]
activation = "mention"

[[channels]]
type = "telegram"
token_env = "CATCLAW_TELEGRAM_TOKEN"
activation = "mention"

[[agents]]
id = "main"
workspace = "./workspace/agents/main"
default = true

[agents.approval]
timeout_secs = 120                  # approval timeout (global)
```

## Tech Stack

| Component | Crate |
|---|---|
| Async runtime | `tokio` |
| Discord | `serenity` + `poise` |
| Telegram | `teloxide` |
| HTTP server (WS + MCP) | `axum` |
| CLI | `clap` (derive) |
| Database | `rusqlite` (bundled SQLite, WAL) |
| TUI | `ratatui` + `crossterm` + `tui-textarea` |
| Config | `toml` + `serde` |
| Scheduling | `croner` (cron expressions) |
| Logging | `tracing` |

## Feedback

Found a bug or have a feature request? [Open an issue](https://github.com/CatiesGames/catclaw/issues).

## License

MIT

---

<p align="center">
  Built with Rust and Claude Code<br>
  <strong>CatiesGames</strong>
</p>
