# Codex Runtime 支援 + 統一 Approval Pipeline — 設計藍圖（v3.4）

> v3.4 自 v3.3 修訂。目標從「無感切換」降級為「**功能對等 + documented 差異**」；補進第三方 MCP server 注入 codex / streaming 降級策略 / diary 品質驗證 / 配置變更生效規則 / cross-runtime regression CI / CLI 警告。

## 部署假設

- **Codex agent 必須跟 catclaw gateway 在同一 host**：codex 連 `http://127.0.0.1:{port}/mcp`。跨 host 部署、container 隔離、reverse proxy 場景**不在本期支援範圍**。Claude agent 既有同樣限制。
- **Codex `developer_instructions` 是 thread-bound 一次性**：catclaw 不能像 Claude 每輪追加 system prompt。動態 context 只能透過 user prompt 餵入。

## 目標（v3.4 修訂）

1. **零回歸**：既有所有 Claude agent / channel adapter / approval flow / memory / diary / TUI 面板 bit-identical。
2. **功能對等 + documented 差異**：
   - **介面 / 能力**：catclaw 用戶介面（Discord/TG/LINE/TUI 審核卡、CLI flag、tools.toml schema、approval 流程）100% 一致；catclaw 核心能力（IG/Threads/Contact/Memory/Skills/Approval/第三方 MCP）功能對等。
   - **不偽裝模型差異**：對話風格、reasoning 長度、工具選擇偏好等 model 本質差異 / codex CLI 固有限制（thread-bound prompt / 無原生 streaming），在 SKILL / README / TUI 明文 documented，**不**在 catclaw 層補洞。
3. **架構最小衝擊**：approval 雙時間模型分離，UI 卡片統一。
4. **不靠 OS 攔截規範 agent 行為**：業務流程放 SKILL 文件。

## v3.4 已知的 model / runtime 本質差異（documented，**不**處理）

| 差異 | 影響 | 處理 |
|---|---|---|
| GPT-5.5 vs Claude 4.7 對話風格 | agent「個性」感受不同 | README + SKILL 註明「切換 runtime 等同換 model」 |
| Codex thread-bound system prompt | 改 agent.system_prompt 後舊 thread 不吃新 prompt | §4.4 規則 + TUI 提示 |
| Codex 無 token streaming | Slack 從「逐字 streaming」降級成「等完一次發」 | §4.5 降級策略 |
| Codex 原生 shell 不過 catclaw approval | tools.toml `denied=["Bash"]` 對 codex shell 無效 | §4.1 SKILL 約束 + TUI 警告 + CLI 警告 |
| Codex 失敗工具不自動 retry | 對 transient error 不同行為 | documented，不補 |
| Diary extraction 對不同 model transcript 品質可能差 | codex agent 長期 memory 品質可能差於 Claude | §4.6 Phase B 量測，必要時改 prompt |

## 設計原則

- CatClaw 是 codex agent 唯一控制源
- Runtime 是 agent 級屬性
- 同步阻塞 vs 非同步 draft 兩種 approval 時間模型分離但 UI 統一
- 既有 Claude 路徑「一字不動」
- SKILL 約束業務流程，不靠 sandbox

---

## ★ Codex 與 Claude 行為差異（v3.2 PoC 實測）

| 行為 | Claude `-p` | Codex `exec` | 對 catclaw 設計影響 |
|---|---|---|---|
| System prompt 注入 | `--append-system-prompt` 每輪可變 | `-c developer_instructions` **僅 thread 首次 spawn 時生效**，resume 後改/拿掉都沒用 | catclaw 必須在**新 thread spawn 時一次打包**完整 system prompt（identity + skill index + memory L1） |
| Resume 時 system prompt | 每次重送 | **完全省略**（thread 已綁定原 prompt） | Codex resume 跳過 `-c developer_instructions`，省 token 與 CLI 解析開銷 |
| MCP 工具 NDJSON 表示 | `assistant.content[].tool_use.name` 用 prefixed (`mcp__catclaw__X`) | `item.started.item` 用 **bare** `server="catclaw"` + `tool="X"` | 命名規範總表（下節）涵蓋兩種 |
| Hook 觸發 | PreToolUse 透過 `--settings` JSON 注入 | exec 模式 user hooks **不觸發**（PoC 證實） | catclaw approval gate 對 codex 走 MCP intercept |
| `_meta.x-codex-turn-metadata` | 不送 | 每次 MCP `tools/call` 都送 `session_id / turn_id / model / sandbox / ...` | catclaw 從這裡識別 codex agent 身份 |
| Auth | `~/.claude/` 之外不認 | `CODEX_HOME` 環境變數可改 root | catclaw 用 isolated `.codex-home/` |
| AGENTS.md / CLAUDE.md 掃描 | CLAUDE.md 自動向上掃描 | AGENTS.md 自動掃描，可用 `project_doc_max_bytes=0` 關掉 | 兩者都用 workspace 保險絲檔 |

---

## ★ 命名規範總表（v3.2 強化 — 含第三方 MCP server）

| 表示位置 | 範例 | 來源 |
|---|---|---|
| MCP server `tools/list` 公告名稱 | `instagram_create_post` | `src/mcp_server.rs:252+`（bare） |
| MCP `tools/call` request 內 `params.name` | `instagram_create_post` | MCP 規範（bare） |
| Claude Code PreToolUse hook input `tool` 欄位 | `mcp__catclaw__instagram_create_post` | Claude CLI 慣例（prefixed） |
| `tools.toml` user 設定 | `mcp__catclaw__instagram_create_post` | 跟 hook input 一致（prefixed） |
| `SOCIAL_PUBLISH_TOOLS`（cmd_hook.rs:79-85） | `mcp__catclaw__instagram_create_post` | prefixed |
| Codex NDJSON `item.started.item.{server, tool}` | `server="catclaw", tool="instagram_create_post"` | bare 拆兩欄 |
| `RuntimeEvent::ToolUseStart.name`（codex 路徑構造） | `mcp__catclaw__instagram_create_post` | 用 server+tool 拼 `format!("mcp__{}__{}", server, tool)` |
| 第三方 MCP（user `.mcp.json` 加 pencil） | `mcp__pencil__open_document` | 用該 server name 拼，**不要** blanket `mcp__catclaw__` |

`Agent::tool_permission(name)` 規則：

```rust
impl Agent {
    /// 輸入 name 已經是 prefixed full name（含 server 或 built-in 直接名）。
    /// Codex 路徑由 mcp_server.rs::handle_codex_tool_call 接收 (server, tool) 後拼好再呼叫此方法。
    /// Claude 路徑來自 hook input，已經是 prefixed format。
    /// 內部不做模糊 normalization，只查 tools.toml 三個列表。
    pub fn tool_permission(&self, name: &str) -> Permission {
        if self.tools.denied.contains(&name.to_string()) { return Permission::Denied; }
        if self.tools.require_approval.contains(&name.to_string()) { return Permission::RequireApproval; }
        if self.tools.allowed.contains(&name.to_string()) { return Permission::Allowed; }
        Permission::Allowed  // default — 跟既有 Claude 行為一致
    }
}
```

**Codex tool dispatch 內部仍用 bare name**（既有 `mcp_server.rs::execute_*` 邏輯 prefix-match 不變）。Prefixed 形式只用於 `tool_permission` 查詢與 transcript 記錄。

---

## 一、Codex Runtime 隔離

### 1.1 Isolated CODEX_HOME

```
<workspace>/agents/<agent_id>/
├── .codex-home/
│   ├── auth.json            # symlink → AgentConfig.codex_auth_path（預設 ~/.codex/auth.json）
│   └── config.toml          # 空檔
├── AGENTS.md                # 空保險絲
├── CLAUDE.md                # 既有保險絲
├── tools.toml
└── ...
```

**Per-agent auth**：`AgentConfig.codex_auth_path: Option<PathBuf>`。

**Preflight 驗證**：`agents.new` / `agents.set_*` 必須驗證 target 存在。

**Symlink 寫穿透 PoC**：Phase B 第一週驗證 codex token refresh 是否破壞 symlink。

### 1.2 Spawn 旗標（thread 首次）

