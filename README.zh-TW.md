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
  <strong>由 Claude Code 驅動的個人 AI 閘道器</strong><br>
  <em>多代理 &bull; 多頻道 &bull; 全天候運行</em>
</p>

---

[English](README.md) | **繁體中文**

CatClaw 是一個 Rust 常駐程式，將你的 **Claude Code 訂閱**轉化為可從 Discord、Telegram 及終端 TUI 存取的個人 AI 助理。靈感來自 [OpenClaw](https://github.com/nicekid1/OpenClaw)，以 Rust 從零打造，追求效能、穩定性與 Anthropic 合規。

## 為什麼選 CatClaw？

- **使用你的 Claude Code 訂閱** — 不需要 API key、不會有意外帳單。CatClaw 產生 `claude -p` 子程序，直接使用你現有的 Claude Code 方案。
- **多代理（Multi-agent）** — 定義多個 AI 角色（主助理、研究專家、程式碼審查員），各自擁有獨立的人格、記憶和工具權限。
- **多頻道（Multi-channel）** — 從 Discord、Telegram 或內建 TUI 與你的代理對話，所有頻道共用同一套會話和記憶系統。
- **工具核准系統** — 要求使用者在代理執行敏感工具（Bash、Edit 等）前進行確認，TUI、Discord 按鈕和 Telegram 鍵盤都有內建的核准 UI。
- **無狀態閘道器** — 所有狀態持久化至 SQLite。隨時終止常駐程式、重啟，一切從中斷處繼續。
- **精美 TUI** — Catppuccin Mocha 主題的終端介面，8 個面板管理一切。

## 快速開始

### 前置需求

- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) 已安裝並登入

### 安裝

```bash
curl -fsSL https://raw.githubusercontent.com/CatiesGames/catclaw/main/install.sh | sh
```

或從原始碼編譯：
```bash
git clone https://github.com/CatiesGames/catclaw.git
cd catclaw
cargo build --release
```

### 啟動

```bash
catclaw onboard
```

首次執行時，CatClaw 會：
1. 顯示啟動動畫
2. 執行互動式設定精靈（驗證 Claude Code CLI、建立代理、設定頻道）
3. 選擇性安裝為系統服務（開機自啟）
4. 在背景啟動閘道器
5. 啟動 TUI

```bash
# 其他執行方式：
catclaw onboard                   # 引導設定 → 啟動閘道器 → 啟動 TUI
catclaw gateway start             # 前景啟動閘道器
catclaw gateway start -d          # 背景常駐模式
catclaw gateway stop              # 停止背景閘道器
catclaw gateway status            # 查看閘道器狀態
catclaw tui                       # 僅啟動 TUI（連線到運行中的閘道器）

# 更新與開機自啟：
catclaw update                    # 自我更新至最新版本
catclaw update --check            # 僅檢查更新
catclaw gateway install           # 安裝為系統服務（開機自啟）
catclaw gateway uninstall         # 移除系統服務
catclaw uninstall                 # 完整移除（停止、移除服務、刪除 binary）
```

## 架構

```
┌──────────────────────────────────────────────────────────────────┐
│                       CatClaw 閘道器 (Rust)                      │
│                                                                  │
│  ┌─────────────┐ ┌─────────────┐                                │
│  │  Discord     │ │  Telegram   │    頻道適配器                    │
│  │  Adapter     │ │  Adapter    │                                │
│  └──────┬───────┘ └──────┬──────┘                                │
│         └────────────────┘                                        │
│                  ▼                                                │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  訊息路由器  →  代理註冊表  →  會話管理器                      │  │
│  │  (綁定表)       (SOUL/工具)    (claude -p 子程序)              │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────────┐  ┌──────────────────┐                     │
│  │  狀態 DB          │  │  排程器           │                     │
│  │  (SQLite WAL)     │  │  (cron/heartbeat)│                     │
│  └──────────────────┘  └──────────────────┘                     │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  WS 伺服器 (/ws)  +  MCP 伺服器 (/mcp)  — 埠 21130      │   │
│  └──────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┘
          ▲                              ▲
          │ WebSocket                    │ MCP JSON-RPC
    ┌─────┴─────┐                 ┌──────┴──────┐
    │  TUI      │                 │  Claude CLI │
    │ (ratatui) │                 │  (工具呼叫)  │
    └───────────┘                 └─────────────┘
```

## 代理系統

