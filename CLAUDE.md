# CatClaw — Development Guide

## Project Overview

Rust gateway for Claude Code CLI — multi-agent, multi-channel (Discord/Telegram/TUI), session management with tool approval system.

## Architecture

```
Channel Adapters (Discord/Telegram) → MsgContext → MessageRouter → SessionManager → ClaudeHandle (claude -p subprocess)
                                                      ↑
                                        WS Server ← TUI/WebUI (GatewayClient)
                                        MCP Server ← Claude CLI (tool calls)
```

### Key Design Patterns

- **Hot-reload**: Config changes go through WS `config.set` → gateway applies immediately. Agent tool/approval changes go through `agents.reload_tools`.
- **Shared state**: `Config` uses `Arc<RwLock<Config>>`, `AgentRegistry` uses `Arc<RwLock<AgentRegistry>>`, `AdapterFilter` uses `Vec<Arc<RwLock<AdapterFilter>>>`. All hot-reloadable.
- **Approval system**: PreToolUse hook → WS `approval.request` → broadcast to TUI + forward to origin channel → user approves/denies → hook receives result.
- **Session metadata**: JSON in `metadata` column stores `model`, `channel_id`, `sender_id`. Use `SessionRow` helper methods.
- **Channel adapters**: `ChannelAdapter` trait with `send_approval()` default method. Discord uses embed+buttons, Telegram uses inline keyboard.

## Critical Rules

### RwLockReadGuard Across Await
**NEVER hold a `std::sync::RwLockReadGuard` across an `.await` point** — it's not `Send` and will fail to compile inside `tokio::spawn`. Always extract to a local variable first:
```rust
// WRONG: guard lives across await
if let Some(agent) = registry.read().unwrap().get(&id).cloned() {
    do_something(agent).await;  // guard still alive here!
}

// RIGHT: drop guard before await
let agent = registry.read().unwrap().get(&id).cloned();
if let Some(agent) = agent {
    do_something(agent).await;
}
```

### ApprovalConfig Default
`ApprovalConfig` has a manual `Default` impl that sets `timeout_secs = 120`. Do NOT use `#[derive(Default)]` — it would give `timeout_secs = 0`.

### Hot-Reload Rules
Any agent config change (tools, approval, model), whether from TUI or CLI, must:
1. Write to disk (`tools.toml` + `catclaw.toml`)
2. Call WS `agents.reload_tools` to notify gateway to update the in-memory `AgentRegistry`

The `agents.reload_tools` handler re-reads config from disk and syncs to memory, covering approval, tools, model, and fallback_model.

**Global settings** (`config set` family) go through WS `config.set`, and the gateway auto-reloads (adapter filters, log level, etc.). `apply_config_set` returns `Ok(false)` for immediate effect, `Ok(true)` for requires restart.

**Bindings** currently live in the in-memory `MessageRouter`. Changes require a gateway restart to take effect (consistent between CLI and TUI).

**Design principle**: When CLI and TUI modify the same setting, they must follow the exact same hot-reload path. One side cannot notify the gateway while the other does not.

### Claude Code CLI Flags
- `--dangerously-skip-permissions` does NOT skip hooks. Hooks (PreToolUse) still fire.
- `--session-id` creates new session, `--resume` resumes existing one.
- `--include-partial-messages` needed for `stream_event` type partial events.
- `--settings` injects hook config as JSON. Only injected when `approval.is_empty() == false`.
- `--tools` is the whitelist (only these tools available). `--disallowedTools` is the blacklist.
- `--allowedTools` only controls permission prompts, NOT tool availability.

### CLI / TUI Feature Parity
CatClaw's CLI (`catclaw` commands) and TUI (terminal interface) must support **exactly the same operations**. When adding any feature, all of the following must be implemented:
- **CLI** (`src/main.rs` subcommand + handler) — the agent's hands via Bash tool
- **TUI** (`src/tui/` corresponding panel) — the user's hands via terminal
- **catclaw skill** (`src/agent/loader.rs` `SKILL_CATCLAW` constant) — the agent's brain (CLI usage docs)
- **README.md** — keep documentation in sync with actual features

All three are required. CLI is the agent's hands, TUI is the user's hands, skill is the agent's brain.

### Global Plugins Are Loaded
`claude -p` automatically loads the user's global plugins (`~/.claude/plugins/`), including pencil, LSP, playground, etc. CatClaw cannot exclude them (no `--exclude-plugin` flag). Impact: agent tool lists and skill indexes will contain unnecessary items, increasing token consumption. No solution yet — noted for future investigation.

