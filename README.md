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

CatClaw is a Rust daemon that turns your **Claude Code subscription** into a personal AI assistant accessible from Discord, Telegram, Slack, and a beautiful terminal UI. Inspired by [OpenClaw](https://github.com/openclaw/openclaw), built from scratch in Rust for performance, reliability, and full Anthropic compliance.

## Why CatClaw?

- **Use your Claude Code subscription** &mdash; no API keys, no surprise bills. CatClaw spawns `claude -p` subprocesses that use your existing Claude Code plan.
- **Multi-agent** &mdash; define multiple AI personas (main assistant, research expert, code reviewer), each with their own personality, memory, and tool permissions.
- **Multi-channel** &mdash; talk to your agents from Discord, Telegram, Slack, or the built-in TUI. All channels share the same session and memory system.
- **Stateless gateway** &mdash; all state persisted to SQLite. Kill the daemon anytime, restart, and everything picks up where it left off.
- **Fork & converge** &mdash; branch a conversation into a Discord thread with full context, then merge the conclusions back.
- **Beautiful TUI** &mdash; Catppuccin Mocha themed terminal interface with 7 panels for managing sessions, agents, skills, tasks, bindings, config, and logs.

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) 1.75+
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and authenticated
- (Optional) Discord bot token for Discord integration
- (Optional) [Ollama](https://ollama.ai/) for local embedding (Phase 2)

### Install

```bash
git clone https://github.com/CatiesGames/catclaw.git
cd catclaw
cargo build --release
```

### Launch

```bash
catclaw
```

That's it. On first run, CatClaw will:
1. Show the splash logo
2. Run the interactive setup wizard (verify Claude Code CLI, create your agent, configure channels)
3. Start the gateway in the background
4. Launch the TUI

On subsequent runs, it skips setup and goes straight to gateway + TUI.

```bash
# Other ways to run:
catclaw init             # Re-run the setup wizard
catclaw gateway          # Start gateway in foreground (no TUI)
catclaw tui              # Launch TUI only (no auto-init or background gateway)
catclaw stop             # Stop the background gateway
```

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                       CatClaw Gateway (Rust)                     │
│                                                                  │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐                │
│  │  Discord     │ │  Telegram   │ │  Slack      │  Channel       │
│  │  Adapter     │ │  Adapter    │ │  Adapter    │  Adapters      │
│  └──────┬───────┘ └──────┬──────┘ └──────┬──────┘                │
│         └────────────────┼───────────────┘                        │
│                          ▼                                        │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Message Router  →  Agent Registry  →  Session Manager    │  │
│  │  (binding table)    (SOUL/tools)       (claude -p spawn)  │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────┐  │
│  │  State DB         │  │  Scheduler       │  │  TUI         │  │
│  │  (SQLite WAL)     │  │  (cron/heartbeat)│  │  (ratatui)   │  │
│  └──────────────────┘  └──────────────────┘  └──────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

**How it works**: When a message arrives from any channel, CatClaw resolves which agent should handle it (via binding table), finds or creates a session, spawns a `claude -p --input-format stream-json --output-format stream-json` subprocess, and streams the response back to the originating channel.

Each `claude -p` subprocess uses your Claude Code subscription &mdash; no API keys needed.

## CLI Reference

All configuration is managed through the CLI or TUI. No manual file editing required.

### Gateway & TUI

```bash
catclaw                  # Unified: splash → auto-init → background gateway → TUI
catclaw gateway          # Start gateway in foreground (no TUI)
catclaw tui              # Launch TUI only
catclaw stop             # Stop the background gateway
catclaw init             # Re-run the setup wizard
```

### Agent Management

```bash
catclaw agent new <name>           # Create a new agent (opens SOUL.md editor)
catclaw agent list                 # List all agents
catclaw agent edit <name> soul     # Edit an agent's SOUL.md
catclaw agent edit <name> user     # Edit an agent's USER.md
catclaw agent tools <name> \
  --allow "Read,Grep,WebFetch" \
  --deny "Bash,Edit"              # Configure tool permissions
catclaw agent delete <name>        # Delete an agent
```

### Channel Adapters

```bash
catclaw channel add discord \
  --token-env DISCORD_TOKEN \
  --guilds "123456789" \
  --activation mention             # Add Discord channel

catclaw channel add telegram \
  --token-env TELEGRAM_TOKEN       # Add Telegram channel

catclaw channel list               # List configured channels
```

### Bindings (Channel → Agent routing)

```bash
catclaw bind "discord:channel:222222" research   # Bind a channel to an agent
catclaw bind "telegram:*" main                   # Bind all Telegram to main
```

### Sessions

```bash
catclaw session list               # List all sessions
catclaw session delete <key>       # Delete a session
```

### Skills

```bash
catclaw skill list <agent>         # List skills for an agent
catclaw skill enable <agent> <skill>   # Enable a skill
catclaw skill disable <agent> <skill>  # Disable a skill
catclaw skill install <agent> <path>   # Install a skill from directory
```

### Scheduled Tasks

```bash
catclaw task list                  # List scheduled tasks
catclaw task add heartbeat \
  --agent main \
  --interval 30                    # Add heartbeat every 30 minutes

catclaw task add cron \
  --agent main \
  --cron "0 9 * * *" \
  --payload "Good morning check"   # Add cron job

catclaw task enable <id>           # Enable a task
catclaw task disable <id>          # Disable a task
catclaw task delete <id>           # Delete a task
```

### Configuration

```bash
catclaw config show                # Show current config
catclaw config set \
  max_concurrent_sessions 5        # Modify a setting
```

## TUI

The TUI provides a beautiful Catppuccin Mocha themed interface with 7 panels:

```
┌─ CatClaw ─────────── Sessions │ Agents │ Skills │ Tasks │ Bindings │ Config │ Logs ─┐
│                                                                                       │
│ ┌─ Sessions ──────────────┐ ┌─ Chat ──────────────────────────────────────────────┐  │
│ │                         │ │                                                      │  │
│ │ ● #general              │ │  ╭─ main ──────────────────────────────────────╮    │  │
│ │   main · Active · 2m    │ │  │ Here are the key best practices for Rust    │    │  │
│ │                         │ │  │ async programming:                          │    │  │
│ │ ○ #research             │ │  │                                              │    │  │
│ │   research · Idle · 15m │ │  │ 1. Use tokio as your runtime...             │    │  │
│ │                         │ │  ╰──────────────────────────────────────────────╯    │  │
│ │ ◌ Thread: auth-bug      │ │                                                      │  │
│ │   main · Suspended · 1h │ │  ╭─ You ───────────────────────────────────────╮    │  │
│ │                         │ │  │ What about tokio::spawn vs join!?            │    │  │
│ │                         │ │  ╰──────────────────────────────────────────────╯    │  │
│ │                         │ │                                                      │  │
│ │                         │ │  ● main is thinking...                               │  │
│ ├─────────────────────────┤ ├──────────────────────────────────────────────────────┤  │
│ │ n New  f Fork  d Delete │ │ > _                                                  │  │
│ └─────────────────────────┘ └──────────────────────────────────────────────────────┘  │
│                                                                                       │
│ ← → Tab  ↑↓ Select  Enter Open  q Quit  ? Help           ● 2 active  ○ 1 idle       │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

| Panel | Description |
|---|---|
| **Sessions** | View all sessions with status, chat directly with any agent |
| **Agents** | Manage agents, edit personality (SOUL.md), configure tool permissions |
| **Skills** | Enable/disable skills per agent |
| **Tasks** | View and manage scheduled tasks (heartbeat, cron) |
| **Bindings** | Map channels to agents |
| **Config** | View and edit gateway configuration |
| **Logs** | Live log tail with level filtering |

**Keyboard shortcuts**: `Tab`/`Shift+Tab` cycle panels, `Alt+1-7` jump directly, `q` quit, `?` help.

## Agent System

Each agent has its own workspace with personality, memory, skills, and tool permissions:

```
workspace/agents/main/
├── SOUL.md              # Personality, tone, values
├── USER.md              # Who the user is
├── IDENTITY.md          # Agent name, role
├── AGENTS.md            # Collaboration rules
├── TOOLS.md             # Tool usage guidelines
├── BOOT.md              # Startup checklist
├── HEARTBEAT.md         # Periodic check tasks
├── MEMORY.md            # Long-term memory (curated)
├── memory/              # Daily notes (YYYY-MM-DD.md)
├── transcripts/         # Session logs (JSONL)
├── skills/              # Agent-specific Claude Code skills
├── .mcp.json            # Agent-specific MCP servers
└── tools.toml           # Tool permissions (allowed/denied)
```

**Tool permissions** give you fine-grained control over what each agent can do:

```toml
# agents/research/tools.toml — read-only research agent
allowed = ["Read", "Grep", "Glob", "WebFetch", "WebSearch"]
denied = ["Bash", "Edit", "Write"]
```

CatClaw leverages **Claude Code's native plugin system** &mdash; each agent's `skills/` directory is loaded via `--plugin-dir`, no custom skill system needed.

## Channel Adapters

Channels are pluggable adapters implementing a common `ChannelAdapter` trait. All channels normalize messages into a unified `MsgContext` format.

| Channel | Status | Features |
|---|---|---|
| **Discord** | Implemented | Threads, typing indicator, chunked messages, 32 MCP actions |
| **Telegram** | Implemented | Long polling, forum topics, 26 MCP actions |
| **Slack** | Planned | Threads, reactions |
| **TUI** | Implemented | Direct chat in terminal |

**Activation modes**: DMs are always responded to. For group chats / server channels:
- `mention` (default) &mdash; respond only when @mentioned
- `all` &mdash; respond to every message

Activation can be configured per-channel in `catclaw.toml`, via TUI Config panel, or with per-channel overrides.

**Fork & Converge** (Discord):
- `/fork` &mdash; Create a Discord thread with a forked session (inherits full context)
- `/converge` &mdash; Summarize the thread and post conclusions back to the parent channel

### Built-in MCP Server

CatClaw runs a built-in [MCP](https://modelcontextprotocol.io/) server on the same port as the gateway (`/mcp`). This exposes channel adapter operations as LLM tools, so agents can autonomously perform platform actions:

```
LLM tool call (discord_get_messages, telegram_send_poll, ...)
  → Claude CLI → MCP protocol → Gateway MCP Server
  → Adapter.execute(action, params) → Platform REST API
  → Result back to LLM
```

**Discord tools** (32): get/send/edit/delete messages, reactions, pins, threads, channels, categories, permissions, guild info, members, roles, emojis, moderation (timeout/kick/ban), events, stickers.

**Telegram tools** (26): send/edit/delete/forward/copy messages, pins, chat info/management, moderation (ban/restrict/promote), polls, forum topics, permissions, invite links.

User-installed MCP servers (via workspace `.mcp.json`) are loaded alongside built-in tools via Claude Code's `--plugin-dir`.

## Session Management

Sessions map channels to Claude Code subprocesses:

```
SessionKey = agent_id + channel_type + channel_id [+ thread_id]
```

**Lifecycle**:
```
Empty → Active (claude -p subprocess alive)
          ↓ idle 30 min
        Suspended (subprocess killed, session_id preserved for --resume)
          ↓ idle 7 days
        Archived (summary written to memory, start fresh)
```

**Concurrency control**: configurable max concurrent sessions with priority queue (DM > mention > message > heartbeat > cron). Excess requests are queued.

**Stateless restart**: all session state persists to SQLite. Kill the daemon, restart, and sessions automatically resume via `--resume`.

## Configuration

All config lives in `catclaw.toml`, managed via CLI/TUI:

```toml
[general]
workspace = "./workspace"
state_db = "./state.sqlite"
max_concurrent_sessions = 3
session_idle_timeout_mins = 30
session_archive_timeout_hours = 168  # 7 days
ws_port = 21130  # Gateway server (WS + MCP on single port)

[[channels]]
type = "discord"
token_env = "CATCLAW_DISCORD_TOKEN"
guilds = ["123456789"]
activation = "mention"  # DMs always respond; this controls server channels

[[channels]]
type = "telegram"
token_env = "CATCLAW_TELEGRAM_TOKEN"
activation = "mention"  # DMs always respond; this controls group chats

[[agents]]
id = "main"
workspace = "./workspace/agents/main"
default = true
```

## Roadmap

### Phase 1: Core &mdash; Complete

- [x] CLI with all subcommands (`init`, `gateway`, `tui`, `stop`, `agent`, `channel`, `bind`, `config`, `session`, `skill`, `task`)
- [x] Unified startup flow (`catclaw` = splash &rarr; auto-init &rarr; background gateway &rarr; TUI)
- [x] Background gateway with PID file lifecycle (`catclaw stop`)
- [x] Config system (TOML read/write, interactive init wizard with per-channel setup)
- [x] State DB (SQLite WAL, sessions, tasks, bindings)
- [x] Agent system (registry, workspace loader, tool permissions, system directives)
- [x] Session manager (spawn, resume, fork, priority queue, persistence)
- [x] Claude Code subprocess (bidirectional NDJSON via `--input-format stream-json`)
- [x] Channel adapter abstraction (`ChannelAdapter` trait, `MsgContext`)
- [x] Discord adapter (serenity, typing, threads, chunked messages, 32 MCP actions)
- [x] Telegram adapter (teloxide long polling, 26 MCP actions)
- [x] Built-in MCP server (axum, MCP JSON-RPC, tool routing to adapters)
- [x] WS + MCP on single port (axum: `/ws` WebSocket, `/mcp` MCP HTTP)
- [x] Message router (binding resolution, agent dispatch)
- [x] Gateway main loop (restart recovery, SIGTERM handling, session cleanup)
- [x] TUI with 7 fully functional panels (Catppuccin Mocha, splash screen, tab navigation)
- [x] TUI Config panel (channel activation/guilds editable, inline editing, auto-save)
- [x] Full-screen Markdown editor (tui-textarea, Ctrl+S save, Ctrl+Q close)
- [x] Scheduler (heartbeat, cron, one-shot, archive cleanup)
- [x] Skills system (built-in skills, per-agent install/enable/disable)
- [x] Session transcript logging (JSONL)
- [x] Archive with AI-generated summary

### Phase 1d: Logging &mdash; Complete

- [x] JSON structured logging to file (workspace/logs/, daily rotation, JSONL)
- [x] Log levels with configurable minimum level
- [x] `catclaw logs` CLI (tail -f, level/grep/time filters, JSON output)
- [x] TUI Logs panel (search, highlight, level filter, structured fields)

### Phase 2: Memory &mdash; Planned

- [ ] Memory tables in SQLite (per-agent + shared)
- [ ] Embedding engine (Ollama / nomic-embed-text)
- [ ] MD to chunk pipeline (sliding window, 400 tokens)
- [ ] Hybrid search (vector cosine 70% + BM25 30%)
- [ ] File watcher (auto-reindex on MD changes)
- [ ] Plugin hooks (`PreCompact` / `Stop` for auto memory capture)

### Phase 3: Autonomy &mdash; Partial

- [x] Scheduler (heartbeat, cron, one-shot, archive cleanup)
- [x] Session archive with AI-generated summary
- [ ] BOOT.md startup flow
- [ ] `sessions_history` MCP tool (cross-session search)

### Phase 4: Collaboration & Extensions

- [ ] Slack adapter
- [ ] Agent-to-agent collaboration (`sessions_spawn`, `agent_send`)
- [ ] `/bind` Discord slash command
- [ ] MCP action permissions (per-agent whitelist/blacklist for channel tools)
- [ ] OpenClaw migration tool

## Comparison with OpenClaw

| Aspect | OpenClaw | CatClaw |
|---|---|---|
| Language | TypeScript | Rust |
| LLM access | OAuth token (banned) | Claude Code CLI subscription |
| Skill system | Custom (ClawHub) | Claude Code native `--plugin-dir` |
| Tool permissions | JSON config cascade | `--allowedTools` / `--disallowedTools` |
| Channel adapters | ChannelPlugin interface | `ChannelAdapter` trait (same pattern) |
| State management | In-memory + disk | SQLite-first (stronger restart guarantees) |
| Memory | MD + SQLite + sqlite-vec | Same architecture, Rust implementation |
| TUI | None | Catppuccin Mocha, 7 panels |

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

## License

MIT

---

<p align="center">
  Built with Rust and Claude Code<br>
  <strong>CatiesGames</strong>
</p>
