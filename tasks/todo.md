# CatClaw Implementation Progress

## Phase 2: Gateway WebSocket 架構重構 — COMPLETE
- [x] 1. `Cargo.toml` — 加 `tokio-tungstenite = "0.24"`
- [x] 2. `src/config.rs` — GeneralConfig 加 `ws_port: u16` (預設 21130)
- [x] 3. `src/ws_protocol.rs` — WsRequest, WsResponse, WsEvent 型別
- [x] 4. `src/ws_server.rs` — TCP accept loop + per-connection handler + method dispatch
- [x] 5. `src/gateway.rs` — GatewayHandle derive Clone + 整合 WS server
- [x] 6. `src/ws_client.rs` — GatewayClient::connect + request/response + event channel
- [x] 7. `src/tui/sessions.rs` — 改用 GatewayClient (WS) 取代 Arc<StateDb>/SessionManager
- [x] 8. `src/tui/tasks.rs` — 改用 GatewayClient (WS) 取代 Arc<StateDb>
- [x] 9. `src/tui/mod.rs` — 接收 ws_url 而非 &GatewayHandle
- [x] 10. `src/main.rs` — Gateway daemon flag, background spawn, TUI WS 連接


## Phase 1a: Core Skeleton — COMPLETE
- [x] 1. `cargo init` + Cargo.toml dependencies
- [x] 2. `src/error.rs` — CatClawError
- [x] 3. `src/config.rs` — catclaw.toml read/write
- [x] 4. `src/state.rs` — State DB (SQLite WAL, migrate, CRUD)
- [x] 5. `src/agent/loader.rs` — Load agent workspace MD + tools.toml
- [x] 6. `src/agent/mod.rs` — AgentRegistry + agent CRUD
- [x] 7. `src/session/mod.rs` — Type definitions (SessionKey, SessionState, Priority)
- [x] 8. `src/session/claude.rs` — ClaudeHandle (spawn/resume, NDJSON, multi --plugin-dir)
- [x] 9. `src/session/queue.rs` — Semaphore + PriorityQueue
- [x] 10. `src/session/manager.rs` — SessionManager + persistence
- [x] 11. `src/main.rs` — clap CLI all subcommands
- [x] 12. `catclaw init` — Interactive init (workspace, main agent, state.sqlite, catclaw.toml)

## Phase 1b: Channel Adapter + Gateway — COMPLETE
- [x] 13. `src/channel/mod.rs` — ChannelAdapter trait + MsgContext + OutboundMessage
- [x] 14. `src/channel/discord.rs` — Discord adapter (serenity, typing, thread, chunked send)
- [x] 15. `src/router.rs` — Message Router (MsgContext → agent → session → response → adapter)
- [x] 16. `src/gateway.rs` — Main loop + restart recovery + cleanup
- [x] 17. catclaw-plugin/ skeleton
- [x] 18. workspace/shared/ shared skills dirs

## Phase 1c: TUI — COMPLETE
- [x] 19. `src/tui/mod.rs` — App + Component trait + main loop + tab navigation + captures_input 機制
- [x] 20. `src/tui/theme.rs` — Catppuccin Mocha colors
- [x] 21. `src/tui/sessions.rs` — Session list (status icons + agent + channel + time) + 預設 main session 不可刪除
- [x] 22. `src/tui/chat.rs` — Chat widget (message bubbles + rendering)
- [x] 23. `src/tui/agents.rs` — Agent management (list + detail + tool toggle + e Edit SOUL.md + d Delete + n New 提示)
- [x] 24. `src/tui/editor.rs` — Full-screen MD editor (tui-textarea, Ctrl+S/Ctrl+Q)
- [x] 25. `src/tui/bindings.rs` — Binding table + CRUD + autocomplete + captures_input
- [x] 26. `src/tui/config_panel.rs` — Config panel (inline edit + r reload + 自動存檔 catclaw.toml)
- [x] 27. `src/tui/logs.rs` — 讀取 gateway.log 真實日誌 + level filter + r refresh + 2s auto-refresh
- [x] 27b. 統一啟動流程 — `catclaw` 無子命令 = splash → auto-init → 背景 gateway → TUI
- [x] 27c. `catclaw stop` — PID file 機制停止背景 gateway
- [x] 27d. Init 完成自動建立預設 main session

