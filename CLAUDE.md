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
| `agents.reload_tools` | Hot-reload agent config (tools, approval, model) | YES |
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

1. **Writing files from TUI does not mean the gateway picks it up** — any change affecting gateway in-memory state (agent approval, tool permissions) must notify the gateway via WS method for hot-reload.
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
18. **`contacts_reply` is the ONLY agent outbound path to a contact** — agents must not call `discord_send_message` / `telegram_send_message` / etc. directly for messages destined for a contact, because direct sends bypass the forward + approval pipeline. The agent's MCP toolbox includes both, but the catclaw skill (SKILL_CATCLAW) instructs agents to use `contacts_reply` for any contact-destined output.
19. **LINE adapter stores reply tokens per LINE userId; check expiry before reuse** — reply tokens are valid 5 minutes from inbound event. `LineAdapter.reply_tokens` is `RwLock<HashMap<userId, (token, expires_unix)>>`. `take_reply_token()` consumes the token (one-shot) and returns None when expired. Outbound `send()` always tries reply token first, then push API.
20. **LINE adapter is registered TWICE in gateway**: once into `adapters` (the generic `Arc<dyn ChannelAdapter>` map for router/MCP dispatch) AND once into `GatewayHandle.line_adapter` as concrete `Arc<LineAdapter>` (so the axum webhook handler can call `verify_signature` + `handle_webhook_payload` without trait downcast). Same pattern as `backend_adapter`. Don't try to fold these into one — Rust doesn't support trait-object downcast cleanly.
21. **`ChannelType::Line` enum variant must include in every match** — adding a new ChannelType variant requires updating `as_str()` in `src/channel/mod.rs`. Compiler catches missing arms; just don't fall back to a wildcard `_ =>` in places that need explicit per-platform routing.