```rust
Command::new("codex")
    .env("CODEX_HOME", workspace.join(".codex-home"))
    .env_remove("CODEX_API_KEY")
    .arg("exec")
    .arg("--json")
    .arg("--skip-git-repo-check")
    .arg("--ignore-user-config")
    .arg("--ignore-rules")
    .arg("-C").arg(&workspace)
    .arg("-c").arg("project_doc_max_bytes=0")
    .arg("-c").arg(format!("model={}", quote(&model)))
    .arg("-c").arg("approval_policy=\"never\"")
    .arg("-c").arg(format!("sandbox_mode={}", quote(&sandbox)))
    .arg("-c").arg(format!("developer_instructions={}", quote(&full_system_prompt)))
    .arg("-c").arg(format!("mcp_servers.catclaw.url={}", quote(&format!("http://127.0.0.1:{port}/mcp"))))
    .arg("-c").arg("mcp_servers.catclaw.default_tools_approval_mode=\"approve\"")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
```

### 1.3 Spawn 旗標（resume）

PoC 證實 `developer_instructions` 在 resume 後不可變。**Resume 時省略**，省 token 與 fork 時間：

```rust
Command::new("codex")
    .env("CODEX_HOME", ...)
    .arg("exec")
    .arg("resume")
    .arg(&thread_id)
    .arg("--json")
    .arg("--skip-git-repo-check")
    .arg("--ignore-user-config")
    .arg("--ignore-rules")
    .arg("-C").arg(&workspace)
    .arg("-c").arg("project_doc_max_bytes=0")
    // model / sandbox / mcp_servers 仍要傳（codex resume 不繼承這些）
    .arg("-c").arg(format!("model={}", quote(&model)))
    .arg("-c").arg(format!("sandbox_mode={}", quote(&sandbox)))
    .arg("-c").arg(format!("mcp_servers.catclaw.url={}", ...))
    .arg("-c").arg("mcp_servers.catclaw.default_tools_approval_mode=\"approve\"")
    // 注意：不傳 developer_instructions（thread 內已綁定）
    // 注意：不傳 approval_policy（thread 內已綁定）
    .arg("-")  // 從 stdin 讀新 prompt
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
```

Resume PoC 驗證項目（Phase B 第一週確認）：哪些 `-c` 旗標 resume 後仍需重送、哪些可省。先列「全傳」當保守起點，確認 token 浪費再優化。

### 1.4 catclaw MCP 走 HTTP

PoC 證實 codex 完全支援 HTTP MCP transport，不需 stdio bridge。

`mcp_servers.catclaw.url` 值：`http://127.0.0.1:{port}/mcp`，port 來自 `config.general.port`（與 `/ws` 同 port，既有 `mcp_server.rs:17`）。

### 1.5 Codex 事件解析

| Codex 事件 | 對應 `RuntimeEvent` |
|---|---|
| `{"type":"thread.started","thread_id":"..."}` | `SystemInit { session_id }` |
| `{"type":"turn.started"}` | 內部 marker |
| `{"type":"item.started","item":{type:"command_execution",command,...}}` | `ToolUseStart { name: "shell", input: { command } }` |
| `{"type":"item.started","item":{type:"mcp_tool_call",server,tool,arguments}}` | `ToolUseStart { name: format!("mcp__{}__{}", server, tool), input: arguments }` |
| `{"type":"item.completed","item":{type:"agent_message","text":"..."}}` | `Assistant { content: [Text] }` |
| `{"type":"item.completed","item":{type:"command_execution",exit_code,aggregated_output}}` | `ToolResult { name: "shell", output: aggregated_output, is_error: exit_code != 0 }` |
| `{"type":"item.completed","item":{type:"mcp_tool_call",server,tool,result,error,status}}` | `ToolResult { name: format!("mcp__{}__{}", server, tool), output: result, is_error: status=="failed" }` |
| `{"type":"turn.completed","usage":{...}}` | `Result { result, session_id }`（result = 累積最後 agent_message） |
| `{"type":"turn.failed","error":{...}}` | `Result { result: error.message, session_id }` |
| `{"type":"error","message":"..."}` | warn log |

### 1.6 Transcript 寫入 — Codex 也要 parse 進 `TranscriptEntry`（reviewer 抓到的 P0）

既有 `transcript.rs:11-31` 寫的不是 raw NDJSON，是 catclaw 自家 `TranscriptEntry { timestamp, role, content, ... }`。Claude 端在 `manager.rs` 收 `ClaudeEvent` 後呼 `log_user/log_assistant/log_system`。

**Codex 端必須同樣 parse 進 `TranscriptEntry`**，**不可** 把 Codex NDJSON 原樣存。`manager.rs` event matching code（line 587 附近）遷移到 `RuntimeEvent` 後，兩 runtime 都呼同樣的 `transcript.log_*` API，產出格式一致的 TranscriptEntry，diary extraction（`scheduler.rs::check_diary_extraction` 用 `TranscriptLog::format_readable`）自動 work。

### 1.7 Transcript 不對稱接受

`ClaudeHandle` 不生成 ToolResult 事件（Claude NDJSON 沒對應）。`RuntimeEvent::ToolResult` 只 codex 用。Claude transcript 維持既有格式，diary 不受影響。

---

## 二、Runtime 抽象層

### 2.1 Enum dispatch

```rust
// src/session/runtime.rs (新)
pub enum RuntimeHandle {
    Claude(ClaudeHandle),
    Codex(CodexHandle),
}
impl RuntimeHandle {
    pub async fn recv_event(&mut self) -> Option<RuntimeEvent> { ... }
    pub async fn wait_for_result(&mut self, observer: Option<...>) -> Result<String> { ... }
    pub async fn kill(&mut self) -> Result<()> { ... }
    pub fn is_running(&mut self) -> bool { ... }
    pub fn session_id(&self) -> Option<&str> { ... }
}
pub enum RuntimeEvent { SystemInit{..}, Assistant{..}, TextDelta{..}, ToolUseStart{..}, ToolResult{..}, Result{..}, Unknown(Value) }
```

### 2.2 Runtime / AgentConfig

```rust
// src/config.rs
pub struct AgentConfig {
    pub id: String,
    pub workspace: PathBuf,
    pub default: bool,
    pub model: Option<String>,
    pub fallback_model: Option<String>,
    pub approval: ApprovalConfig,
    #[serde(default, skip_serializing_if = "is_default_runtime")]
    pub runtime: Runtime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_auth_path: Option<PathBuf>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime { #[default] Claude, Codex }
```

`catclaw.toml`（既有 `[[agents]]` array 形式）：
```toml
[[agents]]
id = "my-codex-agent"
workspace = "..."
runtime = "codex"
codex_auth_path = "/Users/foo/.codex/auth.json"
```

### 2.3 `Agent::spawn_session()` — 3 個 spawn 點 + event matching

> **行號錨點注意**：本 spec 內所有 `:LINE` 都是 v3.3 撰寫時的 snapshot 行號，作為定位用。實作時請以「描述性錨點」為準（如「`send_and_wait` 內主流程的 `ClaudeHandle::spawn_with_prompt` 呼叫」）；行號隨檔案變動，不可作為正式錨點。

3 個遷移點（描述性錨點）：
- `manager.rs::send_and_wait` 的 BOOT.md spawn 分支（snapshot 行號 ~211）
- `manager.rs::send_and_wait` 的主對話 spawn（snapshot 行號 ~283）
- `manager.rs::send_streaming` 的 spawn（snapshot 行號 ~517）

`ephemeral_run`（snapshot 行號 ~405）**刻意保留** 直呼 ClaudeHandle — 它故意不持久化 session，不適合塞進 spawn_session 抽象。

**`SpawnParams` struct 而非長 tuple**（reviewer P1 修正）：

```rust
pub struct SpawnParams<'a> {
    pub session_id: &'a str,
    pub model_override: Option<&'a str>,
    pub mcp_port: Option<u16>,
    pub hook_session_key: Option<&'a str>,  // Claude-only, Codex 忽略
    pub config_path: Option<&'a Path>,       // Claude-only, Codex 忽略
    pub mcp_env: &'a HashMap<String, HashMap<String, String>>,
    pub state_db: Option<&'a StateDb>,
    pub is_resume: bool,
    pub resume_thread_id: Option<&'a str>,   // Codex resume 用
}

impl Agent {
    pub async fn spawn_session(&self, params: SpawnParams<'_>, prompt: &str, env: &HashMap<String, String>) -> Result<RuntimeHandle> {
        match self.runtime {
            Runtime::Claude => Ok(RuntimeHandle::Claude(ClaudeHandle::spawn_with_prompt(self.claude_args_from(&params), prompt, env).await?)),
            Runtime::Codex => Ok(RuntimeHandle::Codex(CodexHandle::spawn_with_prompt(self.codex_args_from(&params), prompt, env).await?)),
        }
    }
}
```