## Phase 1d: Logging System — COMPLETE
- [x] 41. JSON 結構化日誌寫檔 — `src/logging.rs` DailyFileWriter 寫入 workspace/logs/catclaw-YYYY-MM-DD.jsonl（按日期自動輪替），每行一個 JSON object（timestamp, level, target, message, fields）
- [x] 42. Log levels 分級記錄 — catclaw.toml `[logging]` section 設定 level + log_dir，支援 RUST_LOG 環境變數覆蓋
- [x] 43. `catclaw logs` CLI — `--follow` 即時串流 + `--level` 篩選 + `--grep` regex 搜尋 + `--since`/`--until` 時間範圍 + `--json` 原始 JSON 輸出 + `-n` 限制條數 + TTY 自動彩色
- [x] 44. TUI Logs 面板增強 — 讀取 JSONL 日誌檔 + `/` 搜尋框 + 搜尋結果黃底高亮 + 1-4 level 篩選 + g/G 跳頂/底 + c 清除搜尋 + subsystem target 顯示 + structured fields 顯示
- [x] 45. Gateway 背景模式日誌 — dual output（JSON file + console），背景模式 stdout 仍 redirect 到 gateway.log 作為 fallback，結構化日誌獨立寫入 JSONL

## Phase 1e: Channel Adapters + Built-in MCP Server — COMPLETE
- [x] 46. `src/channel/mod.rs` — ChannelAdapter trait 增強：`execute()`, `supported_actions()`, `ActionInfo`, `guild_id` 新增至 MsgContext
- [x] 47. `src/channel/discord.rs` — Discord adapter 32 個 MCP actions（messages, reactions, pins, threads, channels, permissions, guilds, members, roles, emojis, moderation, events, stickers）+ `supported_actions()` + `guild_id` 填入 MsgContext
- [x] 48. `src/channel/telegram.rs` — Telegram adapter（teloxide long polling）+ 26 個 MCP actions（messages, pins, chat info, management, moderation, polls, forum topics, permissions, invite links）
- [x] 49. `src/mcp_server.rs` — Built-in MCP HTTP server（axum），MCP JSON-RPC protocol（initialize, tools/list, tools/call, ping）
- [x] 50. `src/agent/mod.rs` — `claude_args_with_mcp()` + `claude_resume_args_with_mcp()` 注入 `--mcp-config` 指向內建 MCP server
- [x] 51. `src/session/manager.rs` — `with_mcp_port()` builder + 所有 claude spawn path 都注入 MCP config
- [x] 52. `src/agent/loader.rs` — SKILL_DISCORD 和 SKILL_TELEGRAM 擴充 Platform Operations tool 使用指引
- [x] 53. `src/error.rs` — Telegram error variant
- [x] 54. WS + MCP 合併單一端口 — `ws_server.rs` 改用 axum WebSocket（`/ws`），`mcp_server.rs` 改為 `router()` merge 到同一 axum app（`/mcp`），移除 `mcp_port` config（共用 `ws_port` 21130）
- [x] 55. Init 流程加入 Telegram 設定（BotFather 引導、token、activation）
- [x] 56. TUI Config 面板：channel activation 和 guilds 可編輯
- [x] 57. Init activation 提示改善：明確標示「DM 永遠回覆，此設定只影響群組」

## Phase 2: Memory (TODO)
- [ ] 28. Memory tables in state.sqlite
- [ ] 29. Embedding engine (Ollama)
- [ ] 30. MD → chunk → FTS + vectors
- [ ] 31. Hybrid search
- [ ] 32. File watcher
- [ ] 33. Plugin hooks (PreCompact/Stop → auto memory)
- [ ] 34. MCP server (memory + channel tools)

