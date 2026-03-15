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

### Hot-Reload 規範
任何 agent 設定變更（tools、approval、model）不論來自 TUI 或 CLI，都必須：
1. 寫入檔案（`tools.toml` + `catclaw.toml`）
2. 呼叫 WS `agents.reload_tools` 通知 gateway 更新記憶體中的 `AgentRegistry`

`agents.reload_tools` handler 會從磁碟重新讀取 config 並同步到記憶體，涵蓋 approval、tools、model、fallback_model。

**全域設定**（`config set` 系列）走 WS `config.set`，gateway 自動 hot-reload（adapter filters、log level 等）。`apply_config_set` 返回 `Ok(false)` 表示即時生效，`Ok(true)` 表示需要重啟。

**Bindings** 目前存在記憶體中的 `MessageRouter`，修改後需要重啟 gateway 才生效（CLI 和 TUI 行為一致）。

**設計原則**：CLI 和 TUI 修改同一項設定時，必須走完全相同的 hot-reload 路徑。不能一邊有通知 gateway 而另一邊沒有。

### Claude Code CLI Flags
- `--dangerously-skip-permissions` does NOT skip hooks. Hooks (PreToolUse) still fire.
- `--session-id` creates new session, `--resume` resumes existing one.
- `--include-partial-messages` needed for `stream_event` type partial events.
- `--settings` injects hook config as JSON. Only injected when `approval.is_empty() == false`.
- `--tools` is the whitelist (only these tools available). `--disallowedTools` is the blacklist.
- `--allowedTools` only controls permission prompts, NOT tool availability.

### CLI 與 TUI 功能對等
CatClaw 的 CLI（`catclaw` 命令）和 TUI（終端介面）必須能做到**完全相同的操作**。新增任何功能時，必須同時實作：
- **CLI**（`src/main.rs` 的 subcommand + handler）— agent 透過 Bash tool 代替使用者執行
- **TUI**（`src/tui/` 對應 panel）— 使用者直接在終端操作
- **catclaw skill**（`src/agent/loader.rs` 的 `SKILL_CATCLAW`）— 更新 CLI 用法讓 agent 知道怎麼操作
- **README.md** — 保持文件與實際功能同步

三者缺一不可。CLI 是 agent 的手，TUI 是使用者的手，skill 是 agent 的腦。

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
| `src/cmd_hook.rs` | PreToolUse hook binary logic |
| `src/tui/agents.rs` | TUI Agents panel: tools 3-state toggle (allowed/approval/denied) |
| `src/tui/config_panel.rs` | TUI Config panel: editable settings including `approval.timeout_secs` |
| `src/scheduler.rs` | Heartbeat, cron, archive cleanup |

## 語言慣例

- **程式碼**：英文（變數名、函式名、struct 名、log message、程式碼註解）
- **溝通與文件**：中文（與使用者對話、commit 描述、task 名稱、CLAUDE.md 中的說明）
- **Skill 內容**：英文（agent 操作手冊，因為 Claude Code 主要是英文語境）
- **README.md**：英文

## 新增 Config Key Checklist

加一個新的可設定項需要改以下所有地方：

1. `src/config.rs` — struct 欄位 + `config_get()` + `apply_config_set()` + serde attributes
2. `src/tui/config_panel.rs` — `build_entries()` 加 `ConfigEntry` + `completions_for_key()` 如有選項
3. `src/agent/loader.rs` — `SKILL_CATCLAW` 常量中的 config key 表格
4. `README.md` — Configuration 段落

如果是 per-agent 設定（不是全域）：
1. `src/config.rs` — `AgentConfig` 或子 struct
2. `src/tui/agents.rs` — 對應的 UI 操作
3. `src/main.rs` — CLI subcommand flag
4. `src/ws_server.rs` — `handle_agents_reload_tools` 確保 hot-reload 涵蓋新欄位
5. `src/agent/mod.rs` — `Agent` struct + `reload_agent_config()` + `claude_args_with_mcp()` 如影響啟動參數

## Embedded Skill 更新流程