**`manager.rs` event matching code 也要遷移到 `RuntimeEvent`** — `send_and_wait` 與 `send_streaming` 內 stream event 處理迴圈（snapshot 行號 ~587）的 `match &event { ClaudeEvent::... }` 全改 `RuntimeEvent::...`，否則 codex 寫不出 TranscriptEntry。

**Phase A 驗收項目**：「diff 一次 Claude session pre/post Phase A 的 transcript 檔，必須完全相同。」確保 RuntimeEvent 改造不會偷偷改寫 transcript bytes。

**`RuntimeEvent::Unknown(Value)` 對 Claude 來的事件**：既有 `ClaudeEvent::Unknown(Value)` 在 `From<ClaudeEvent>` 直接 map 到 `RuntimeEvent::Unknown(Value)`，內容 byte-by-byte 一致。Codex 來的未知事件同樣走 Unknown。Transcript / log 處理 Unknown 的既有邏輯不變。

### 2.4 Cross-runtime session resume — metadata.runtime

既有 `sessions.metadata TEXT` JSON 欄位（`state.rs` SessionRow 結構確認）。

**`metadata.runtime` 寫入流程**（v3.3 修正避免兩次 DB write）：
- 既有 `state_db.upsert_session(SessionRow { ..., metadata: Some(json!({...}).to_string()) })` 簽名已能容納
- `manager.rs` 建立 SessionRow 時，metadata JSON **同時** 加入 `runtime` 與既有欄位（`channel_id`/`sender_id`/`model`）
- **不**改 `upsert_session` 簽名、**不**做兩次 DB call（先 upsert 再 update）
- 新 session 一定有 runtime；舊 session 沒有，read 時 `runtime_from_metadata()` 返回 `None` → default `Claude`

Resume guard 路徑：
```rust
// manager.rs::send_and_wait 主流程入口
if let Some(row) = state_db.get_session(&session_key)? {
    let stored_runtime = row.runtime_from_metadata().unwrap_or(Runtime::Claude);
    if stored_runtime != agent.runtime {
        return Err(CatClawError::Session(format!(
            "Session was created with runtime={:?}; agent is now {:?}. Start a new session.",
            stored_runtime, agent.runtime
        )));
    }
    // resume 用 stored row.session_id
}
```

**錯誤呈現**：友善訊息走既有 `CatClawError::Session` 路徑，TUI / CLI / Channel adapter 既有錯誤渲染照常顯示給用戶。

**Codex 首次 spawn 無 thread_id 問題**：跟 Claude 既有流程相同 — `upsert_session` 先寫 row（session_id 暫無或佔位），subprocess 起來吐 `SystemInit { session_id }` 後 update row。Codex 走 `thread.started` 事件路徑等價。

**Phase A metadata diff 預期變化**：Phase A 加 `metadata.runtime = "claude"` 欄位到新建 row。舊 row metadata 不變。**驗收明文**：「sessions.metadata 新增 `runtime` 欄位是預期 Phase A 變更；既有讀取邏輯（platform_channel_id/sender_id/model helper）對 unknown 欄位透明，無回歸。」

### 2.5 CLI / TUI / Skill / README 同步

| 變更點 | 內容 |
|---|---|
| `catclaw.toml` `[[agents]]` | optional `runtime` / `codex_auth_path` |
| `src/main.rs` | `catclaw agent new --runtime codex --codex-auth-path ...` |
| `src/tui/agents.rs` | runtime + auth path UI；**新增 codex 原生工具警告**：用戶嘗試把 `shell` / `apply_patch` 設入 denied 時提示「Native Codex tools controlled by sandbox_mode, not tools.toml」 |
| `src/ws_server.rs` | `agents.new` / `agents.set_*` / `agents.reload_tools` 全收 runtime/auth_path |
| `SKILL_CATCLAW` | runtime-agnostic + codex 章節 + 業務流程約束（發 IG / 回 contact 必須走 catclaw MCP 工具） |
| `README.md` | Codex Support 章節 |

---

## 三、Approval 雙模型 + 統一 UI

### 3.1 兩種時間模型

| 類型 | 模型 | 持久 | timeout |
|---|---|---|---|
| 同步阻塞 | subprocess 等回應 | in-memory `pending_approvals: Arc<DashMap>` | 120s |
| 非同步 draft | admin 自由處理 | `social_drafts` / `contact_drafts` 表 | 無 |

### 3.2 ApprovalCard enum

```rust
pub enum ApprovalCard {
    Tool { approval_id, agent_id, session_id, tool_name, tool_input },
    SocialPost { draft_id, agent_id, platform, caption_preview, media_count, media_urls },
    ContactReply { draft_id, agent_id, contact_id, contact_display_name, platform, body_preview },
}
```

### 3.3 PendingApproval — 3-way ApprovalDecision

```rust
pub struct PendingApproval {
    pub request_id: String,
    pub session_key: SessionKey,
    pub agent_id: AgentId,                                  // 新增
    pub turn_id: Option<String>,                            // 新增（codex）
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub created_at: Instant,
    pub response_tx: oneshot::Sender<ApprovalDecision>,     // bool → ApprovalDecision
}
pub enum ApprovalDecision { Approved, Denied { reason: Option<String> }, Timeout }
```

3 個遷移點：`gateway.rs:316` / `:455` / `ws_server.rs:474`。`request_id` 用 `uuid::Uuid::new_v4().to_string()` 保 Discord 按鈕 callback format 一致。

### 3.4 WS wire format

既有 `ApprovalResultEvent`（reviewer 確認 approval.rs:44-50 已存在）：
```rust
pub struct ApprovalResultEvent {
    pub request_id: String,
    pub approved: bool,                                     // 永不移除
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,                             // 既有
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,                           // 新增：approved | denied | timeout
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,                           // 新增
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,                            // 新增（codex）
}
```

**`approval.respond` WS handler 升級**（reviewer 確認 既有 `ws_server.rs:464-478` 只收 `approved`）：
- 加 optional `reason` 參數
- 構造 `ApprovalDecision::Denied { reason }` 或 `Approved` 餵 `response_tx`
- 寫入 `ApprovalResultEvent.reason` 與 `decision`

`cmd_hook.rs:255` 升級：先讀 `decision` fallback `approved`。

**Phase B+D+E 驗收**：「`approved` 欄位永不從 wire 移除」明文寫入。

### 3.5 非同步 draft 型

IG / Threads / Contact 走 `social_drafts` / `contact_drafts` 表 + work card。

### 3.6 Claude vs Codex 路徑（v3.3 補全 Claude empty-approval fallback）

| Runtime + 配置 | Tool approval 走哪 | Social/Contact 走哪 |
|---|---|---|
| Claude，配 `require_approval` 包含該 social tool | PreToolUse hook（cmd_hook.rs，既有） | hook 攔截 → `social.draft.submit_for_approval` via WS → exit 2 deny。MCP server **看不到** |
| Claude，`require_approval` **不**含該 social tool（empty 或部分清單） | hook 不觸發（CLAUDE.md lesson 4） | MCP handler 收到呼叫，**Claude 路徑** = 無 `_meta`，走 `dispatch_existing_tool` → `execute_social_tool` 既有直呼 Meta API（v3.3 保留此既有行為） |
| Claude，工具是非社群（譬如 memory_write） | hook 不攔（不在 SOCIAL_PUBLISH_TOOLS） | MCP handler 收到，無 `_meta`，走 `dispatch_existing_tool` 既有路徑 |
| Codex（任何配置） | `handle_codex_tool_call` 同步阻塞 gate | `handle_codex_tool_call` 內社群分支 → `submit_for_approval_direct` → 立即回 success「queued」 |

**雙路徑互斥保證**：MCP handler 用 `_meta.x-codex-turn-metadata` 存在性區分（§5.3）。Claude 永遠沒 `_meta` → 永遠走 `dispatch_existing_tool` 既有邏輯。Codex 永遠送 `_meta` → 永遠走新 gate。

