# CatClaw — Development Guide

## Project Overview

Rust gateway for Claude Code CLI — multi-agent, multi-channel (Discord/Telegram/Slack/TUI), session management with tool approval system.

## Architecture

```
Channel Adapters (Discord/Telegram/Slack) → MsgContext → MessageRouter → SessionManager → ClaudeHandle (claude -p subprocess)
                                                      ↑
                                        WS Server ← TUI/WebUI (GatewayClient)
                                        MCP Server ← Claude CLI (tool calls)
```

### Key Design Patterns

- **Hot-reload**: Config changes go through WS `config.set` → gateway applies immediately. Agent tool/approval changes go through `agents.reload_tools`.
- **Shared state**: `Config` uses `Arc<RwLock<Config>>`, `AgentRegistry` uses `Arc<RwLock<AgentRegistry>>`, `AdapterFilter` uses `Vec<Arc<RwLock<AdapterFilter>>>`. All hot-reloadable.
- **Approval system**: PreToolUse hook → WS `approval.request` → broadcast to TUI + forward to origin channel → user approves/denies → hook receives result.
- **Session metadata**: JSON in `metadata` column stores `model`, `channel_id`, `sender_id`. Use `SessionRow` helper methods.
- **Channel adapters**: `ChannelAdapter` trait with `send_approval()` default method. Discord uses embed+buttons, Telegram uses inline keyboard, Slack uses Block Kit buttons.
- **Streaming**: `ChannelCapabilities.streaming` flag. Slack supports native AI streaming (`chat.startStream`/`appendStream`/`stopStream`). Adapters implement optional `send_stream_start()`/`send_stream_append()`/`send_stream_stop()` methods.

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

**Bindings** are hot-reloaded — `MessageRouter::bindings` is wrapped in `RwLock` and the WS handlers (`bindings.set` / `bindings.delete`) call `router.set_bindings(...)` after writing `catclaw.toml`. No gateway restart needed.