## Phase 3: Autonomy (partial)
- [x] 35. `src/scheduler.rs` — Scheduler loop (60s tick) + heartbeat + cron + one-shot + archive cleanup + CLI `catclaw task` commands
- [x] 35b. Skills system — built-in skills (sessions-history, skills-creator), per-agent install, enable/disable, CLI `catclaw skill`, TUI Skills panel
- [x] 35c. TUI Tasks panel — view/toggle/delete scheduled tasks
- [ ] 36. BOOT.md startup flow
- [ ] 37. Session lifecycle — archive after 7 days idle + pre-archive summary to memory/
- [x] 38. Session transcript logging — JSONL per session in agent workspace/transcripts/
- [x] 39. Archive with summary — archive_with_summary() generates summary via claude, saves to memory/YYYY-MM-DD.md
- [x] 39b. find_stale_sessions() — find sessions idle longer than threshold (for scheduler to call)
- [ ] 40. `sessions_history` tool — MCP/skill to let agent query other sessions' transcript files

## Phase 4: Collaboration & Extensions (TODO)
- [x] 38. Telegram adapter — 完整實作（long polling + 26 MCP actions）
- [ ] 39. Slack adapter
- [ ] 40. Agent collaboration (sessions_spawn, agent_send)
- [ ] 41. `/bind` Discord command
- [ ] 42. OpenClaw migration tool

## TODO: MCP Action 權限系統
目前所有 adapter 的 MCP actions 對所有 agent 全開。未來需要：
- [ ] Per-agent MCP action whitelist/blacklist（類似 tools.toml 的 allowed/denied）
- [ ] 危險操作分級（讀取 vs 寫入 vs 管理）：
  - **read**: get_messages, get_channels, get_guilds, get_chat, member_info, list_roles 等
  - **write**: send_message, create_channel, create_thread, pin_message, send_poll 等
  - **admin**: delete_channel, kick_member, ban_member, edit_permissions, timeout_member 等
- [ ] 預設策略：read 全開，write 需明確允許，admin 需明確允許
- [ ] MCP server 層面的權限檢查（需要知道當前 session 的 agent_id → 查 tools.toml → 過濾 tools/list 和 tools/call）
- [ ] TUI Agents panel 加入 MCP action 權限編輯

## Known Bugs / UX Issues
- [x] Init channel 選擇：改為逐一 Confirm 詢問，預設 yes，不再用 MultiSelect
- [x] Init 每步操作提示更明確（y/n 確認）
- [x] skill-creator 遠端下載：修正 jq filter 排除目錄（type==blob），改善 gh 未安裝的錯誤訊息
- [x] TUI Sessions 面板：移除 Enter(Chat) 假提示，改為實際可用提示
- [x] TUI Agents 面板：實作 e(Edit SOUL.md)、d(Delete)、n(提示用 CLI)
- [x] TUI Config 面板：實作 Enter(Edit value)、r(Reload config)
- [x] TUI Logs 面板：改為讀取 gateway.log 真實日誌，支援 r 刷新
- [x] TUI 全域 q 退出：在面板輸入模式時抑制全域快捷鍵（防止誤退出）
- [x] TUI Bindings/Skills 面板：加入 captures_input 防止輸入模式被全域鍵攔截
- [x] Init 完成自動建立預設 main session（不可刪除）
- [x] Init 移除 Step 4 並行數量設定（改為系統預設值 3）
- [x] Init 精緻化 CLI 介面：box-drawing 邊框、進度點、彩色 section、summary card（cli_ui.rs）
- [x] Discord 設定流程：7 步驟圖文指引（建立 Application → Bot Token → Intents → OAuth2 → 邀請）
- [x] `catclaw logs` CLI 命令（Phase 1d）
- [x] `send_and_wait` / `send_streaming` 新 session else 分支漏傳 MCP config — 已修復

## Build Status
- Compiles cleanly (0 errors, 0 warnings)
- `catclaw init` tested and working
- `catclaw agent new/list/delete` tested and working
- `catclaw config show` tested and working
- `catclaw session list` tested and working
- `catclaw stop` implemented
- TUI 所有面板按鍵 100% 對應實際功能（無假提示）