**`execute_social_tool` 簽名不動**：Codex 社群工具在 `handle_codex_tool_call` 入口就先攔，根本不會走到 `execute_social_tool`；Claude 路徑保持既有，包括「empty require_approval 時 IG MCP 走 direct Meta API」既有行為（明文保留為合法路徑）。

**SKILL「不要 shell curl Meta API」是給 Codex 的指示**：對 Claude 是冗餘（hook 已 gate）。SKILL 文字寫明 codex-specific reason 即可，Claude 看到不會多餘行為改變。

### 3.7 ChannelAdapter `send_approval` 改造

新 trait method `send_approval_card(&self, channel: &str, card: &ApprovalCard) -> Result<String>`。

舊 `send_approval(...)` 標 `#[deprecated]`，default impl 改為 wrap `ApprovalCard::Tool` 呼叫新 method（既有呼叫點不需即時遷移）。

**bit-identical 渲染**：Discord/Telegram/Slack 對 `ApprovalCard::Tool` 渲染輸出與既有 `send_approval` byte-by-byte 等價。Phase B+D+E 驗收項：「Claude tool approval Discord screenshot diff = 0」。

**LINE 新建**（reviewer 確認既有 `line.rs:213` 只處理 message event，postback 完全沒實作）：
- 加 postback event 解析
- 加 action receiver channel（mirror Discord `interaction_create_rx`）
- gateway.rs 接 wire
- Flex 三套 layout（tool / social / contact）

**LINE postback PoC 列為 B.1 prerequisite**（reviewer P2 修正），不能拖到 B.5。

### 3.8 TUI Pending Approvals — 新 `approvals.rs` + 原子搬遷

既有 `sessions.rs:109-126` 的 `pending_approvals: Vec<PendingApprovalItem>` + 鋪在 ~15 處的 button handler / 渲染 offset / 索引邊界（reviewer 確認）必須**原子搬遷**到新 `src/tui/approvals.rs`。

**Phase B+D+E 驗收**：「`sessions.rs` 內 `pending_approvals` 欄位與相關 ~15 處呼叫全部移除；`approval.pending` WS 事件只被 `approvals.rs` 消費；既有 TUI 既有 session 頁面在無 approval 時無變化」。原子 commit。

---

## 四、Codex 工具 approval 範圍 + Runtime 差異處理

### 4.1 Codex 原生工具不 gate + 雙重警告

`shell` / `apply_patch` 走 codex OS sandbox。catclaw 不攔截。

**UX 警告同步雙位**（v3.4 修正 gap F）：
- **TUI**（`src/tui/agents.rs`）：用戶嘗試把 `Bash`/`shell`/`apply_patch` 放入 `denied` 時即時提示「Native Codex tools controlled by sandbox_mode, not tools.toml」
- **CLI**（`src/main.rs::handle_agent_set_tools`）：同邏輯，命令列下命令時也警告
- **TOML load**（`src/agent/loader.rs::load_tools_toml`）：載入時若 codex agent 的 tools.toml 含 `shell` / `apply_patch` 在任何列表，log warn 並標 deprecation

### 4.2 catclaw approval gate 覆蓋範圍

覆蓋 catclaw MCP 工具 + 經 catclaw 注入的第三方 MCP 工具（兩者都會經過 `mcp_server.rs::handle_mcp` 或 catclaw 注入 stub）。

### 4.3 第三方 MCP server 注入 codex（v3.4 修正 gap A）

**問題**：v3.3 §1.2 只 inject `mcp_servers.catclaw`，user `.mcp.json` 的 pencil/figma 等第三方 MCP 對 codex agent **消失**。Claude 既有自動讀 workspace `.mcp.json` 並 inject，行為不同。

**解法**：`codex_args_from(SpawnParams)` 與 `claude_args_with_mcp` 共用「讀 workspace `.mcp.json` 列出所有 MCP server」邏輯，把每個第三方 server 都用 `-c mcp_servers.X.<type/command/args/url>=...` 注入 codex。

```rust
// src/agent/codex_args.rs
fn inject_user_mcp_servers(args: &mut Vec<String>, workspace: &Path) {
    let mcp_json_path = workspace.join(".mcp.json");
    if !mcp_json_path.exists() { return; }
    let Ok(content) = fs::read_to_string(&mcp_json_path) else { return };
    let Ok(parsed): Result<serde_json::Value, _> = serde_json::from_str(&content) else { return };
    let Some(servers) = parsed.get("mcpServers").and_then(|v| v.as_object()) else { return };

    for (name, cfg) in servers {
        if name == "catclaw" { continue; }  // catclaw 自己 inject 過了
        // 對每個 server，根據 type（stdio command/args 或 streamable_http url）注入對應 codex -c
        if let Some(typ) = cfg.get("type").and_then(|v| v.as_str()) {
            match typ {
                "stdio" | "" => {
                    if let Some(cmd) = cfg.get("command").and_then(|v| v.as_str()) {
                        args.push("-c".into()); args.push(format!("mcp_servers.{}.command={}", name, quote(cmd)));
                        if let Some(a) = cfg.get("args") {
                            args.push("-c".into()); args.push(format!("mcp_servers.{}.args={}", name, a));
                        }
                        if let Some(e) = cfg.get("env").and_then(|v| v.as_object()) {
                            for (k, v) in e {
                                args.push("-c".into()); args.push(format!("mcp_servers.{}.env.{}={}", name, k, v));
                            }
                        }
                    }
                }
                "http" | "streamable_http" => {
                    if let Some(url) = cfg.get("url").and_then(|v| v.as_str()) {
                        args.push("-c".into()); args.push(format!("mcp_servers.{}.url={}", name, quote(url)));
                    }
                }
                _ => log::warn!("unsupported mcp server type for codex: {} ({})", name, typ),
            }
        }
        // 第三方 MCP 工具預設不過 codex 自家 approval（catclaw 也不 gate）
        args.push("-c".into()); args.push(format!("mcp_servers.{}.default_tools_approval_mode=\"approve\"", name));
    }
}
```

**Phase B 驗收**：「user `.mcp.json` 加 pencil → codex agent `tools/list` 看得到 `mcp__pencil__*` 工具且可呼叫」。

### 4.4 配置變更生效規則（v3.4 修正 gap D）

| 配置 | Claude 生效時機 | Codex 生效時機 |
|---|---|---|
| `agent.model` | 下一輪 spawn（既有） | **新 thread** 才生效；既有 thread 繼續用初始 model |
| `agent.system_prompt` / SKILL | 下一輪 spawn 即生效 | **新 thread** 才生效；既有 thread 繼續用初始 prompt |
| `tools.toml`（require_approval / denied / allowed） | 下一輪 spawn 即生效（hook 重讀） | 下一輪 spawn 即生效（catclaw MCP gate 動態讀取 agent.tool_permission） |
| `codex_auth_path` | N/A | 下一輪 spawn 即生效 |
| `runtime` 本身 | 切換不影響舊 session（v3.3 §2.4 guard） | 同上 |

**TUI 提示** + **CLI flag warn**：改 codex agent 的 model / system_prompt / SKILL 後，列出該 agent 的 active codex sessions 並提示「config saved; existing codex threads will continue with the prior settings. New threads will pick up changes.」

**`agents.reload_tools` WS handler** 對 codex agent 不嘗試重啟 thread — 既有 active session 跑完自然死亡（既有 Claude 也不重啟 in-flight session）。

### 4.5 Streaming 降級策略（v3.4 修正 gap B）

Claude 透過 `--include-partial-messages` 產 `stream_event` 給 Slack 做 native streaming（CLAUDE.md 既有）。Codex `--json` 沒等價 token-level delta 事件。

**v3.4 策略**：
- `RuntimeHandle::recv_event` 從 codex 收到 `item.completed` 含 `agent_message` 時，**buffer 累積後一次性產 Assistant event**
- `manager.rs::send_streaming` 對 codex agent 走「結束才發」路徑，跟 Claude `streaming = false` 的行為一致
- Channel adapter 既有 `send_stream_*` 對 codex agent **不呼叫**；只呼 `send_message` 一次性發送

**用戶體驗 documented**：「Slack streaming response 在 codex agent 上會變成『等完整段才發送』，不會逐字流式顯示。這是 codex CLI 限制。」README 寫明。

**Phase B 驗收**：「Slack agent 用 codex runtime 跑長對話 → 訊息一次性出現，不卡住、不破訊息分塊。」