**Design principle (rewritten)**: **Gateway is the sole owner of disk + in-memory state.** All writes to `catclaw.toml` and per-agent `tools.toml` must go through the WS server. CLI subprocesses, TUI panels, and agents (via Bash) are all WS clients — they call methods like `agents.new` / `bindings.set` / `agents.set_tools` / `config.set` and the gateway atomically writes disk + updates memory + notifies router/registry. CLI commands fall back to direct file write only when the gateway is offline (and print "will apply on next start"). The legacy "TUI writes file then notifies gateway" pattern is gone — it caused stale-memory writes to silently delete entries (see lesson #1).

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

### Scheduled Tasks: `--at` One-Shot
`catclaw task add` supports `--at` for absolute-time one-shot scheduling. Accepts ISO 8601 (`2026-03-20T09:00:00`), RFC 3339, or `HH:MM` / `HH:MM:SS` (today, local timezone). Mutually exclusive with `--cron`, `--every`, `--in-mins`. Time must be in the future. Times without explicit timezone are interpreted as the gateway's local timezone.

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
| `src/channel/discord.rs` | Discord adapter: serenity handler, slash commands (`/stop`, `/new`), approval embed+buttons, `interaction_create` |
| `src/channel/slack.rs` | Slack adapter: Socket Mode WS, approval Block Kit buttons, native AI streaming |
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
| `src/tui/social_inbox.rs` | TUI Social Inbox panel: list/filter/approve/discard inbox items |
| `src/tui/social_drafts.rs` | TUI Social Drafts panel: outgoing draft queue, filter by status, approve/discard |
| `src/scheduler.rs` | Heartbeat, cron, archive cleanup, diary extraction (→ palace DB), social polling |
| `src/memory/mod.rs` | Memory palace data types, chunking logic |
| `src/memory/db.rs` | StateDb memory CRUD (memory_nodes, vec_memories, kg_*) |
| `src/memory/embed.rs` | fastembed wrapper (BGE-M3, 1024 dims) |
| `src/memory/search.rs` | Hybrid search: FTS5 + sqlite-vec cosine + RRF merge |
| `src/memory/kg.rs` | Knowledge graph: entities, triples, temporal queries |
| `src/memory/tools.rs` | 11 MCP tool schemas + execute dispatch |
| `src/memory/analyze.rs` | Haiku post-processing: diary → summary + room + facts + KG |
| `src/memory/context.rs` | L1 wake-up context generator (top-importance → system prompt) |
| `src/memory/migrate.rs` | One-time migration: markdown diary/MEMORY.md → palace DB |
| `src/social/mod.rs` | Social Inbox core types (`SocialItem`, `ResolvedAction`), action router, `run_ingest()` orchestrator |
| `src/social/instagram.rs` | Instagram Graph API client (`InstagramClient`) — profile, media, comments, insights |
| `src/social/threads.rs` | Threads API client (`ThreadsClient`) — timeline, replies, two-step post/reply |
| `src/social/webhook.rs` | Axum webhook handlers for `/webhook/instagram` and `/webhook/threads` with HMAC verification |
| `src/social/poller.rs` | Polling logic for Instagram/Threads feeds, cursor management via `social_cursors` table |
| `src/social/forward.rs` | Forward card builder (`ForwardCard`) + adapter-specific renderers for Discord/Slack/Telegram |
| `src/contacts/mod.rs` | Contacts core types (`Contact`, `ContactChannel`, `ContactDraft`, `ContactRole`, `ContactPayload`) + `owning_agents()` helper for v2 multi-agent extension |
| `src/contacts/pipeline.rs` | Outbound pipeline: `submit_reply` / `approve_draft` / `discard_draft` / `request_revision` / `mirror_inbound` / `try_manual_reply` |
| `src/contacts/tools.rs` | `mcp__catclaw__contacts_*` MCP tool schemas + dispatch (14 tools) |
| `src/channel/line.rs` | LINE adapter: webhook handler with HMAC verify, reply-token + push API, image/follow/unfollow inbound, 11 `line_*` MCP actions (rich menu / flex / quota / profile) |
| `src/tui/contacts.rs` | TUI Contacts panel with [Contacts] / [Drafts] sub-tabs |

## Memory Palace System (MemPalace)

Structured memory system in `src/memory/` backed by SQLite (state.db), replacing the old markdown-based diary+distillation system. Based on the MemPalace competition-winning design.

### Architecture
- **Spatial organization**: Wing (agent isolation) → Room (topic, auto-classified by Haiku) → Hall (memory type: facts/events/discoveries/preferences/advice)
- **Verbatim storage**: Raw content in memory_nodes (drawers), AI-generated summary (closets)
- **Hybrid search**: FTS5 full-text + sqlite-vec cosine similarity, merged via Reciprocal Rank Fusion (RRF). Supports `cross_wing` for multi-agent search.
- **Knowledge graph**: Temporal entity-relationship triples with valid_from/valid_to
- **Tunnels**: Rooms shared across multiple wings, discoverable via `memory_tunnels` tool
- **L1 context**: Top-importance memories (≥7) auto-loaded into system prompt (~800 tokens)
- **11 MCP tools**: memory_status/write/search/delete/list_wings/list_rooms/tunnels + kg_add/invalidate/query/timeline
- **Automatic post-processing**: After diary extraction, Haiku analyzes the diary to produce summary (closet), room classification, extracted facts, and KG triples

### Key files
- `src/memory/mod.rs` — Data types (WriteRequest, MemoryNode, SearchResult, Triple, DiaryAnalysis), chunking
- `src/memory/db.rs` — StateDb CRUD methods for memory_nodes, vec_memories, kg_*, tunnels
- `src/memory/embed.rs` — fastembed wrapper (BGE-M3, 1024 dims)
- `src/memory/search.rs` — Hybrid search (FTS5 + vector + RRF merge), supports cross-wing
- `src/memory/kg.rs` — Knowledge graph operations on StateDb
- `src/memory/tools.rs` — MCP tool schemas and dispatch (11 tools)
- `src/memory/analyze.rs` — Haiku post-processing: diary → summary + room + facts + KG triples
- `src/memory/context.rs` — L1 wake-up context generator
- `src/memory/migrate.rs` — One-time migration from old markdown files

### Diary extraction pipeline
`check_diary_extraction()` in `src/scheduler.rs` runs every 60s tick:
1. **Diary generation** (agent's model): transcript → diary text → `memory_nodes` (hall=events, source=diary)
2. **Haiku post-processing** (background, non-blocking): diary text → `analyze_diary()` in `src/memory/analyze.rs`:
   - Produces summary (closet) and room classification
   - Extracts facts/preferences/advice as separate `memory_nodes` (source=extraction, importance=7-9)
   - Creates KG triples for entity relationships
   - Generates embeddings for diary + facts
Distillation has been removed — importance field + Haiku extraction replaces it.

### Migration
On first startup after upgrade, `run_migration()` in gateway.rs imports existing `memory/*.md` diary files and `MEMORY.md` into palace DB. Controlled by `palace_meta.migration_v1` key. Old files are preserved but no longer read by the system.

## Contacts System

Cross-platform identity layer in `src/contacts/`. CatClaw stores **who** the agent talks to (across DC/TG/Slack/LINE) but **not the business data** — that is the agent's responsibility (Notion / palace / self-managed). 

### Three tables
- `contacts` (id, agent_id, role, tags, forward_channel, approval_required, ai_paused, external_ref JSON, metadata JSON)
- `contact_channels` (platform, platform_user_id) → contact_id, with last_active_at
- `contact_drafts` (status: pending → awaiting_approval → sent / ignored / revising / failed, payload JSON, forward_ref, revision_note)

### Outbound pipeline (agent → contact)
`agent → contacts_reply → draft (pending) → mirror to forward_channel → approval gate (if approval_required) → adapter.send (via or last-active channel) → status=sent/failed`. Agents **cannot bypass** this — the only outbound path is `contacts_reply`. Direct `discord_send_message`/native sends from the agent will not show up in admin's forward channel and will not respect approval.

### Inbound pipeline (contact → agent)
Router (`src/router.rs`) checks `contact_channels` for the sender:
1. If matched, touch `last_active_at`, mirror to `forward_channel`, inject `[Contact: ...]` into system prompt; if `ai_paused`, skip agent dispatch entirely.
2. If sender NOT matched, check whether the inbound channel itself IS a `forward_channel` of any contact — if so, treat as **manual reply** and forward verbatim to the contact (admin types directly in `#client-foo` Discord channel → contact gets the message under the agent's identity). The agent is not invoked.

### Multi-agent extension predicate
`Contact::owning_agents() -> Vec<AgentId>` is the abstraction layer for v1→v2 migration. v1 returns `vec![self.agent_id]`. To enable multi-agent shared contacts, migrate to a `contact_agents` join table and update only this helper — call sites unchanged.

### LINE specifics
LINE is the first channel built with contacts in mind. The adapter (`src/channel/line.rs`) registers via `GatewayHandle.line_adapter` (concrete-typed, like `backend_adapter`) so the axum webhook handler can call `verify_signature` + `handle_webhook_payload` directly. Reply token cache (5-min validity) per LINE userId; outbound auto-tries reply token then falls back to push API. **Rich Menu is fully agent-managed** — CatClaw stores no `role↔menu` mapping; agents create menus via `line_rich_menu_*` actions and store the IDs themselves (in `contacts.external_ref` or memory).

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
- **Gateway startup** — built-in skills are always overwritten with the version compiled into the binary. User modifications to built-in skill files will not survive a restart. Custom (non-built-in) skills are never touched.

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
| `agents.reload_tools` | Hot-reload agent config (tools, approval, model) — legacy entry-point, prefer `agents.set_tools` | YES |
| `agents.new` | Create agent (workspace + skills + catclaw.toml + registry) | YES |
| `agents.delete` | Remove agent from catclaw.toml + registry (workspace files preserved) | YES |
| `agents.set_tools` | Write `tools.toml` + reload registry — single entry-point for tool permission edits | YES |
| `agents.set_model` | Update `model` / `fallback_model` in catclaw.toml + registry | YES |
| `agents.set_runtime` | Switch agent runtime (claude ↔ codex) in place — rebuilds the registry Agent (not just `reload_agent_config`), resets model to runtime default, archives old sessions | YES |
| `agents.set_default` | Mark an agent as default in catclaw.toml + registry | YES |
| `bindings.set` | Upsert binding (pattern → agent) — calls `router.set_bindings()` so route table is live | YES |
| `bindings.delete` | Remove binding by pattern + reload router | YES |
| `tasks.list` / `.enable` / `.disable` / `.delete` | Scheduled task CRUD | — |
| `config.get` / `.set` | Global config read/write | YES (some require restart) |
| `approval.request` / `.respond` / `.list` | Tool approval flow | — |
| `mcp_env.list` / `.get` / `.set` / `.remove` | MCP env var management | YES (hot-reload) |
| `mcp.tools` | Query discovered MCP tools per server | — |
| `social.inbox.list` | Query social_inbox (supports `status` filter) | — |
| `social.inbox.get` | Get single inbox item by ID | — |
| `social.inbox.approve` | Approve draft → call Meta API → status=sent | — |
| `social.inbox.discard` | Discard draft → status=ignored | — |
| `social.inbox.reprocess` | Reset item to pending, re-run action router | — |
| `social.poll` | Trigger manual poll for instagram / threads | — |
| `social.mode` | Hot-reload platform mode (polling/webhook/off) | YES |
| `social.draft.list` | Query social_drafts (supports `platform`, `status`, `limit` filters) | — |
| `social.draft.approve` | Approve draft → call Meta API → status=sent | — |
| `social.draft.discard` | Discard draft → status=ignored | — |
| `social.draft.submit_for_approval` | Called by hook: find latest draft, send review card, set status=awaiting_approval | — |
| `contact.list` / `.get` / `.update` / `.delete` | Contacts CRUD | — |
| `contact.bind` / `.unbind` | Bind/unbind platform user id ↔ contact | — |
| `contact.draft.list` / `.approve` / `.discard` / `.request_revision` | Contact outbound draft management | — |
| `contact.ai_pause` / `.ai_resume` | Pause/resume AI for a contact | — |

When adding a new WS method, update this table and the `dispatch()` function in `src/ws_server.rs`.

## Build & Test

```bash
cargo check          # Fast type-check
cargo clippy -- -D warnings  # Lint — MUST pass with zero warnings
cargo build --release  # Production build (output: target/release/catclaw)
```

**Both `cargo check` and `cargo clippy -- -D warnings` must pass with zero errors AND zero warnings.** CI enforces this on every push.

No unit tests currently — verification relies on cargo check + clippy + manual TUI/CLI testing.

### No Shortcuts Policy
Always do the complete fix. Never leave warnings, tech debt, or half-done work with "fix later" / "TODO" / "skip for now". If clippy complains, fix all warnings — not just the ones in your new code. If CI fails, fix the root cause — don't weaken the CI checks. Every commit should leave the codebase cleaner than it was.

## Dependencies (version constraints)

- `tui-textarea 0.7` requires `ratatui 0.29` + `crossterm 0.28` (not 0.30/0.29)
- `serenity 0.12` + `poise 0.6`
- `tokio-tungstenite 0.24` for WS

## Lessons Learned

**When to add a lesson:** Whenever the user corrects a wrong assumption, a bug is caused by misunderstanding the architecture, or a code review catches an issue that could have been avoided with better knowledge. Write the lesson here immediately — this is the single source of truth for project-specific lessons across all sessions.

1. **Gateway is the sole owner of `catclaw.toml` + per-agent `tools.toml` + the in-memory `Config` / `AgentRegistry` / `MessageRouter`.** Any write path (CLI subprocess, TUI panel, agent via Bash) must go through a WS method — `agents.new`, `agents.delete`, `agents.set_tools`, `agents.set_model`, `agents.set_runtime`, `agents.set_default`, `bindings.set`, `bindings.delete`, `config.set`, `mcp_env.set/remove`, `env.set/remove`, `social.mode`. The handler atomically writes disk + updates memory + notifies router/registry. **Why this matters:** earlier versions had CLI subprocess writing the file then exiting without telling the gateway, so the gateway's in-memory `Config` stayed stale; the next WS handler that re-serialised the in-memory `Config` (e.g. `mcp_env.set`) would silently overwrite the disk and delete CLI-added agents/bindings. Defensive bottom line: every WS handler that does a "whole-file rewrite" reloads `Config` from disk first (never from `gw.config.read().clone()`), so even if some new path forgets to notify, no data is lost.
2. **`#[derive(Default)]` is a trap for config structs with custom defaults** — `u64`'s Default is 0, not the desired 120. Use manual `impl Default`.
3. **`std::sync::RwLockReadGuard` is not `Send`** — cannot be held across `.await`. Extract to a local variable before awaiting.
4. **Approval hook is only injected when `!approval.is_empty()`** — if `require_approval` in config is empty, `--settings` is not added to claude args and the hook does not fire.
5. **Config panel vs Agents panel responsibility split** — global settings (timeout_secs) go in Config panel; per-agent settings (which tools need approval) go in Agents > Tools.
6. **Feature updates must sync the skill** — the catclaw skill (`SKILL_CATCLAW` constant in `src/agent/loader.rs`) is the agent's operation manual. Any new feature (CLI flag, config key, TUI operation) must be reflected in the skill content, otherwise the agent won't know how to guide the user. Same applies to README.md.
7. **`Arc<AgentRegistry>` changed to `Arc<RwLock<AgentRegistry>>`** — to support hot-reload, the registry must be mutable. Read with `.read().unwrap()`, write with `.write().unwrap()`. All `.get()` calls need `.cloned()` to get an owned `Agent` and avoid holding the guard across await.
8. **Hook subprocess cannot create a new tokio runtime** — `catclaw hook pre-tool` runs as a subprocess where `main` is already `#[tokio::main]` (runtime exists). Using `tokio::runtime::Builder` in `cmd_hook.rs` to create a second runtime would panic. Use `async fn` + `.await` instead.
9. **Transcript JSONL is ours, not Claude's** — CatClaw's transcript is written by `send_and_wait` (immediate append on each event), not by the Claude subprocess. After `stop_session()` kills the subprocess, no new events are produced, so the transcript is already complete. Don't assume async flush issues or add sleeps before reading transcript. **Rule:** Before reasoning about race conditions, clarify data flow ownership — who writes, when, and where.
10. **`Skill` is a built-in tool that must be in the `--tools` whitelist** — Claude Code agents load skills via the `Skill` tool, not `/` slash commands. If `--tools` whitelist is set but doesn't include `Skill`, agents can't load skills and will fall back to reading files manually with Bash. CatClaw now auto-injects `Skill` into every whitelist.
11. **Social Inbox is a separate subsystem, not a ChannelAdapter** — its staged approval flow (draft stored in DB indefinitely, admin reviews at their own pace) is fundamentally incompatible with the hook-based approval system (synchronous timeout). Never try to shoehorn social events into the ChannelAdapter trait.
12. **`Theme` in TUI is a unit struct with const colors, not an instance** — use `Theme::MAUVE`, `Theme::BASE`, etc. (static constants), not `Theme::default()` (doesn't exist). `Theme` has no fields.
13. **Social channel creation order in gateway.rs** — `social_item_tx`/`social_item_rx` must be created before the scheduler block (scheduler config references `social_item_tx`) but the ingest task spawn must happen after `adapters_list` is built. Solution: create the unbounded channel early, spawn the ingest task after `adapters_list` is available.
14. **TUI Agents > Tools uses a static list — must be kept in sync with mcp_server.rs** — `list_catclaw_mcp_tools()` in `src/tui/agents.rs` is a hardcoded list of `mcp__catclaw__*` tool names. It does NOT auto-discover from the running gateway. Any time a new built-in MCP tool is added to `mcp_server.rs`, it must also be added to `list_catclaw_mcp_tools()`. Social tools are conditional: only add when `config.social.instagram.is_some()` / `config.social.threads.is_some()`.
15. **Social card restore must be self-healing, not best-effort** — `update_forward_card` can fail (Discord edit-limits, rate limit, deleted message) and if the caller only `warn!`s, the inbox row silently desyncs from its UI — user sees a resolved card with no buttons while the DB says `pending`. Always restore via `forward::ensure_inbox_card_restored()`, which falls back to sending a new card and updating `forward_ref`. Pair with `forward::notify_admin()` when even the fallback fails so the human knows to run `catclaw social reprocess <id>` / `/social-reprocess` in Discord.
16. **Router needs adapter map, must be built after adapters** — contacts forward mirroring + manual reply detection both require `Arc<HashMap<String, Arc<dyn ChannelAdapter>>>`. The original gateway.rs created `MessageRouter` in step 5 (before adapters), then built adapters in step 7. Now router construction is deferred until step 9 so `set_adapters()` can inject the populated map. **Rule:** when adding a router-level dependency on adapter state, double-check the construction order in gateway.rs.
17. **Contacts business data is NOT CatClaw's responsibility** — `contacts.metadata` is for slow-changing profile (allergies, goals); `contacts.external_ref` is a free-form JSON pointer (e.g. Notion page id). Per-day metrics, training logs, counseling notes belong in agent-managed external storage (Notion MCP / palace / self-managed SQLite). Do NOT add domain-specific schemas to CatClaw — that locks the system to one vertical.
18. **Every reply destined for a contact MUST traverse `contacts::pipeline::submit_reply`** — this includes (a) agents calling `contacts_reply` explicitly, AND (b) the router's own terminal-text outbound path in `router.rs::route()`. The router used to call `adapter.send` directly with the agent's final text, which silently bypassed the approval gate even when `approval_required=true`. Now the router checks for a known non-admin contact at the send step and routes through `submit_reply`; the pipeline branches on `approval_required` internally (true → work card awaiting approval; false → auto-send + work card audit trail). Admin contacts skip the gate (the operator IS the admin; cards reviewing their own messages = noise). **Direct platform-level send tools** (`line_send_message`, `line_send_flex`, `discord_send_message`, etc) intentionally remain exposed as MCP tools — they are needed for legitimate proactive outreach to non-contact targets (broadcasts, groups, unknown users). SKILL_CATCLAW + SKILL_LINE teach the agent: **target is a contact → write text (Path A: router auto-pipelines) or `contacts_reply` (Path B: explicit proactive)**; **target is NOT a contact → use platform send tool directly**. The enforcement is prompt-level, not structural, because runtime interception of MCP tools would block legitimate broadcast/group use-cases.
19. **LINE adapter stores reply tokens per LINE userId; check expiry before reuse** — reply tokens are valid 5 minutes from inbound event. `LineAdapter.reply_tokens` is `RwLock<HashMap<userId, (token, expires_unix)>>`. `take_reply_token()` consumes the token (one-shot) and returns None when expired. Outbound `send()` always tries reply token first, then push API.
20. **LINE adapter is registered TWICE in gateway**: once into `adapters` (the generic `Arc<dyn ChannelAdapter>` map for router/MCP dispatch) AND once into `GatewayHandle.line_adapter` as concrete `Arc<LineAdapter>` (so the axum webhook handler can call `verify_signature` + `handle_webhook_payload` without trait downcast). Same pattern as `backend_adapter`. Don't try to fold these into one — Rust doesn't support trait-object downcast cleanly.
21. **`ChannelType::Line` enum variant must include in every match** — adding a new ChannelType variant requires updating `as_str()` in `src/channel/mod.rs`. Compiler catches missing arms; just don't fall back to a wildcard `_ =>` in places that need explicit per-platform routing.
22. **toC entry points auto-register unknown contacts to prevent stranger LLM cost** — when `contacts.enabled=true`, every inbound on a toC entry point auto-creates a `role=unknown` contact + binds the platform userId (no LLM). Router sees `role=unknown` and skips agent dispatch entirely (storage-only). Promotion to client/admin is a deliberate human action via `contacts_update`. **Uniform rule: ONLY 1:1 direct messages enter contacts — group/guild/room is always out of scope, on every platform.** The toC entry points are: **LINE DMs** (`source.type == "user"`, incl. follow events, `LineAdapter::ensure_unknown_contact`), **Discord DMs** (`is_dm`, `ensure_unknown_discord_contact`), and **Telegram private chats** (`is_private`, `ensure_unknown_telegram_contact`). Groups are workspace chat (admin ↔ agent) and would contaminate the contacts table. Slack still does NOT auto-register (no toC story yet). **Two gates must agree** (lesson #41): the adapter's auto-register gate (DM-only) AND `router::route`'s contact-lookup gate (`if ctx.is_direct_message`). If only one is DM-gated you get split-brain — e.g. a LINE/Telegram client who's bound on DM sends in a group, the adapter skips auto-register but the router still looks them up by `sender_id`, pulls the group message into the approval/forward pipeline, and replies in the DM instead of the group (the bug behind lesson #41). See lesson #38 for per-platform default-agent ownership.
23. **Forward channel doubles as agent chat — use `>>` prefix for manual reply** — a forward_channel is both (a) where the admin sees mirrors and work cards and (b) where they chat with the agent about that contact. To distinguish "I want to talk to the agent" from "relay this verbatim to the contact", `pipeline::try_manual_reply` only fires when the message starts with `>>` (constant `MANUAL_REPLY_PREFIX`). Without prefix, the message goes through normal agent dispatch. Used to be `if contact.is_none()` gate (broke when admin themselves was a bound contact); now applies to all senders. SKILL_CATCLAW Contacts section teaches the agent to explain this to the user when first setting forward_channel.
24. **`forward_channel` falls back to `contacts.unknown_inbox_channel`** — when a contact has `forward_channel = None` but `approval_required = true`, work cards would have nowhere to render and approvals would stall silently. `Contact::effective_forward_channel(unknown_inbox)` returns the per-contact channel when set, otherwise the global fallback. All pipeline functions (`submit_reply`, `approve_draft`, `discard_draft`, `request_revision`, `mirror_inbound`, `try_manual_reply`, `edit_and_approve`, `refresh_card`) now take an `unknown_inbox: Option<&str>` parameter — callers (mcp_server / ws_server / gateway / router / contacts/tools) read `cfg.contacts.unknown_inbox_channel` and pass it down.
25. **Disk-first reload before any whole-file rewrite** — WS handlers that re-serialise the entire `Config` (`config.set`, `mcp_env.set/remove`, `env.set/remove`, `social.mode`, `agents.new` / `.delete` / `.set_*`, `bindings.set` / `.delete`) MUST start with `Config::load(&gw.config_path)` instead of `gw.config.read().clone()`. Reason: even though all writes are supposed to go through the gateway now, a small bug or a disk edit by another process would leave the in-memory copy stale; cloning from memory and writing it back would silently delete entries that exist on disk. The `gw.config` `RwLock` is still updated after the disk write, so memory stays consistent. Pattern used everywhere: `let mut full = Config::load(&gw.config_path)?; mutate(&mut full); fs::write(...); *gw.config.write().unwrap() = full;`.
26. **`MessageRouter.bindings` is `RwLock<Vec<BindingEntry>>` — lock only inside `resolve_agent()`** — the read guard is acquired and dropped within a sync function, never held across `.await` (CLAUDE.md lesson #3). `pub fn set_bindings(&self, ...)` takes `&self` (not `&mut self`) so the WS handler can call it through `Arc<MessageRouter>` without needing an outer `RwLock<MessageRouter>`. **Don't** wrap the whole router in `RwLock` — only its mutable sub-state needs protecting, and putting `RwLock` higher up would make `route()` hold a guard across awaits.
27. **`contacts.forward_channel` is unique across all contacts** — enforced by partial unique index `idx_contacts_forward` on `contacts(forward_channel) WHERE forward_channel IS NOT NULL`. Reason: `pipeline::try_manual_reply` resolves `>>` admin replies by reverse-lookup on `forward_channel` (`find_contact_by_forward_channel` with LIMIT 1) — if two contacts shared a channel, the admin's manual reply would go to an arbitrary one. Existing DBs are migrated by `migrate_contacts_forward_unique` (state.rs): on startup it checks `sqlite_master` for the index's `UNIQUE` keyword, dedups by nulling all-but-newest duplicate rows (logging a warn for each), then drops + recreates the index as UNIQUE. **Pre-flight check** lives in `contacts/tools.rs::contacts_update` and `main.rs::ContactCommands::Update` — both query `find_contact_by_forward_channel` and return a friendly error before SQLite barks; `is_unique_violation` catches the rare race where two callers slip past pre-flight. SKILL_CATCLAW Contacts section instructs the agent to refuse the "share a channel" request and offer to create per-client subchannels via `discord_create_channel`.
28. **Discord DM enters contacts, Discord guild does not** — Discord adapter auto-registers DM senders (`is_dm==true`) as `role=unknown` contacts when `contacts.enabled=true`, mirroring the LINE adapter. Guild messages bypass the contacts table entirely: `router::route` gates the `get_contact_by_platform_user` call on `ctx.is_direct_message` (uniform across all platforms as of lesson #41 — was a Discord-only `platform != "discord" || is_direct_message` gate, which let Telegram/LINE group messages slip through). Rationale: guild channels are workspace chat (admin ↔ agent, often the operator themselves), while DMs are customer service. Without the asymmetry, a Discord admin who happens to be bound as a contact (or whose user_id collides via cross-platform binding) would have all their guild messages flagged for approval and routed through `submit_reply`, breaking the workspace flow. **`DiscordAdapter::set_contacts_context`** is the wiring point — gateway must call it before `start()` so the Handler captures `state_db + config Arc` for the auto-register path (`ensure_unknown_discord_contact` in `channel/discord.rs`). **Telegram now mirrors this**: `TelegramAdapter::set_contacts_context` injects the same `state_db + config Arc`, but because the teloxide dispatcher has no `Handler` struct (just a `move` endpoint closure), the `Option<ContactsContext>` is snapshotted via `self.contacts_state.read().await.clone()` in `start()` and captured directly into the closure (cloned per-invocation like `msg_tx`), gated on `is_private`. Slack still does NOT auto-register — needs manual `contacts_create + contacts_bind_channel`.
29. **Outbound to a contact uses `ChannelAdapter::send_to_user`, not `send`** — a bound `contact_channels.platform_user_id` is a *user id*. On LINE (and most platforms) a user id is a valid push target, so the trait's default `send_to_user` just forwards to `send` with the user id as `channel_id`. On Discord a user id is NOT a channel id — you must open a DM channel first (`UserId::create_dm_channel`, serenity-cached). `pipeline::send_to_contact` calls `adapter.send_to_user(platform_user_id, text)`; the Discord adapter overrides `send_to_user` to do the DM-channel dance. Symptom of getting this wrong: `discord error: failed to send message: Unknown Channel` (Discord error 10003) and the contact draft lands in `status=failed`. Failed drafts are NOT auto-retried — the user must re-trigger the reply or hit retry on the work card.
30. **Embedding is always in-process fastembed BGE-M3 — there is no provider config** — the old `[embedding] provider = "ollama"` keys were dead (read by nobody at runtime); removed. `gateway::start()` calls `Embedder::new()` unconditionally → loads BGE-M3 (~4 GiB RSS after warm-up, ~2.3 GB download on first run — see lesson #36 for why RSS is high). On a small VM this is the single biggest RAM consumer; combined with docker / CI runners on the same box it's how the gateway got dragged into 45-minute swap thrash (incident 2026-05-13). Mitigations in place: `Embedder` has a `Semaphore(1)` so concurrent `memory_write`s don't stack inference spikes; the systemd unit ships `MemoryHigh=5G MemoryMax=6G` (cgroup OOM-kills the biggest offender — usually a runaway `claude` subprocess — instead of thrashing) and `Type=notify` + `WatchdogSec=120` (`gateway::run` sends `READY=1` and a `WATCHDOG=1` ping every 45s via the hand-rolled `dist::sd_notify`; a frozen runtime → systemd restart). `catclaw update` rewrites the unit file (`service_install` is idempotent) so unit-level changes propagate. Memory accounting in *user* units needs cgroup delegation (systemd ≥ v244, default on modern distros) — silently ignored otherwise. `TimeoutStartSec=300` covers the first-run model download.
31. **Archived sessions are pruned after `general.session_retention_days` (default 30, 0 = never)** — without this, archived `sessions` rows + their transcript jsonl files accumulate forever, bloating `state.db` and the transcripts dir. The 6-hourly archive-cleanup pass in `scheduler.rs` calls `prune_old_sessions` → `StateDb::delete_old_archived_sessions` (deletes rows, returns `(agent_id, session_id)`), then removes matching `{agent_workspace}/transcripts/*{session_id}.jsonl` files, then `StateDb::reclaim_space` (`PRAGMA incremental_vacuum` if the DB was created with `auto_vacuum=INCREMENTAL`, else full `VACUUM`). New DBs get `auto_vacuum=INCREMENTAL`; existing DBs keep their mode and fall back to `VACUUM` (brief exclusive lock, fine for a 6-hourly job). **Not touched**: `memory_nodes` / `kg_*` / `vec_memories` — that's user data, separate concern. Note this cleanup is hygiene (disk doesn't grow unbounded), NOT the fix for thrash — thrash is the embedding RAM spike (lesson #30), not DB size.

32. **All model strings use canonical `provider/model` form** — `claude/opus-4-7`, `codex/gpt-5.5`, etc. Parsed by `agent::models::parse_model_string` which also accepts legacy un-prefixed aliases (`opus` → `claude/opus-4-7`) and bare full IDs (`claude-opus-4-7` → same). `Config::load` migrates old un-prefixed values to canonical form on first load (warn log + write-back). `agents.set_model` / `sessions.set_model` reject provider/runtime mismatches with a clear error message; `claude/*` requires `agent.runtime=claude`, `codex/*` requires `codex`. The args builder (`claude_args_with_mcp`, `codex_args_from`) calls `resolve_model` to strip the provider prefix back off — the CLI itself gets the bare ID. UI surfaces that surface model strings (TUI agents panel, `config get`, `agent list`) display the prefixed form.

33. **Background analysis ("diary") model is separate from agent models** — `general.diary_model` (default `claude/haiku-4-5`) drives `memory::oneshot::run_oneshot_inference` which `memory::analyze::call_haiku` + `scheduler::generate_diary` both call. Independent of any agent's runtime — set it to `codex/gpt-5.4-mini` to route catclaw's internal background analysis through OpenAI even when all your agents are Claude. Hot-reloads via `config.set diary_model X` → installs a new `ProviderModel` snapshot in `memory::oneshot::CURRENT_DIARY_MODEL` immediately. The snapshot pattern avoids threading a Config reference through every diary call site.

34. **Subscription / auth status has two layers** — file-presence (fast, free, no API): `claude auth status` (JSON) + `codex login status` (free-form text). Real failure marker (definitive, persisted): when a real model call's stderr matches the auth-failure heuristic (`401` / `403` / `unauthorized` / `invalid api key` / `not logged in`), `subscription::record_failure` writes `~/.catclaw/auth_status.json` and the TUI flips that provider to ⚠️. Next successful call clears the marker. The check is callable as `catclaw auth` (CLI), `auth.status` WS method, or `subscription::check_all` (internal). Codex's status line goes to STDERR (not stdout) when stdout isn't a TTY — `probe_codex` reads both pipes; missing this is the difference between "✓ logged in" and a misleading "? unknown" in the UI.

35. **Diary extraction must not full-scan transcripts on every tick** — historic 104 GiB disk-read spikes came from `read_since_last_marker` reading the entire JSONL on every 60-second scheduler tick, multiplied by N idle sessions and re-tried indefinitely on failure (no marker written → re-read next tick). Fix lives in `src/session/transcript.rs::MarkerState` (a `{path}.marker` JSON sidecar) + `src/scheduler.rs::DIARY_FAILURE_BACKOFF_SECS` + `src/scheduler.rs::RollingDiaryTrigger`. Three rules:
   - **Sidecar is the source of truth for "what's new"** — `byte_offset` lets `read_since_last_marker` seek directly to the tail. Missing/stale sidecar triggers one full scan + rebuild, never a hot-loop. Never write a code path that re-reads the entire transcript per scheduler tick.
   - **Every failure must advance the marker** — the `diary_failed:{rfc3339}` system entry is what stops the next tick from re-reading the same 5 MiB. The back-off table (5min/15min/1hr/6hr) is keyed on `fail_attempt`, which is incremented in `log_system` when the marker kind is `Failed` and reset to 0 on `Extracted`/`Skipped`. **Do not** suppress the marker on transient errors thinking "we'll retry soon" — without the marker, "retry soon" means "re-read the whole file every 60s".
   - **All diary code paths share one semaphore** — `scheduler::DiarySemaphore` (default capacity 1, configurable via `general.diary_max_concurrent`). Three callers (idle-scan, rolling per-N-turn, `/new`) all funnel through `extract_diary_for_session(.., throttle)` which acquires before reading the transcript. Without this, an idle-burst can fan out 100 simultaneous transcript reads + `claude -p` subprocesses and saturate disk/CPU/RAM to the point sshd can't get a tokio slice — incident 2026-05-19 was unrecoverable without a forced VM reboot. The semaphore is built once at gateway start and shared via `Arc`; resizing requires restart (warn the user when raising it).

   The rolling trigger lives in `SessionManager::notify_diary_trigger` (via `DiaryTrigger` trait — abstract to dodge the scheduler↔session-manager circular dep). Threshold is read live from `Config` so `config.set diary_turn_threshold N` takes effect on the next turn. Trigger respects `agent.memory_disabled()` and the same `diary_in_flight` set the scheduler uses, so the two paths can't double-fire on the same session.

36. **BGE-M3 must be loaded as owned bytes, not mmap — DO NOT revert to `try_new()`** — historic 100+ GiB disk-read spikes (incident 2026-05-19, separate from the transcript-rescan issue in lesson #35) came from kernel evicting mmap-backed model pages under anon-memory pressure and re-faulting them from disk on every inference. The model file (`model.onnx_data`, ~2.27 GiB) sits in **page cache** under `try_new`, which kernel can drop "for free" any time `claude` / `docker` / catclaw's own anon heap grows even slightly. Each `memory_write` then page-faults the entire weight blob back from disk; an idle-burst that triggers ~45 inferences will read ~100 GiB. The kicker: **RAM monitoring never shows pressure** (mmap pages aren't billed to RSS), only `read_bytes` in `/proc/<pid>/io` reveals it. The diagnostic fingerprint is `read_bytes` ≫ `rchar` (16× ratio in our case) — userspace never `read()`s but kernel does.

   Fix in `src/memory/embed.rs`: two-phase load — call `TextEmbedding::try_new` once to let fastembed handle hf-hub download + cache, immediately drop it (releases the mmap session), then `std::fs::read` the three ONNX files (`onnx/model.onnx` main graph + `onnx/model.onnx_data` external weights + `onnx/Constant_7_attr__value` aux constant) into `Vec<u8>` and rebuild via `TextEmbedding::try_new_from_user_defined` with all three registered via `UserDefinedEmbeddingModel::new(..).with_external_initializer(name, bytes)`. Under the hood this routes to ort's `CreateSessionFromArray` (impl_commit.rs:187) instead of `CreateSession` (impl_commit.rs:147) — owned heap allocations instead of file mappings. Pooling MUST be `Pooling::Cls` (BGE-M3 default in fastembed's `get_default_pooling_method`), `quantization` MUST be `None` (BGE-M3 is not in the Q-list in `get_quantization_mode`), `output_key` MUST be `None`. Mismatch breaks vector compatibility with the 1835 already-stored embeddings.

   **Tradeoffs to remember:**
   - RSS jumps from ~1.8 GB to ~4 GB. Monitors will look alarming but this is the model **finally being accounted for** instead of hiding in page cache. Lesson #30 + `dist.rs` systemd unit now reflect this with `MemoryHigh=5G MemoryMax=6G` (was 3G/4G — those values WILL cgroup-OOM the gateway on first inference if a future change resets them).
   - The fallback path (`try_new` retry on owned-load failure) is intentional — embedding being degraded is much better than memory palace being broken. If the fallback log warning appears in prod, **investigate** (cache layout changed, file missing) rather than ignoring it.
   - **Skip the three external files** at your peril: ONNX Runtime will fall back to mmap-loading them from the main graph's directory, defeating the entire fix silently. The `BGEM3_ONNX_EXTERNAL_FILES` const lists them — keep in sync with fastembed's `additional_files` for BGE-M3 (`src/models/text_embedding.rs`).
   - The fastembed cache structure (`models--BAAI--bge-m3/refs/main` → commit hash → `snapshots/<hash>/`) is a hf-hub convention. If a future fastembed version changes this layout, `locate_bgem3_snapshot` will fail, fallback engages, and the warning log fires.

37. **`catclaw update` returning `Ok(None)` skips `service_install` — any non-`catclaw update` deploy path leaves the systemd unit stale** — `dist::perform_update` early-returns when remote version ≤ current (`src/dist.rs:179`). The caller in `src/main.rs::Commands::Update` only calls `service_install` on the `Ok(Some(version))` branch, so when the binary was deployed by any other path (manual `scp`, CI pipeline, `cargo install --force`, release-please auto-deploy) the unit file on disk is whatever onboard wrote — never refreshed for new memory limits, watchdog timer, exec path, etc. Symptom seen 2026-05-19: model thrash fix shipped + verified via `read_bytes` collapse, but `systemctl --user cat catclaw` still showed `MemoryMax=infinity` (the pre-fix value) — only the binary upgraded, not the unit.

   Mitigations now in place:
   - `dist::service_install` is **byte-for-byte idempotent** — compares the rendered unit (`dist::build_systemd_unit`) against the file on disk via `dist::unit_sync_state`. `UnitSyncState::InSync` → skip the whole stop-service + uninstall + rewrite + daemon-reload + restart dance. Deploy scripts can therefore call `catclaw gateway install` unconditionally without disrupting the running service when nothing changed.
   - `gateway::start` calls `unit_sync_state` when `NOTIFY_SOCKET` is set (i.e. running under systemd `Type=notify`) and `warn!`s on `Drifted` with a one-line instruction to run `catclaw gateway install`. This means any drift surfaces in the journal on the very next gateway restart, not silently when production happens to need the limits.

   **Rules for adding/changing unit-file fields:**
   - Any deploy-time variable (binary path, memory limits, watchdog, env) MUST flow through `build_systemd_unit` — never patch after `service_install` returns. Out-of-band patches break drift detection (the function never produces them, so they look like drift every restart and trigger a reinstall loop).
   - When changing the rendered unit content, also bump anything that affects byte equality (whitespace, key ordering). The comparison is `String == String`, not semantic.
   - macOS plist drift detection is intentionally not implemented (`unit_sync_state` returns `InSync` unconditionally on non-Linux). XML whitespace noise would make it fragile and macOS isn't the production target. Revisit only when there's a real deploy story for mac.
   - **Don't** auto-reinstall from `gateway::start` — that would race with the systemd process group catclaw is currently a member of. Warn only; let the operator decide.

38. **Auto-registered contacts get a per-platform owning agent — resolve through `Config::default_agent_for_platform`, never `default_agent_id` directly** — each toC entry point can route its new `role=unknown` contacts to a different agent via `contacts.default_agent_{telegram,line,discord}` (e.g. a Telegram bot → agent `alice`, a LINE OA → agent `bob`). The three auto-register call sites (`channel/telegram.rs`, `channel/line.rs::line_webhook_handler`, `channel/discord.rs` message handler) all call `cfg.default_agent_for_platform(platform).unwrap_or("main")` instead of `default_agent_id()`. The helper returns the per-platform override **only when the named agent still exists**, otherwise falls back to the global default — so deleting/renaming an agent never strands new contacts on a dangling owner. `apply_config_set` validates the agent exists at set time (rejects unknown ids with a clear error) — this is the rare config key that does existence validation, because a typo'd owner is silently wrong rather than loudly broken. The keys are global (`ContactsConfig`), hot-reload (auto-register reads config live), `Option<String>` with `skip_serializing_if`. **When adding a new toC platform**: add a `default_agent_{platform}` field + a match arm in `default_agent_for_platform` + config_get/apply_config_set arms + the TUI entry + SKILL table + README — and have the adapter's auto-register path call `default_agent_for_platform`, not `default_agent_id`.

39. **Codex image generation is auto-on for every codex agent — via one `-c` flag + a SKILL note, NOT a per-agent config knob** — image generation (`gpt-image-2`) is a codex built-in: `codex_args::codex_args_from` unconditionally pushes `-c features.image_generation=true` for every codex spawn (mirroring how codex's bundled `imagegen` skill is always present). No `AgentConfig` field, no WS method, no TUI toggle — the user asked for it to "just work when you're on codex," like a tool/skill. Verified facts (live `codex exec --json`, codex-cli 0.130, ChatGPT login): (a) `image_gen` tool + `imagegen` skill (`$CODEX_HOME/skills/.system/imagegen/`) are available with **ChatGPT login alone — no `OPENAI_API_KEY`**; (b) inline `-c` overrides ARE honoured under `--ignore-user-config` (same as model/approval injection); (c) output lands in `$CODEX_HOME/generated_images/` = the per-agent `.codex-home/generated_images/`, and the imagegen skill always reports the final saved path + moves project-bound assets into the workspace (so the agent reliably has an absolute path); (d) `codex exec --json` has **no dedicated image event** — generation surfaces as `command_execution` + `agent_message`, both already parsed by `codex.rs`, so no parser change is needed. The catclaw-specific piece (codex doesn't know it) is the send-to-channel convention, added to `CODEX_RUNTIME_OVERRIDES` in `agent/mod.rs` (codex-only by construction — injected at `build_system_prompt` only when `runtime == Codex`): generate → `{platform}_upload_file(file_path=<abs>)` → end turn with `NO_REPLY`. **Don't hardcode the upload_file text/channel param names in the SKILL** — they differ per platform (`text`/`channel_id` Discord, `caption`/`chat_id` Telegram, `message`/`channel` Slack); tell the agent to read the tool's own schema. This is "route A" (agent uploads its own file, like the Claude image-send precedent); "route B" (catclaw auto-detects the file and attaches it) was explicitly deferred. Aside: codex's HTTP MCP servers in `~/.codex/config.toml` must use `url`-only (no `command`/`args`) — mixing the two triggers `url is not supported for stdio` and blocks codex from starting; this is the user's global config, isolated from catclaw's per-agent `.codex-home`.

40. **Inbound agent priority: explicit binding > contact.agent_id > global default — `router::route` reads `contact.agent_id` only when no binding matched** — the contact's owning agent (set at auto-register via `default_agent_for_platform`, or reassigned via `contacts_update`) must answer the customer in both directions. Outbound already routed through `contact.owning_agents()` (`contacts/pipeline.rs`), but inbound originally went through `resolve_agent` (binding table → global default) and **never read `contact.agent_id`** — so a Telegram client with `agent_id=domi` got *replied to* by domi but *received by* `main` (whichever the default was). Symptom: user sets `contacts.default_agent_telegram=domi`, confirms the contact's `agent_id` is domi, yet `main` keeps handling inbound. Fix in `router::route` (right after `resolve_agent`): `if !is_explicit_binding { if let Some(c) = contact { if !c.agent_id.is_empty() { agent_id = c.agent_id.clone(); } } }`. **A binding is the admin's deliberate routing rule and OUTRANKS the contact's owning agent** — `catclaw bind "telegram:*" main` deliberately pulls all Telegram (incl. known contacts) to main; `contact.agent_id` is the auto-filled default that fills in when no binding claims the message. `is_explicit_binding` is `resolve_agent`'s flag for "a real binding pattern matched" (vs the default fallback) — do NOT set it true on the contact override (that would also wrongly let a backend-channel contact bypass the `Backend && !is_explicit_binding` reject guard). Only client/admin contacts reach this point (unknown + ai_paused already `return`ed). The `registry.get(agent_id).or_else(default_agent())` fallback at the agent lookup covers a contact pointing at a deleted agent. Per-contact routing knob = `contact.agent_id` (now drives inbound when unbound); `telegram:* → agent` binding is the platform-wide override on top. (Closes the loop on lesson #38: `default_agent_telegram` writes the field at register time, but it was inert for routing until this fix read it back.)

41. **`current_exe()` is poisoned by self-replacement — `catclaw update` must never bake a bare `current_exe()` path into a service unit** — `catclaw update` replaces its own binary via `fs::rename` (dist.rs `perform_update`) and then, **in the same process**, calls `service_install` to refresh the systemd unit. On Linux `std::env::current_exe()` reads `/proc/self/exe`; once the original inode is unlinked by the rename, that link resolves to the real path **with a literal `" (deleted)"` suffix** (`/home/u/.local/bin/catclaw (deleted)`). That string flowed into `build_systemd_unit`'s `ExecStart`, so systemd's `execve` failed with **`status=203/EXEC`** and the unit thrashed in `activating (auto-restart)` forever. **Same root cause produced a phantom "unit drift detected"**: `unit_sync_state` also called `current_exe()`, computed an expected unit containing `(deleted)`, found it ≠ the clean on-disk unit, and triggered a needless reinstall — which then wrote the broken unit. Manually re-running `catclaw gateway install` "fixed" it only because that was a *fresh* process running the *already-landed new binary*, whose `/proc/self/exe` points at a live inode. **macOS never reproduces this** (`current_exe` has no `(deleted)` marker), so it's a Linux-prod-only landmine invisible on a Mac dev box (incident 2026-05-22, v0.51.1). Fix in dist.rs: `resolve_self_exe()` strips the `" (deleted)"` suffix then `canonicalize`s; `ensure_executable()` is the backstop that **refuses to write a unit** when the resolved path isn't a runnable file (turns "thrash forever" into a loud, actionable error telling the user to re-run `catclaw gateway install`). Both `service_install` and `unit_sync_state` now go through `resolve_self_exe`. **Rules:** (a) never feed bare `current_exe()` into anything *persisted to disk* (unit, plist, config, script) or *spawned* after a possible self-replacement — use `dist::resolve_self_exe()` (`pub(crate)`); `start_background_gateway{,_quiet}` in main.rs were also switched as defense-in-depth even though they're not on the update path today. (b) `perform_update`'s own `current_exe()` (the rename *target*, called *before* the rename) and `cmd_uninstall`'s (the *delete* target) intentionally stay bare — they want the current location, not the stripped one. (c) any new "write the exe path somewhere durable" code path must validate with `ensure_executable` before persisting.

42. **Only 1:1 DMs enter contacts — the adapter auto-register gate AND the router contact-lookup gate must BOTH be DM-only, or you get split-brain** — contacts is a toC (customer-service) layer; group/guild/room messages are workspace chat and must route normally. There are two independent gates and they have to agree: (a) each adapter's auto-register path (`telegram.rs` `is_private`, `discord.rs` `is_dm`, `line.rs` `source.type == "user"`) — decides whether a *new* contact is created; (b) `router::route`'s lookup `if ctx.is_direct_message { db.get_contact_by_platform_user(...) }` — decides whether an *existing* contact is consulted for an inbound message. **Bug shipped 2026-05-22 (v0.51.1):** the router gate was `platform != "discord" || ctx.is_direct_message` — Discord-only. When Telegram/LINE were added to contacts, their group messages still passed the router gate, so a `role=client` who was bound on DM and then `@mentioned` the bot in a group got their group message pulled into the contacts pipeline: a `sent` reply card rendered, and the reply went out via `contacts::pipeline` to their **DM** (`send_to_contact` uses the bound `platform_user_id`), never reaching the group. Symptom (user screenshot): "JY (via telegram) @bot hi" → reply card `role=client / via telegram / sent`, but the group saw nothing. Fix: router gate is now uniform `if ctx.is_direct_message` (all platforms), and LINE auto-register is gated to `source.type == "user"` (LINE groups/rooms no longer auto-register either) — the rule is now identical on both gates and all platforms. **When adding a platform to contacts, wire BOTH gates** — a DM-only auto-register with a platform-blind router lookup is the trap, because cross-channel/DM-bound contacts still get matched by `sender_id` in group context.

43. **`service_install` must wait for the old gateway to release the WS/MCP port before `systemctl start` — `stop` only signals, the port frees on exit** — distinct from lesson #41 (which fixed the `(deleted)` path *content*). Even with a clean unit, `catclaw update` failed at the `systemctl start` step: `update` → `service_uninstall` (`systemctl --user stop catclaw`, returns after SIGTERM) → fixed `300ms` sleep → `service_install_linux`'s `systemctl --user start catclaw`. The old gateway (PID still draining BGE-M3 teardown + active `claude -p` subprocesses) hadn't released port 21130 yet, so the new instance's bind failed → start returned non-zero. The user's later manual `catclaw gateway install` "fixed" it only because by then the old process had finished dying and freed the port (a *timing* coincidence, NOT a fresh-process effect — that's the lesson #41 story, easy to conflate). The `unit drift detected` reinstall amplified it by forcing the full stop→uninstall→reinstall→start cycle. Fix: `dist::wait_for_port_release(port)` polls `TcpListener::bind(("127.0.0.1", port))` (succeeds only once the holder exits) for up to ~6s, replacing the fixed sleep; called in `service_install` after the stop/uninstall block, before the platform install. Best-effort — warns + proceeds on timeout (systemd `Restart=on-failure` is the final backstop, which is also why the service was *eventually* up when we checked even though the update command reported failure). **`restart_service()` does NOT need this** — `systemctl --user restart` is internally sequenced (waits for stop before start, respecting `TimeoutStopSec`); the race only existed because `service_uninstall` + `service_install`'s `start` are two unsequenced commands. **Rule:** any "stop the gateway then start a new one" sequence that isn't a single `systemctl restart` must wait on the port, not a fixed sleep.

44. **Discord forum support is built on serenity's forum API + the existing message/activation paths — no new WS method, no duplicate wake-up.** A forum "post" is a thread whose parent channel kind == `Forum`; `discord_send_message`/`discord_get_messages` already work on it by passing the post id as channel_id. New pieces: (a) MCP actions `discord_create_forum_post` (Discord *requires* the initial message in the same request — `CreateForumPost::new(name, CreateMessage)` — which is why plain `create_thread` can't open a post), `list_forum_posts`/`forum_post_info`/`list_forum_tags`/`edit_forum_post`, plus `create_channel` gained `kind="forum"`+`available_tags` and `send_message` gained `reply_to_message_id`. (b) `EventHandler::thread_create` — fires on new posts; it does **NOT** emit an inbound wake-up (the post's first message is `MessageType::Regular`, id == thread id, and arrives as a normal MESSAGE_CREATE that wakes the agent through the existing `message` handler). `thread_create`'s only jobs are to gate on the forum channel's `activation` (`none` → stay silent) and `join_thread` so subsequent replies are delivered (Discord only forwards thread messages to members). Trying to wake from `thread_create` AND the first message = double-fire; don't. (c) the `message` handler enriches forum posts with `ForumPostContext{title,tags,is_new_post}` on `MsgContext` (resolving applied_tags against the parent forum's available_tags from cache), injected into the system prompt by `router.rs` like the `[Contact: ...]` block. **Gotchas:** `ForumTag` is `#[non_exhaustive]` — can't struct-literal it; build via `serde_json::from_value` with `id:"0"` (Discord assigns the real id). `is_new_post` = `msg.id == thread.id` (forum starter message shares the thread id; verified against Discord docs — it is type 0/Regular, NOT type 21/ThreadStarterMessage, so the existing `ThreadCreated|ThreadStarterMessage` filter does **not** drop it). All 5 sync points done (dispatch + `discord_action_infos` + `list_catclaw_mcp_tools` + SKILL_CATCLAW/SKILL_DISCORD + README); note `list_catclaw_mcp_tools` count comment is informational.

45. **`activation` accepts `none`, and per-scope overrides have a CLI now — the engine always supported both; the gaps were validation + tooling.** Channel handlers treat any `activation` value that isn't `all`/`mention` as "never reply" (`discord.rs` `_ => false`), so `none` always worked at runtime — but `apply_config_set` hard-rejected it with "must be 'mention' or 'all'" (now accepts `mention`/`all`/`none`, rejects only typos). Per-scope overrides (`channels[N].overrides`, resolved channel→guild→global by `AdapterFilter::activation_for_guild`) used to be hand-edit-toml-only; now `catclaw channel override set/list/delete <pattern> <activation>` exists. **Key design choice: override mutation is an `apply_config_set` key (`channels[N].override.<pattern>`, value = activation, empty/`-` deletes), NOT a new WS method** — because `handle_config_set` already disk-first-reloads + rebuilds every `AdapterFilter::from_config` (hot-reload) on any `config.set`. So CLI (`apply_config_set_cli` WS-first/file-fallback helper), TUI (config_panel lists existing overrides as editable `ConfigEntry`s), and WS all flow through the one existing path. TUI can edit/delete existing overrides but not add brand-new patterns (no free-form add flow) — CLI covers add. This is the fix for "agent thinks it can't set `none`": the SKILL taught `none` but the CLI errored on it, so the agent learned the wrong lesson.

46. **Switching an agent's runtime in place must REBUILD the registry Agent, not call `reload_agent_config` — that helper doesn't touch `runtime`.** `runtime` (which CLI to spawn: `claude -p` vs `codex exec`) is a per-agent `AgentConfig` field, settable only at `agent new` time until now. The new `agents.set_runtime` (CLI `catclaw agent set-runtime <name> <claude|codex>`, TUI `R` key, both WS-first/file-fallback) switches **in place** — no delete/recreate — so workspace, tools.toml, transcripts, memory, contacts, bindings all survive. **The trap:** `AgentRegistry::reload_agent_config` only syncs `approval`/`tools`/`model`/`fallback_model` (see `agent/mod.rs`), NOT `runtime` or `codex_auth_path`. So after writing the new `runtime` to disk, calling `reload_agent_config` would leave the in-memory `Agent.runtime` stale and the next spawn would still launch the old CLI. The handler instead does `registry.remove(id)` + `AgentLoader::load(&new_cfg, ..)` + `registry.add(agent)` (the same path `agents.new` uses), then restores `is_default` explicitly (because `add()` only re-flags default when none exists). Three more invariants the handler enforces: (a) **model reset** — a `claude/*` model can't run on codex and vice versa (the `provider==runtime` guard in `set_model`/`sessions.set_model` would reject it), so it resets `model` to `models::default_model_for_runtime(rt)` (`claude/opus-4-8` / `codex/gpt-5.5`) and clears `fallback_model`; (b) **codex preflight BEFORE persist** — `setup_codex_home` builds `.codex-home/` + verifies `auth.json` exists (fails loudly here, not at first message); switching to claude calls `cleanup_codex_home` + nulls `codex_auth_path`; (c) **archive old sessions** — each session binds its runtime in metadata at creation (`runtime_from_metadata`), so `SessionManager::archive_all_for_agent` stops in-flight subprocesses + archives all non-archived sessions, forcing the next message to open a fresh session under the new runtime. **Why `set-model` got a CLI too but no file fallback:** model changes need the gateway's provider/runtime validation + registry hot-reload; a direct file edit would skip validation and leave memory stale — so `cmd_agent_set_model` errors when the gateway is offline rather than writing the file (unlike `set-runtime`/`set-default`, which safely fall back because their file write is self-contained). **runtime ≠ model** is the user-facing confusion this closes: you can't make a claude agent into a codex agent by changing its model; the SKILL + README now teach the distinction explicitly.

47. **`KNOWN_MODELS` / TUI / CLI model lists are HINTS, not truth — the real codex model set depends on the user's ChatGPT plan, so never present the hardcoded list as authoritative, and verify codex model ids by probing, not by trusting the const.** `agent::models::parse_model_string` passes `codex/*` through unchanged (only `claude/*` aliases resolve against `KNOWN_MODELS`), so any codex id the user types reaches `codex exec -m <id>` verbatim — the entitlement check happens server-side at OpenAI. Verified 2026-06 on a standard ChatGPT-account Codex login (codex-cli 0.142): **available** = `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `o3`; **NOT available** = `gpt-5.5-mini` → `400 invalid_request_error: "The 'gpt-5.5-mini' model is not supported when using Codex with a ChatGPT account."` The original `KNOWN_MODELS` shipped `gpt-5.5-mini` as the codex "cheap tier" hint and omitted the `gpt-5.4*` line entirely — actively misleading (a user picking the suggested completion would get a hard 400). Fixed: hints are now `gpt-5.5 / gpt-5.4 / gpt-5.4-mini / o3` across `models.rs`, `tui/config_panel.rs` (default_model + diary_model completions), and `tui/sessions.rs` (session model picker). **`default_model_for_runtime(Codex)` stays `codex/gpt-5.5`** (the flagship, confirmed available — used as the reset target on `set-runtime`). **How to verify a codex model id** (don't guess, don't trust the const): `codex exec -m <id> --skip-git-repo-check "Reply with exactly: OK"` — a 400 "not supported" means the account lacks it; a normal reply means it's usable. macOS has no `timeout` (use `gtimeout` or rely on `codex exec` self-terminating). The `diary_model` / `heartbeat.model` cheap-tier docs now recommend `codex/gpt-5.4-mini`, not the nonexistent mini. **Rule:** any time you surface a codex model name (skill, README, completion, default), it's a suggestion gated by the user's plan — phrase it that way and tell the user they can type their own id.

48. **Codex's catclaw MCP calls fail with "unknown codex session" unless the lookup row is written the moment `thread.started` fires — and the fix must be on EVERY spawn path, not just `ephemeral_run`.** Codex generates its own session id (the `thread.started` `thread_id`) and starts issuing `mcp__catclaw__*` calls (memory_write, kg_*, contacts_*, discord_*) **mid-turn**, carrying that id in `_meta.x-codex-turn-metadata`. The MCP intercept (`mcp_server::resolve_agent_from_session`) resolves the caller by looking that id up in the `sessions` table. But the main session row stores catclaw's own placeholder UUID (needed for resume + transcript naming) and only gets updated to the codex id *after the whole turn finishes* (`manager.rs` `returned_session_id` flow) — so the first MCP call races ahead of the DB write and lands as `unknown codex session: <id>`, hard-erroring EVERY catclaw tool the codex agent calls. The fix (`write_codex_mcp_lookup_row` + `register_codex_session_pre_wait` in `manager.rs`): on the `SystemInit` event, upsert a SEPARATE `archived` + `origin=ephemeral` row keyed on the codex thread_id (`session_key=catclaw:{agent}:ephemeral:{thread_id}`) so MCP resolves it; the main row is untouched (archived+ephemeral keeps it out of TUI/resume lists). **The trap:** this fix already existed in `ephemeral_run` (background diary), so diary worked and it *looked* fine — but the three real conversation paths (`send_and_wait`, `send_streaming`, the BOOT.md branch) all lacked it, so every channel/WS/TUI codex turn silently lost memory palace + all catclaw MCP tools. `send_streaming`'s fix lives in its detached `tokio::spawn` SystemInit arm and can't borrow `&self` — that's why `write_codex_mcp_lookup_row` is a free fn taking `&StateDb` (the task captures a cloned `Arc<StateDb>`). Symptom fingerprint: codex agent can chat + use native `shell`, but `memory_nodes` stays empty and the gateway log shows `unknown codex session` (count it: `grep -c` should be 0 after the fix). **Rule:** any codex feature that depends on catclaw MCP must be exercised on the *streaming channel path*, not just a one-shot `ephemeral_run`, or the bug hides.

49. **Codex must gate catclaw MCP tools with `mcp_tool_permission`, NOT `tool_permission` — the `allowed` whitelist governs built-in tools only (parity with Claude's `--tools`).** Under Claude Code, `tools.toml::allowed` becomes the `--tools` flag, which whitelists **built-in** tools (Read/Bash/Edit/...) but **does not gate MCP tools** (CLAUDE.md MCP-permission constraints / lesson #25) — so an agent whose `allowed` lists only `Read,Bash,...` still has every `mcp__catclaw__*` tool available, and your prod `main` (claude) writes memory fine despite no MCP entry in `allowed`. Codex has no `--tools` flag; catclaw enforces the gate itself in `mcp_server.rs`. The original code routed codex's MCP calls through `Agent::tool_permission`, which applies the whitelist to ALL names — so the moment an agent had a non-empty `allowed` list (i.e. always), every catclaw MCP tool returned `tool denied by agent policy: mcp__catclaw__memory_write`. **Net effect: a claude→codex migration with an unchanged `tools.toml` silently lost memory/contacts/discord** — the agent could chat but couldn't touch any catclaw subsystem. Fix: `Agent::mcp_tool_permission` (agent/mod.rs) mirrors `tool_permission` minus the allowed-whitelist denial (MCP tools default `Allowed`; only explicit `denied`/`require_approval` change that); both codex call sites in `mcp_server.rs` (social-publish branch + general gate) now use it. **Rule:** the `allowed` whitelist is a *built-in-tool* concept on both runtimes — never apply it to `mcp__*` names, or codex diverges from claude. The two MCP bugs (#48 session, #49 permission) compounded: fixing #48 alone just moved the failure from "unknown session" to "denied by policy" — both are required for codex to use catclaw MCP at all. Verified 2026-06-30: after both fixes a fresh codex agent's `memory_write` persists to `memory_nodes` and `memory_search` recalls it, zero session/policy errors.

50. **`find_latest_draft_for_tool`'s "reuse a failed draft" rule must never apply to drafts without a precise alignment key — a weak `(platform, draft_type, status='failed')` match silently overwrites an unrelated old draft, corrupting an already-sent-out approval card.** `stage_draft_from_tool` (ws_server.rs, called from the approval hook) auto-stages a `social_drafts` row every time a social publish tool fires, and reuses an existing non-terminal row instead of inserting a new one "so retries don't leave zombie rows behind." For `reply`/`dm` drafts this is safe — `reply_to_id` (the comment id / recipient id) is a precise key, so reuse only ever matches the *same target*. For `post`/`carousel` drafts (`instagram_create_post`, `threads_create_post`) there is **no alignment key** — the old query matched on just `platform + draft_type + status='failed'`, i.e. "any post ever staged for this platform that's currently failed, regardless of which task or which day produced it." Symptom (reported 2026-07-08): admin approves yesterday's IG post draft after it failed and ignores it; today's scheduled post creates a *new* `instagram_create_post` call; `find_latest_draft_for_tool` matches yesterday's still-`failed` row purely because it's the only failed post row, and `update_social_draft_content` overwrites its `content`/`media_url` in place — same DB row id, same Discord approval card `forward_ref` binding. The stale "發送失敗，點擊重試" card sitting in Discord never gets touched (nothing re-renders it), so it still visually looks like yesterday's post, but its `social_draft:approve:{id}` button now points at a row holding *today's* content. Admin clicks retry-approve on what they believe is yesterday's post → today's content ships instead. **Fix:** `find_latest_draft_for_tool` now returns `None` immediately when `reply_to_id` is `None` — post/carousel drafts never reuse, `stage_draft_from_tool` always falls through to `insert_social_draft` for them, so every task's draft gets its own row + its own Discord card + its own button binding, even if an older draft for the same platform is sitting in `failed`. **Rule:** before adding a "find and reuse the latest matching row" shortcut anywhere in the codebase, ask "what happens when two *semantically unrelated* calls produce the same match key?" — if the answer is "they'd get silently merged," the key isn't precise enough to reuse on; require an explicit identifier (task id, reply-to id, idempotency key) or don't reuse at all. This is the same failure shape as lesson #43/#27 (a too-loose match key causing cross-item interference) applied to a new subsystem.