Skills 是 `src/agent/loader.rs` 中的 `const` 字串常量，編譯進 binary。安裝到使用者 workspace 的時機：
- `catclaw agent new` — 建立新 agent 時自動安裝所有 built-in skills
- `catclaw init` — 初始化時安裝

**更新 skill 內容後**：`cargo build --release` 產生新 binary，但已安裝的 workspace 檔案不會自動更新。需要手動覆蓋或刪除 `workspace/skills/{name}/SKILL.md` 讓下次 `agent new` 重建。

## WS Protocol Methods

Gateway WS server（`/ws`）支援的 JSON-RPC methods：

| Method | 用途 | Hot-reload |
|--------|------|-----------|
| `gateway.status` | 查詢 agent 數量、active sessions | — |
| `sessions.list` / `.delete` / `.stop` | Session CRUD | — |
| `sessions.send` | 發送訊息到 session（streaming/non-streaming）| — |
| `sessions.transcript` | 讀取 session transcript | — |
| `sessions.set_model` | 設定 session 的 model override | — |
| `agents.list` / `.get` / `.default` | Agent 查詢 | — |
| `agents.reload_tools` | Hot-reload agent config（tools、approval、model）| YES |
| `tasks.list` / `.enable` / `.disable` / `.delete` | Scheduled task CRUD | — |
| `config.get` / `.set` | 全域設定讀寫 | YES（部分需重啟）|
| `approval.request` / `.respond` / `.list` | Tool approval 流程 | — |

新增 WS method 時需更新此表和 `src/ws_server.rs` 的 `dispatch()` 函式。

## Build & Test

```bash
cargo check          # Fast type-check
cargo build --release  # Production build (output: target/release/catclaw)
cargo clippy         # Lint
```

Always run `cargo check` after changes — zero errors AND zero warnings required.

目前沒有 unit test — 驗證靠 `cargo check`（零錯誤零警告）+ 手動 TUI/CLI 測試。

## Dependencies (version constraints)

- `tui-textarea 0.7` requires `ratatui 0.29` + `crossterm 0.28` (not 0.30/0.29)
- `serenity 0.12` + `poise 0.6`
- `tokio-tungstenite 0.24` for WS

## Lessons Learned

1. **TUI 直接寫檔不等於 gateway 生效** — 任何影響 gateway 記憶體狀態的改動（agent approval、tool permissions）必須透過 WS method 通知 gateway 做 hot-reload。
2. **`#[derive(Default)]` 對含預設值的 config struct 是陷阱** — `u64` 的 Default 是 0，不是你想要的 120。手動 impl Default。
3. **`std::sync::RwLockReadGuard` 不是 `Send`** — 不能跨 `.await` 持有。提取到 local variable 再 await。
4. **approval hook 只在 `!approval.is_empty()` 時注入** — 如果 config 裡 `require_approval` 為空，`--settings` 不會加到 claude args，hook 不會觸發。
5. **Config panel 和 Agents panel 的設定分工** — 全域設定（timeout_secs）放 Config panel，per-agent 設定（哪些 tool 需要 approval）放 Agents > Tools。
6. **功能更新必須同步更新 skill** — catclaw skill（`src/agent/loader.rs` 中的 `SKILL_CATCLAW` 常量）是 agent 的操作手冊。任何新增功能（CLI flag、config key、TUI 操作）都必須反映在 skill 內容中，否則 agent 不知道怎麼教使用者操作。同理 README.md 也要保持同步。
7. **`Arc<AgentRegistry>` 改為 `Arc<RwLock<AgentRegistry>>`** — 為了支持 hot-reload，registry 需要可變。讀取時用 `.read().unwrap()`，寫入時用 `.write().unwrap()`。所有涉及 `.get()` 的地方需要 `.cloned()` 取得 owned Agent 避免 guard 跨 await。
8. **Hook subprocess 不能建立新 tokio runtime** — `catclaw hook pre-tool` 作為子進程執行時，`main` 已經是 `#[tokio::main]`（有 runtime）。在 `cmd_hook.rs` 中不能用 `tokio::runtime::Builder` 建立第二個 runtime，否則 panic。改用 `async fn` + `.await`。