### 4.6 Diary / Memory 品質驗證（v3.4 修正 gap C）

`scheduler.rs::check_diary_extraction` 把 `TranscriptLog::format_readable` 結果丟給 Haiku 提取。Haiku prompt 既有是針對 Claude 對話格式設計。Codex transcript 即使統一寫成 `TranscriptEntry`，內容語氣 / 工具呼叫頻率 / reasoning 段落結構不同，Haiku 提取品質可能差。

**v3.4 策略**：
- **Phase B 量測**：跑 codex agent 兩天，比對 Haiku 對 codex transcript vs Claude transcript 提取出的 memory facts 數量、品質（手工抽樣評分）
- 若品質明顯差 → 調 Haiku prompt 加 runtime-aware 指示（譬如「以下對話來自 GPT-5.5 / Claude 4.7」），不改 transcript 格式
- 若品質可接受 → 維持現狀

**驗收項目**：「跑 codex agent ≥ 50 個對話輪 → diary extraction 不報錯、提取出的 memory facts 經人工抽樣品質 ≥ Claude baseline 70%」（量化門檻可調）。

### 4.7 Cross-runtime regression CI（v3.4 修正 gap E）

CI 加新 job：用相同 prompts 跑 Claude agent + Codex agent，比對：
- 是否都產出 agent_message（不是 turn.failed）
- 工具呼叫種類是否覆蓋預期清單（memory_write / contacts_reply / instagram_create_post）
- 不比對「文字內容相同」（model 行為自然差異）
- 比對「結構性結果相同」（譬如 IG draft 是否寫入 social_drafts、approval card 是否送出 4 個 surface、TranscriptEntry 是否正常累積）

**CI script 位置**：`scripts/regression-cross-runtime.sh`，CI 跑時建一個 throwaway gateway + 兩個 agent（claude/codex）+ 同一個 mock channel，跑預定義 prompts 矩陣。

### 4.8 SKILL 業務流程約束

`SKILL_CATCLAW` 新章節（runtime-agnostic）：

```
## 發 IG / Threads / 回覆 Contact 的正確方式

要發 IG 貼文：用 `mcp__catclaw__instagram_create_post`。
不要 shell curl graph.facebook.com。原因：
1. catclaw MCP 工具走 approval，admin 可審核
2. 直接呼 API 繞過審核 — admin 不知道你發了什麼
3. 不會記錄到 social_drafts，沒有歷史追蹤

要回覆 contact：用 `mcp__catclaw__contacts_reply`。
不要用 `discord_send_message` / `line_send_message` 平台級工具。

要寫 memory：用 `mcp__catclaw__memory_write`。

正確路徑 = 自動審核 + 自動記錄 + 對 admin 透明。
```

---

## 五、CatClaw HTTP MCP server approval 攔截

### 5.1 既有結構事實確認

- MCP server stateless（不識別 session）
- 既有 `pending_approvals: Arc<DashMap>` 在 `GatewayHandle` 上
- 既有 dispatch 用 **bare** tool names（`mcp_server.rs::handle_mcp` 內 prefix-match 分支）
- `notifications/initialized` 既有回 `200 + {}` — 對 codex 不合規

### 5.1.1 抽出 `dispatch_existing_tool` helper（v3.3 新增）

既有 `mcp_server.rs::handle_mcp` 在 `"tools/call"` 分支內**inline** 寫了 prefix-match dispatch（memory_/kg_/contacts_/instagram_/threads_/discord_/telegram_/slack_/line_）。v3.3 把這段抽成獨立 helper：

```rust
async fn dispatch_existing_tool(
    gw: &GatewayHandle,
    tool_name: &str,
    arguments: serde_json::Value,
    id: serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    // 把既有 handle_mcp 的 "tools/call" 分支內所有 prefix-match dispatch 邏輯搬進這裡
    // 行為 byte-by-byte 等價既有。Claude 路徑與 Codex Allowed 路徑共用。
}
```

**Phase B.4 第一步**：先做這個 helper 抽出（pure refactor，無行為變化），再加 Codex 分流。

**驗收**：抽出前後 Claude 用 MCP 跑所有 catclaw tool 種類（memory / contacts / social / adapter actions / Discord/Telegram/Slack/LINE actions）回應 byte-by-byte 一致。

### 5.2 `_meta` 解析

`_meta` 是 MCP 協議 spec 中 `tools/call` request 的擴充欄位。**Claude 與 Codex 兩個 client 都會送 `_meta`**，但內容 key 不同（PoC 驗證見 §5.2.1）。`extract_codex_meta` 找的是 Codex 專屬的 `x-codex-turn-metadata` 子鍵，Claude 的 `claudecode/toolUseId` 不會誤觸：

```rust
fn extract_codex_meta(params: &Value) -> Option<(String, Option<String>)> {
    let meta = params.get("_meta")?;
    // Codex-specific marker: presence of x-codex-turn-metadata = codex client.
    // Claude sends _meta with different keys (claudecode/toolUseId), so this
    // returns None for Claude requests → caller falls through to legacy path.
    let turn_meta = meta.get("x-codex-turn-metadata")?;
    let session = turn_meta.get("session_id").and_then(|v| v.as_str())?.to_string();
    let turn = turn_meta.get("turn_id").and_then(|v| v.as_str()).map(String::from);
    Some((session, turn))
}

fn resolve_agent_from_session(gw: &GatewayHandle, session_id: &str) -> Option<Arc<Agent>> {
    let row = gw.state_db.get_session_by_session_id(session_id).ok()??;
    gw.agent_registry.read().unwrap().get(&row.agent_id).cloned()
}
```

### 5.2.1 `_meta` 內容分辨 Claude vs Codex（v3.4 PoC 驗證）

**重要前提：兩個 runtime 都會送 `_meta`**，但 key 不同：

| Runtime | MCP `tools/call` 的 `_meta` 內容 |
|---|---|
| Claude (`claude-code/2.1.142`) | `{"claudecode/toolUseId":"toolu_...","progressToken":N}` |
| Codex (`codex-mcp-client/0.130.0`) | `{"x-codex-turn-metadata":{"session_id":"...","turn_id":"...","model":"...","sandbox":"...","turn_started_at_unix_ms":N,...},"progressToken":N}` |

**分流靠 `_meta.x-codex-turn-metadata` 這個具體 key 的存在性**，不是 `_meta` 本身存在性。Claude 送的 `_meta.claudecode/toolUseId` 不會誤觸 Codex 路徑（`extract_codex_meta` 返回 `None`），Claude 走 fallback 進 `dispatch_existing_tool` 既有邏輯。

HTTP User-Agent 也可區分（Claude `claude-code/X.Y.Z (sdk-cli)` vs Codex `codex-mcp-client`），但 v3.4 不依賴 user-agent sniffing（脆弱、Anthropic 可能改版本字串）— 純粹靠 `_meta.x-codex-turn-metadata` 結構性 marker。

**Phase B 驗收**：補 PoC script 同時跑 Claude + Codex agent 對同一 catclaw MCP endpoint，確認兩 runtime 各自走對應路徑、互不誤觸。

### 5.3 `handle_tool_call` 分流（v3.3：JSON-RPC `id` 一路帶下去）

```rust
async fn handle_tool_call(
    id: serde_json::Value,
    body: serde_json::Value,
    gw: Arc<GatewayHandle>,
) -> (StatusCode, Json<serde_json::Value>) {
    let params = body.get("params").cloned().unwrap_or(serde_json::Value::Null);
    let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or_default();

    match extract_codex_meta(&params) {
        // ===== Codex 路徑 =====
        Some((session_id, turn_id)) => {
            // _meta 存在但 resolve 失敗 = 硬錯（不 silent fallthrough）
            let Some(agent) = resolve_agent_from_session(&gw, &session_id) else {
                return mcp_error_response(id, &format!("unknown codex session: {}", session_id));
            };
            handle_codex_tool_call(id, &gw, tool_name, arguments, &agent, &session_id, turn_id).await
        }
        // ===== Claude 路徑（或無 _meta 客戶端）=====
        None => {
            // 直接走抽出的 dispatch helper — 既有邏輯一字不動
            dispatch_existing_tool(&gw, tool_name, arguments, id).await
        }
    }
}
```