### Skill Triggering Is Claude's Decision
Skills are not auto-triggered — the system prompt only includes a skill index (name + one-line description), and Claude decides whether to use `/skill-name` to load the full content based on the description. If a skill needs to **always be active** (e.g., injection-guard), put a condensed version of the core rules in AGENTS.md or TOOLS.md (loaded every time), and keep the full version in the skill.

### MCP Tools Permission Constraints (Claude Code CLI)
- `--tools` (whitelist) **only restricts built-in tools**, MCP tools are unaffected.
- `--disallowedTools` (blacklist) can block MCP tools (`mcp__pencil__*`), but requires enumeration.
- **Global MCP tools (installed at the Claude Code level by the user) are automatically loaded into all `claude -p` subprocesses. CatClaw cannot control and should not manage them.** This is a Claude Code CLI limitation, not CatClaw's responsibility. CatClaw only manages its own injected MCP server (catclaw built-in) and the agent workspace's `.mcp.json`.
- TUI Tools list has three sections: Built-in Tools, CatClaw MCP Tools, User MCP Servers. Global MCP tools are not listed.
- User MCP is a shared pool like skills: definitions live in `workspace/.mcp.json`, shared by all agents, each agent controls enable/disable via denied list.
- User MCP is currently managed at the server level (`mcp__{server}__*`), because catclaw does not start MCP servers to query `tools/list`. Future improvement: query specific tool lists at startup.

### CLAUDE.md Is Visible to Agents
Claude Code automatically searches upward for CLAUDE.md. If the catclaw binary runs inside the source tree (e.g., `target/release/`), agent subprocesses will find and load the development CLAUDE.md. **For production deployment, ensure the binary does not run inside the source tree**, or place an empty CLAUDE.md in the workspace directory to block upward search.

### tools.toml Three-State Design
Each tool exists in exactly one list: `allowed` (directly usable), `denied` (unavailable), `require_approval` (usable but requires confirmation).
- `allowed` + `require_approval` both go into the `--tools` whitelist (a tool must be "available" for the hook to intercept it)
- `denied` goes into the `--disallowedTools` blacklist
- `catclaw.toml`'s `[agents.approval]` only keeps `timeout_secs` (global); tool lists are entirely in `tools.toml`

### Serialization Gotchas
- `ApprovalConfig.timeout_secs`: skip serializing if 120 or 0 (via `is_default_approval_timeout`).
- `ApprovalConfig.require_approval` / `blocked`: skip if empty.
- TOML serialization uses `toml::to_string_pretty`. Watch for field ordering when adding new serde attributes.

## File Map

| File | Purpose |
|------|---------|
| `src/gateway.rs` | Gateway startup, `GatewayHandle`, adapter wiring, approval channel setup |
| `src/ws_server.rs` | WS + MCP server, all JSON-RPC handlers including `agents.reload_tools` |
| `src/router.rs` | Message routing: binding resolution → agent dispatch → session |
| `src/session/manager.rs` | `SessionManager`, `SenderInfo`, session lifecycle (create/resume/fork/archive) |
| `src/session/claude.rs` | `ClaudeHandle` — subprocess spawn, stdin/stdout streaming |
| `src/channel/mod.rs` | `ChannelAdapter` trait, `MsgContext`, `OutboundMessage`, `send_approval()` |
| `src/channel/discord.rs` | Discord adapter: serenity handler, approval embed+buttons, `interaction_create` |
| `src/channel/telegram.rs` | Telegram adapter: teloxide dispatcher, approval inline keyboard, `callback_query` |
| `src/agent/mod.rs` | `Agent`, `AgentRegistry`, `ToolPermissions`, claude args builder, system prompt |
| `src/agent/loader.rs` | Agent workspace creation, skill management, TOML loading |
| `src/config.rs` | `Config`, `ApprovalConfig`, `config_get`/`apply_config_set` |
| `src/state.rs` | `StateDb` (SQLite WAL), `SessionRow` with platform ID helpers |
| `src/approval.rs` | Approval types: `PendingApproval`, `ApprovalPendingEvent`, `HookInput` |
| `src/dist.rs` | Self-update (GitHub Releases) + system service (launchd/systemd) + uninstall |
| `src/cmd_hook.rs` | PreToolUse hook binary logic |
| `src/tui/agents.rs` | TUI Agents panel: tools 3-state toggle (allowed/approval/denied) |
| `src/tui/config_panel.rs` | TUI Config panel: editable settings including `approval.timeout_secs` |
| `src/scheduler.rs` | Heartbeat, cron, archive cleanup |

## Language Conventions

- **Code**: English (variable names, function names, struct names, log messages, code comments)
- **Communication**: Chinese (conversations with the user, commit messages, task names)
- **Skill content**: English (agent operation manuals, since Claude Code primarily operates in English)
- **README.md**: English