每個代理擁有獨立的工作區，包含人格、記憶、技能和工具權限：

```
workspace/agents/main/
├── SOUL.md              # 人格、語氣、價值觀
├── USER.md              # 使用者資訊
├── IDENTITY.md          # 代理名稱、角色
├── AGENTS.md            # 工作區規範
├── TOOLS.md             # 工具使用指南
├── BOOT.md              # 啟動指令（在新會話的第一則訊息前注入）
├── HEARTBEAT.md         # 定期檢查任務
├── MEMORY.md            # 長期記憶（策展版）
├── memory/              # 每日筆記 (YYYY-MM-DD.md)
├── transcripts/         # 會話日誌 (JSONL)
└── tools.toml           # 工具權限
```

### 工具權限（三態）

每個工具只存在於三種狀態之一：

```toml
# workspace/agents/research/tools.toml
allowed = ["Read", "Grep", "Glob", "WebFetch", "WebSearch"]
denied = ["Bash"]
require_approval = ["Edit", "Write"]
```

- **allowed** — 工具可直接執行
- **denied** — 工具完全封鎖
- **require_approval** — 工具需使用者核准後才能執行（TUI 內嵌小工具、Discord 按鈕或 Telegram 鍵盤）

### 內建技能

| 技能 | 說明 |
|------|------|
| `catclaw` | CatClaw 系統管理（代理知道所有 CLI 指令） |
| `discord` | Discord 格式化與 MCP 工具使用 |
| `telegram` | Telegram 格式化與 MCP 工具使用 |
| `sessions-history` | 查詢其他會話的逐字記錄 |
| `injection-guard` | 防禦外部內容的提示注入攻擊 |

## 頻道適配器

| 頻道 | 狀態 | 功能 |
|------|------|------|
| **Discord** | ✅ | 討論串、打字指示器、核准按鈕、32 個 MCP 工具 |
| **Telegram** | ✅ | 長輪詢、論壇主題、核准鍵盤、26 個 MCP 工具 |
| **Slack** | 規劃中 | — |
| **TUI** | ✅ | 串流對話、內嵌核准小工具 |

### 內建 MCP 伺服器

CatClaw 將頻道適配器的操作暴露為 MCP 工具，代理可以自主執行平台操作：

```
代理想要列出 Discord 頻道
  → Claude 呼叫 mcp__catclaw__discord_get_channels
  → CatClaw MCP 伺服器 → serenity → Discord REST API
  → JSON 結果回傳給代理
```

## 會話管理

```
SessionKey = catclaw:{agent_id}:{origin}:{context_id}
```

**生命週期**：新建 → 活躍（subprocess 運行中）→ 閒置（30 分鐘無活動）→ 歸檔（7 天無活動，摘要寫入記憶）

**並行控制**：可設定最大並行數（預設 3），優先順序佇列（私訊 > 提及 > 排程任務）。

**無狀態重啟**：所有狀態在 SQLite 中。終止再重啟 — 會話透過 `--resume` 自動恢復。

## 設定

```toml
[general]
workspace = "./workspace"
state_db = "./state.sqlite"
max_concurrent_sessions = 3
port = 21130                        # WS + MCP 共用單一埠
streaming = true
default_model = "opus"              # 選填：opus, sonnet, haiku

[[channels]]
type = "discord"
token_env = "CATCLAW_DISCORD_TOKEN"
guilds = ["123456789"]
activation = "mention"

[[agents]]
id = "main"
workspace = "./workspace/agents/main"
default = true

[agents.approval]
timeout_secs = 120                  # 核准逾時（全域）
```

## 技術棧

| 元件 | Crate |
|------|-------|
| 非同步執行期 | `tokio` |
| Discord | `serenity` + `poise` |
| Telegram | `teloxide` |
| HTTP 伺服器 (WS + MCP) | `axum` |
| CLI | `clap` (derive) |
| 資料庫 | `rusqlite` (bundled SQLite, WAL) |
| TUI | `ratatui` + `crossterm` + `tui-textarea` |
| 設定 | `toml` + `serde` |
| 排程 | `croner` (cron 表達式) |
| 日誌 | `tracing` |

## 回饋

發現 bug 或有功能建議？[開一個 issue](https://github.com/CatiesGames/catclaw/issues)。

## 授權

MIT

---

<p align="center">
  以 Rust 和 Claude Code 建造<br>
  <strong>CatiesGames</strong>
</p>