JSON-RPC `id` 是 request-response 對應的唯一鍵，**必須** 從 `body.get("id")` 一路帶到所有 response builder（`mcp_error_response(id, ...)` / `mcp_success_text(id, ...)`）。Pseudocode 內所有 `id` 都來自 caller 注入，無例外。

### 5.4 `handle_codex_tool_call`

```rust
async fn handle_codex_tool_call(
    id: serde_json::Value,
    gw: &GatewayHandle,
    tool_name: &str,
    arguments: serde_json::Value,
    agent: &Agent,
    session_id: &str,
    turn_id: Option<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let prefixed = format!("mcp__catclaw__{}", tool_name);  // codex 路徑：bare → prefixed for tool_permission

    // 1) 社群發文 — 非同步 draft（v3.2: Phase E 合入 B+D+E）
    if matches!(tool_name, "instagram_create_post" | "instagram_send_dm" | "instagram_reply_comment" | "threads_create_post" | "threads_reply")
       && social_review_required_for(gw, tool_name) {
        let draft_id = social::submit_for_approval_direct(gw, tool_name, &arguments, agent).await?;
        return mcp_success_text(id, &format!("draft {} queued for review", draft_id));
    }

    // 2) Contact 回覆 — 非同步 draft
    if tool_name == "contacts_reply" && contact_approval_required_for(gw, &arguments)? {
        let draft_id = contacts::pipeline::submit_reply_internal(gw, &arguments).await?;
        return mcp_success_text(id, &format!("draft {} queued", draft_id));
    }

    // 3) 同步阻塞 — 一般 catclaw MCP 工具
    match agent.tool_permission(&prefixed) {
        Permission::Denied => mcp_error_response(id, "tool denied by policy"),
        Permission::Allowed => dispatch_existing_tool(gw, tool_name, arguments, id).await,
        Permission::RequireApproval => {
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = oneshot::channel();
            let approval = PendingApproval {
                request_id: request_id.clone(),
                session_key: SessionKey::from_agent_session(&agent.id, session_id),
                agent_id: agent.id.clone(),
                turn_id,
                tool_name: prefixed.clone(),
                tool_input: arguments.clone(),
                created_at: Instant::now(),
                response_tx: tx,
            };
            gw.pending_approvals.insert(request_id.clone(), approval);
            send_approval_card_to_channel(gw, agent, ApprovalCard::Tool {
                approval_id: request_id.clone(),
                agent_id: agent.id.clone(),
                session_id: Some(session_id.to_string()),
                tool_name: prefixed,
                tool_input: arguments.clone(),
            }).await?;
            let timeout_secs = agent.approval.timeout_secs;
            match timeout(Duration::from_secs(timeout_secs), rx).await {
                Ok(Ok(ApprovalDecision::Approved)) => dispatch_existing_tool(gw, tool_name, arguments, id).await,
                Ok(Ok(ApprovalDecision::Denied { reason })) => mcp_error_response(id, &format!("admin denied: {}", reason.unwrap_or_default())),
                Ok(Ok(ApprovalDecision::Timeout)) | Err(_) => mcp_error_response(id, "approval timeout"),
                Ok(Err(_)) => mcp_error_response(id, "approval channel closed"),
            }
        }
    }
}
```

### 5.4.1 `submit_for_approval_direct` lock 安全保證（v3.3 新增）

`social::submit_for_approval_direct` 由兩個 caller 共用（`ws_server.rs` WS handler + `mcp_server.rs` HTTP handler）。兩 caller 持鎖狀態不同，helper 內部**禁止** 持有 caller-side 已持有的 lock 以避免死鎖。

設計守則：
- Helper 內部只 `await` 落地寫入 `state_db.insert_social_draft`（既有 API）+ adapter `send_approval_card`
- **不**在 helper 內呼叫任何 `gw.config.read()` 之類 long-lived lock；需要 config 值時 caller 先取出 plain owned value 傳入
- Phase B.4 加 audit：用 `tokio::time::timeout` 包 helper call，超時即視為死鎖 fail，CI 跑壓測抓

### 5.5 `notifications/*` 回 204

`mcp_server.rs::handle_mcp` 內 `"notifications/initialized"` 分支改：
```rust
"notifications/initialized" => return (StatusCode::NO_CONTENT, ()),
// 其他 notifications/* 同
```

**Phase B.1 第一日驗證**：用 Claude 跑完整 regression（IG hook / contacts / tool approval / memory），確認 Claude MCP client 對 204 沒問題。若 Claude 壞 → file Claude CLI bug + 暫時回 200 直到 upstream fix（**不**用 user-agent sniffing fallback，reviewer 確認此 fallback 脆弱）。

### 5.5.1 Codex agent 啟動 sanity check（v3.3 新增）

Codex agent 首次 spawn 後，第一輪 codex 子程序會連 `http://127.0.0.1:{port}/mcp` 做 `initialize` + `tools/list`。若 MCP server 不通或 streamable HTTP 規範不符，**codex 仍會跑** 但少所有 catclaw 工具，agent 行為退化為只能用 codex 原生 shell。

**Phase B 驗收**：「Codex agent 啟動後第一輪 `tools/list` 必須回傳所有 catclaw 公開工具 — 用 codex agent 跑「請列出你可用的 mcp 工具」prompt，比對輸出 vs `mcp_server.rs::build_tool_list` 預期清單。」

### 5.6 MCP HTTP server 併發

既有 Axum per-request task（reviewer 確認）。120s 阻塞單請求不影響其他。Phase B+D+E 加併發壓測。

### 5.7 `social::submit_for_approval_direct` helper

`cmd_hook.rs` 是 subprocess 透過 WS 呼自己（既有）。`mcp_server.rs` 在 gateway 內，**不能** 自呼 WS（會循環）。

**抽 in-process helper** `src/social/mod.rs::submit_for_approval_direct(gw: &GatewayHandle, tool_name: &str, args: &Value, agent: &Agent) -> Result<String>`，直接寫 DB + 發卡。

兩個 caller：
- `ws_server.rs::handle_social_draft_submit_for_approval`（既有 WS endpoint，hook 透過 WS 呼進來）
- `mcp_server.rs::handle_codex_tool_call`（codex 直呼 in-process）

```
cmd_hook.rs ──WS──> ws_server.rs::handle_social_draft_submit_for_approval
                                                                   ↓
                                                                   social::submit_for_approval_direct
                                                                   ↑
              mcp_server.rs::handle_codex_tool_call ────────────────┘
```

---

## 六、檔案異動清單

### 6.1 新增

| 檔案 | 目的 |
|---|---|
| `src/session/codex.rs` | `CodexHandle` |
| `src/session/runtime.rs` | `RuntimeHandle` + `RuntimeEvent` + `SpawnParams` |
| `src/agent/codex_args.rs` | `codex_args_from(SpawnParams)` builder（含第三方 MCP 注入） |
| `src/tui/approvals.rs` | 新統一面板 |
| `scripts/regression-cross-runtime.sh` | CI cross-runtime regression（v3.4 gap E） |

### 6.2 修改