## New Config Key Checklist

Adding a new configurable key requires changes in all of the following:

1. `src/config.rs` — struct field + `config_get()` + `apply_config_set()` + serde attributes
2. `src/tui/config_panel.rs` — `build_entries()` add `ConfigEntry` + `completions_for_key()` if options exist
3. `src/agent/loader.rs` — `SKILL_CATCLAW` constant's config key table
4. `README.md` — Configuration section

For per-agent settings (not global):
1. `src/config.rs` — `AgentConfig` or sub-struct
2. `src/tui/agents.rs` — corresponding UI operation
3. `src/main.rs` — CLI subcommand flag
4. `src/ws_server.rs` — `handle_agents_reload_tools` must cover the new field in hot-reload
5. `src/agent/mod.rs` — `Agent` struct + `reload_agent_config()` + `claude_args_with_mcp()` if it affects launch arguments

## Embedded Skill Update Flow

Skills are `const` string literals in `src/agent/loader.rs`, compiled into the binary. They are installed to the user's workspace at:
- `catclaw agent new` — all built-in skills are auto-installed when creating a new agent
- `catclaw onboard` — installed during initialization

**After updating skill content**: `cargo build --release` produces a new binary, but already-installed workspace files are not auto-updated. Manually overwrite or delete `workspace/skills/{name}/SKILL.md` so the next `agent new` recreates it.

## WS Protocol Methods

JSON-RPC methods supported by the gateway WS server (`/ws`):

| Method | Purpose | Hot-reload |
|--------|---------|-----------|
| `gateway.status` | Query agent count, active sessions | — |
| `sessions.list` / `.delete` / `.stop` | Session CRUD | — |
| `sessions.send` | Send message to session (streaming/non-streaming) | — |
| `sessions.transcript` | Read session transcript | — |
| `sessions.set_model` | Set session model override | — |
| `agents.list` / `.get` / `.default` | Agent queries | — |
| `agents.reload_tools` | Hot-reload agent config (tools, approval, model) | YES |
| `tasks.list` / `.enable` / `.disable` / `.delete` | Scheduled task CRUD | — |
| `config.get` / `.set` | Global config read/write | YES (some require restart) |
| `approval.request` / `.respond` / `.list` | Tool approval flow | — |

When adding a new WS method, update this table and the `dispatch()` function in `src/ws_server.rs`.

## Build & Test

```bash
cargo check          # Fast type-check
cargo build --release  # Production build (output: target/release/catclaw)
cargo clippy         # Lint
```

Always run `cargo check` after changes — zero errors AND zero warnings required.

No unit tests currently — verification relies on `cargo check` (zero errors, zero warnings) + manual TUI/CLI testing.

## Dependencies (version constraints)

- `tui-textarea 0.7` requires `ratatui 0.29` + `crossterm 0.28` (not 0.30/0.29)
- `serenity 0.12` + `poise 0.6`
- `tokio-tungstenite 0.24` for WS

## Lessons Learned

1. **Writing files from TUI does not mean the gateway picks it up** — any change affecting gateway in-memory state (agent approval, tool permissions) must notify the gateway via WS method for hot-reload.
2. **`#[derive(Default)]` is a trap for config structs with custom defaults** — `u64`'s Default is 0, not the desired 120. Use manual `impl Default`.
3. **`std::sync::RwLockReadGuard` is not `Send`** — cannot be held across `.await`. Extract to a local variable before awaiting.
4. **Approval hook is only injected when `!approval.is_empty()`** — if `require_approval` in config is empty, `--settings` is not added to claude args and the hook does not fire.
5. **Config panel vs Agents panel responsibility split** — global settings (timeout_secs) go in Config panel; per-agent settings (which tools need approval) go in Agents > Tools.
6. **Feature updates must sync the skill** — the catclaw skill (`SKILL_CATCLAW` constant in `src/agent/loader.rs`) is the agent's operation manual. Any new feature (CLI flag, config key, TUI operation) must be reflected in the skill content, otherwise the agent won't know how to guide the user. Same applies to README.md.
7. **`Arc<AgentRegistry>` changed to `Arc<RwLock<AgentRegistry>>`** — to support hot-reload, the registry must be mutable. Read with `.read().unwrap()`, write with `.write().unwrap()`. All `.get()` calls need `.cloned()` to get an owned `Agent` and avoid holding the guard across await.
8. **Hook subprocess cannot create a new tokio runtime** — `catclaw hook pre-tool` runs as a subprocess where `main` is already `#[tokio::main]` (runtime exists). Using `tokio::runtime::Builder` in `cmd_hook.rs` to create a second runtime would panic. Use `async fn` + `.await` instead.