| 檔案 | 變更 |
|---|---|
| `src/session/mod.rs` | export 新型別 |
| `src/session/claude.rs` | `From<ClaudeEvent> for RuntimeEvent` |
| `src/session/manager.rs` | 3 spawn 點 + **event matching code (line 587)** 都換 RuntimeEvent；`ephemeral_run` 保留；建 SessionRow 處呼 `set_runtime_in_metadata` |
| `src/session/transcript.rs` | 新增 `log_tool_use_codex` / `log_tool_result_codex` API（Claude 端不動） |
| `src/agent/mod.rs` | 加 `runtime` / `codex_auth_path` + `spawn_session()` + `tool_permission()` + `Permission` enum |
| `src/agent/loader.rs` | 建 `.codex-home/` + auth symlink + preflight；`SKILL_CATCLAW` 加業務流程約束 |
| `src/config.rs` | `AgentConfig.runtime` / `codex_auth_path`（optional） |
| `src/approval.rs` | `ApprovalCard` enum；`PendingApproval` 升 ApprovalDecision；`ApprovalResultEvent` 加 `decision`/`agent_id`/`turn_id` |
| `src/ws_protocol.rs` / `ws_server.rs` | `approval.respond` 收 optional `reason`；`agents.new`/`set_*`/`reload_tools` 收 runtime/auth_path；`handle_agents_delete` 加清 `.codex-home/auth.json`；line 349 構造 / line 474 `send` 升 ApprovalDecision |
| `src/channel/mod.rs` | 新 `send_approval_card` trait method；舊 `send_approval` deprecated 內部 wrap |
| `src/channel/discord.rs`, `telegram.rs`, `slack.rs` | `render_card` helper + 三套 layout；Tool kind bit-identical |
| `src/channel/line.rs` | **新建** postback event 解析 + action receiver channel + Flex 三套 layout |
| `src/tui/sessions.rs` | **原子刪除** inline pending_approvals Vec / 渲染 / ~15 處 button handler |
| `src/mcp_server.rs` | `extract_codex_meta` + `resolve_agent_from_session` + `handle_codex_tool_call` 分流；`notifications/*` 改 NO_CONTENT |
| `src/cmd_hook.rs` | `approval.result` 先讀 `decision` fallback `approved`；社群 draft 走 WS（不變） |
| `src/social/mod.rs`, `instagram.rs`, `threads.rs` | 抽 `submit_for_approval_direct` helper 給 ws_server + mcp_server 共用 |
| `src/contacts/pipeline.rs` | 卡片渲染改 `ApprovalCard::ContactReply` |
| `src/state.rs` | `SessionRow` 加 `runtime_from_metadata()` / `set_runtime_in_metadata()` helper；schema 不變 |
| `src/scheduler.rs` | （無變更 — codex transcript 由 `manager.rs` 寫成標準 `TranscriptEntry` 後既有 diary 路徑自動相容） |
| `src/gateway.rs` | 2 個 `response_tx.send` 點（line 316/455）改 ApprovalDecision；LINE postback receiver 接入 |
| `src/tui/agents.rs` | runtime + codex_auth_path UI + 原生 codex 工具警告訊息 |
| `src/main.rs` | CLI 子命令 |
| `README.md` | Codex Support 章節 |

### 6.3 不動

| 檔案 | 為什麼 |
|---|---|
| `src/router.rs` | 路由邏輯不變 |
| `src/state.rs` schema | 用既有 metadata JSON 欄位 |
| `src/scheduler.rs::check_diary_extraction` | codex transcript 統一寫成 TranscriptEntry 後自動相容 |

### 6.4 `agents.delete` in-flight session 處理（v3.3 新增）

既有 `handle_agents_delete` 不主動 stop in-flight session（CLAUDE.md 未明列）。v3.3 codex 加 `.codex-home/auth.json` symlink 清理，**順序明寫**：

1. **先** 呼 `sessions.stop` 把該 agent 所有 active session 終止（既有 `gw.session_manager` 應有 stop API；若無則 Phase B.2 補）
2. **再** 從 catclaw.toml 移除 agent
3. **再** 從 in-memory registry 移除
4. **最後** 清 `.codex-home/auth.json` symlink（若存在）

Sub-process 終止後再刪 symlink 可避免：active codex 子程序持有 auth.json file handle 期間 symlink 被改寫，token refresh 觸發未定義行為。

**Phase B.2 驗收**：「`agents.delete` 期間若有 codex 子程序在跑，先 stop session 等 subprocess 退出，再清 symlink。Sanity test: 跑 codex agent → delete agent → 確認無 zombie subprocess + symlink 已清。」

---

## 七、實施階段（B+D+E 合併單一 release）

### Phase A：Runtime 抽象（內部重構，零行為變化）

1. `RuntimeHandle` / `RuntimeEvent` / `SpawnParams` enum + `From<ClaudeEvent>`
2. `Agent::spawn_session()` 3 spawn 點 + event matching code 都換 RuntimeEvent
3. `ephemeral_run` 保留直呼 ClaudeHandle
4. `state.rs::SessionRow` 加 `runtime_from_metadata` / `set_runtime_in_metadata` helper
5. `manager.rs` 建 SessionRow 處同步寫入 `metadata.runtime = "claude"`
6. **驗收**：
   - cargo clippy 零警告
   - 既有 Claude regression 全通過
   - **transcript JSONL diff = 0**（同 prompt 跑 Phase A 前後比對）
   - sessions.metadata 多 `runtime` 欄位是預期變化；既有 read helper（`platform_channel_id` / `sender_id` / `model`）對 unknown 欄位透明
   - `RuntimeEvent::Unknown` 對既有 Claude event byte-identical map

### Phase B+D+E：Codex runtime + Approval gate + Social draft（單一 release）

#### B.1 PoC 前置（第一週，blocking）
- `notifications/*` 改 204，Claude regression 全跑（IG hook / contacts / tool approval / memory）
- auth.json symlink rename 寫穿透測試
- HTTP MCP codex 工作流 PoC
- **LINE postback round-trip PoC**（用真實 LINE channel 測 webhook 收到 postback、postback_data 路由通）
- Codex `developer_instructions` 行為驗證腳本進 CI（防 codex 升版破壞 thread-bound 行為）

#### B.2 Runtime 基礎
- `CodexHandle` + NDJSON 解析（含 server+tool 兩欄拆解）
- `codex_args_from(SpawnParams)` 首次與 resume 兩種模式
- `agent/loader.rs` 建 `.codex-home/` + preflight 驗證 `codex_auth_path` 存在
- `handle_agents_delete` 依序：(1) stop in-flight sessions (2) 移除 config (3) 清 `.codex-home/auth.json` symlink
- `Agent::tool_permission()`（接 prefixed full name，不做模糊 normalize）
- cross-runtime resume guard（友善錯誤透過 `CatClawError::Session` 路徑）
- Codex 首次 spawn 無 thread_id 流程驗證等價 Claude

#### B.3 ApprovalCard + 同步路徑
- `ApprovalCard` enum
- `PendingApproval` 升 ApprovalDecision
- WS wire `decision`/`agent_id`/`turn_id` 欄位（`approved` 永保留）
- `approval.respond` 收 optional `reason`
- `cmd_hook.rs` 升級 wire 讀取（先 `decision` fallback `approved`）
- `gateway.rs` / `ws_server.rs` 3 個 send 點遷移

#### B.4 MCP handler Codex 分流
- **第一步**：抽出 `dispatch_existing_tool` helper（pure refactor）— 既有 `handle_mcp::"tools/call"` 內 prefix-match dispatch 全部搬進。驗證 Claude 用 MCP 跑所有工具種類 byte-by-byte 一致。
- `extract_codex_meta` + `resolve_agent_from_session`
- Claude 路徑（無 `_meta`）走 `dispatch_existing_tool` **一字不動**
- Codex 路徑（有 `_meta`）→ `handle_codex_tool_call`
- **`_meta` 存在但 resolve 失敗 = 硬錯**（不 silent fallthrough）
- 同步阻塞 + 社群/contact 非同步 draft 分支
- `social::submit_for_approval_direct` helper 抽出（不持 caller-side lock）
- JSON-RPC `id` 一路帶下去到所有 response builder
- 併發 audit + 兩 codex agent 同時觸發 approval 壓測 + helper lock 死鎖檢測
- **Codex agent 啟動 sanity check**：第一輪 `tools/list` 必須回傳完整 catclaw tool 清單

#### B.5 Channel adapter
- Discord/Telegram/Slack `render_card` + 三套 layout
- **Tool kind 視覺 bit-identical 既有版本（screenshot diff = 0）**
- LINE **新建** postback infrastructure + send_approval_card + Flex 三套 layout

#### B.6 TUI 新面板
- `src/tui/approvals.rs` 新建（三來源：in-memory pending_approvals + social_drafts + contact_drafts）
- `sessions.rs` 原子刪除 inline pending_approvals（~15 處呼叫）— 單一 commit

#### B.7 CLI + 文件 + Runtime 差異處理
- `--runtime codex --codex-auth-path` CLI / TUI
- 三重警告（TUI / CLI / TOML load）：codex 原生工具放入 denied 無效（gap F）
- `SKILL_CATCLAW` 業務流程約束章節
- README 增加 v3.4 「documented 差異」表
- 配置變更生效規則 TUI/CLI 提示（gap D）：改 codex agent 的 model/system_prompt 後標記既有 thread 不會 pick up

#### B.8 第三方 MCP server 注入 codex（v3.4 gap A，blocking）
- `src/agent/codex_args.rs::inject_user_mcp_servers` 實作
- 支援 stdio + streamable_http 兩種 transport
- Phase B 驗收：user 加 pencil MCP → codex `tools/list` 看得到

#### B.9 Streaming 降級（v3.4 gap B）
- `RuntimeHandle::recv_event` 對 codex 收 `item.completed` 後一次性產 Assistant event
- `send_streaming` 對 codex agent 走「結束才發」
- 既有 `send_stream_*` 對 codex agent 不呼叫
- Slack agent 用 codex 驗證訊息一次性出現

#### B.10 Diary 品質量測（v3.4 gap C）
- 跑 codex agent ≥ 50 對話輪，比對 Haiku 提取品質
- 必要時調 Haiku prompt 加 runtime-aware 指示
- 結果寫入 Phase B 量測報告（不 ship 改 Haiku prompt 除非品質明顯差）

#### B.11 Cross-runtime regression CI（v3.4 gap E）
- `scripts/regression-cross-runtime.sh` 撰寫
- CI 整合 — 每 PR 自動跑
- 比對結構性結果（不比對文字）

**Phase B+D+E 出貨驗收**（單一 release）：

零回歸（Goal 1）
- [ ] 既有 Claude regression：IG 發文（hook） / contacts reply / tool approval / memory write / diary 提取
- [ ] Claude tool approval 卡片視覺 bit-identical（Discord/Telegram/Slack screenshot diff）
- [ ] WS wire `approved` 欄位永不移除
- [ ] `notifications/*` 改 204 後 Claude 跑既有功能全通過
- [ ] `sessions.rs` 移除 inline pending_approvals 後既有 TUI 頁面渲染正常
- [ ] cross-runtime resume 不 silent crash

功能對等（Goal 2）
- [ ] 建 codex agent → AGENTS.md/CLAUDE.md 隔離 prompt 回 `NONE`
- [ ] Codex agent 啟動 sanity：第一輪 `tools/list` 回傳完整 catclaw tool 清單
- [ ] Codex tool require_approval → 卡片送 Discord/Telegram/LINE/TUI 四 surface
- [ ] Codex Approve / Deny with reason / Timeout 三結果正確處理
- [ ] **Codex 呼 `instagram_create_post` → 卡片 → admin approve → 真的發 IG**
- [ ] **Codex 呼 `contacts_reply` → 卡片 → admin approve → 對 contact 發訊息**
- [ ] Codex `--resume` 取回 thread_id + 上下文
- [ ] Codex diary 提取出有意義內容（transcript 統一格式）
- [ ] Codex `_meta.session_id` 經 HTTP MCP 正確 resolve；`_meta` 存在但 resolve 失敗回硬錯
- [ ] Dual-client PoC：Claude + Codex agent 同時對同一 catclaw MCP endpoint 跑工具呼叫，Claude 走 legacy 路徑、Codex 走 codex 路徑，互不誤觸
- [ ] TUI `approvals.rs` 顯示三類 kind 並 round-trip
- [ ] LINE Flex 三套 layout + postback 全 round-trip
- [ ] `agents.delete` 期間有 codex 子程序 → 先 stop session 再清 symlink，無 zombie
- [ ] `codex_auth_path` per-agent override 通
- [ ] 併發測試：兩 codex agent 同時觸發 approval 不互卡
- [ ] `submit_for_approval_direct` 死鎖檢測通過
- [ ] codex `developer_instructions` thread-bound 行為驗證 ✓ (PoC 已驗，CI 防迴歸)
- [ ] **第三方 MCP server 注入**：user `.mcp.json` 加 pencil → codex `tools/list` 看得到 + 可呼叫
- [ ] **Streaming 降級**：Slack agent codex runtime 長對話一次性發送、不破訊息分塊
- [ ] **Diary 品質量測報告** ≥ Claude baseline 70%
- [ ] **Cross-runtime regression CI** 通過（結構性結果比對）
- [ ] **配置變更提示**：改 codex agent.model/system_prompt 後 TUI/CLI 提示 active thread 不會 pick up
- [ ] **三重 codex 原生工具警告**（TUI / CLI / TOML load）

### Phase F：完整文件 + 全矩陣
1. SKILL_CATCLAW + README 補齊
2. 四 surface × 兩 runtime × 三 ApprovalCard kind 全矩陣
3. Claude regression 全程確認

---

## 八、風險與應對

| 風險 | 應對 |
|---|---|
| `notifications/*` 改 204 破壞 Claude | Phase B.1 第一日驗；壞了暫回 200 + file Claude CLI bug（**不** user-agent sniffing） |
| auth.json symlink 被 codex rename 寫壞 | Phase B.1 PoC；壞了改 copy + 啟動前 sync |
| MCP tool name format 混亂 | 命名規範總表寫明；Codex 路徑用 (server, tool) 拼，不 blanket normalize |
| Codex 升版改 NDJSON / `_meta` schema | 寬鬆解析；CI PoC 腳本 |
| Codex 升版改 `developer_instructions` thread-bound 行為 | CI 加 PoC 驗證 |
| 多帳號 | `codex_auth_path` per-agent + preflight |
| Cross-runtime resume | `metadata.runtime` 比對，回友善錯誤 |
| Codex agent 不照 SKILL 走 shell curl 打 API | SKILL 寫清楚 + admin 監控 transcript |
| `approved` bool 被誤刪 wire format | 驗收清單明文要求 |
| `sessions.rs` 留死欄位 | 原子 commit 搬遷 + 驗收明文 |
| LINE postback 從零建 | B.1 PoC blocking 進度確認 |
| `_meta` 存在但 resolve 失敗 | 硬錯不 fallthrough（v3.2 安全強化） |
| 第三方 MCP server 工具誤路由 | 用 (server, tool) 兩欄拼 prefixed 名稱 |
| Codex `developer_instructions` 大幅膨脹 | thread-bound 行為實測，第一輪後不重送（PoC 確認） |
| 第三方 MCP server 對 codex 消失 | `inject_user_mcp_servers` 自動讀 `.mcp.json` 並轉換成 codex `-c` 旗標 |
| Codex 無 streaming → Slack 體驗降級 | 一次性發送，README documented 為已知 codex CLI 限制 |
| Diary extraction 對 codex transcript 品質下降 | Phase B 量測，必要時調 Haiku prompt |
| 用戶改 codex agent 配置不知道舊 thread 不吃新設定 | TUI/CLI 提示「new threads only」 |
| 用戶 CLI 設 `denied=["shell"]` 期待 codex 也擋 | 三重警告（TUI/CLI/TOML load） |
| 兩 runtime 行為差異被用戶誤判為 catclaw bug | README + SKILL 寫明「runtime 切換等同換 model」 |
| Claude 也送 `_meta` 導致誤入 Codex 路徑 | PoC 已驗證 Claude `_meta` 不含 `x-codex-turn-metadata` 子鍵；分流靠具體 key 不是 `_meta` 存在性（§5.2.1） |
| Codex 升版改 `_meta` 內 key 名稱 | CI PoC 監控 codex 對 catclaw MCP 的 raw request；schema 變動立即偵測 |

---

## 九、不做的事

- 不支援 codex user-level hooks
- 不支援 user `~/.codex/config.toml` 透明繼承
- 不做 codex TUI 模式
- 不重做 sandbox
- 不新增 `mcp__catclaw__shell` / `read_file` / `edit_file`
- 不攔截 codex 原生 shell tool（靠 SKILL）
- 不對 Claude transcript 偽造 ToolResult
- 不支援 cross-runtime session resume
- 不做 `catclaw mcp-bridge` stdio bridge（HTTP MCP 通）
- 不為「對抗惡意 agent」加 sandbox 業務 API 攔截
- 不支援 codex ephemeral session
- 不實作 MCP Streamable HTTP `mcp-session-id` header
- 不嘗試 codex resume 後重設 `developer_instructions`（thread-bound 不可變）
- 不對 codex 客戶端做 user-agent sniffing（用 `_meta` 存在性分流）
- 不在 `execute_social_tool` 內加 runtime 守衛（Codex 在更上層攔，Claude 路徑不動）
- **不支援 codex agent 跨 host 部署**：codex 子程序必須跟 catclaw gateway 在同一 host（127.0.0.1 MCP endpoint）。container 隔離 / multi-host / reverse proxy 場景需未來另案
- 不支援 codex agent 動態系統 prompt 注入（thread 首次後不可改）

---

## 十、驗收 checklist

統一在 §7 Phase B+D+E 與 Phase A 內列出。
