# LINE Adapter + Contacts / Forward & Approval 管線 — 設計藍圖

## 設計原則

CatClaw 只做「**通訊與身份**」基建，**不碰業務資料**。
個案的營養紀錄、健身數據、諮商筆記等業務資料，由 agent 自選工具（Notion MCP / Palace / 自管 SQLite）儲存，CatClaw 不選邊、不維護領域 schema。

每個專業領域（營養師、健身教練、心理師…）以**獨立 skill 包**承載業務邏輯,CatClaw 程式碼保持通用。

LINE adapter 為**選用功能**——使用者可決定是否啟用 LINE OA 串接;contacts 系統獨立於 LINE,既有 DC/TG/Slack 用戶可主動綁定享受同等功能。

## 分層職責

| 層 | 職責 | 位置 |
|---|---|---|
| 通訊層 | 收發訊息、Rich Menu、Forward、Approval 管線 | CatClaw 內建 |
| 身份層 | contacts (人 / role / tags)、平台綁定、外部系統 ref | CatClaw `contacts` 表 |
| 業務層 | 飲食紀錄、目標達成率、諮商筆記 | **Agent 自選**（Notion / Palace / 自管） |
| 記憶層 | 對話提煉、模糊搜尋 | Palace（CatClaw 內建,可選用） |

## 跨平台身份統一

contacts 機制天生跨平台:DC / TG / Slack / LINE 全部適用。

- **既有 DC/TG/Slack 用戶**:未綁 contact → role=unknown → router 行為與目前完全一致(零回歸)
- **漸進式啟用**:對特定用戶呼叫 `contacts_bind_channel` 即納入身份系統,享有 role/tags 注入、forward 鏡射、approval pipeline 全套
- **同一個小明可同時綁 LINE + DC + TG**:agent 視為同一人,回覆走 last_active 平台

---

## 一、資料模型(state.db schema 新增)

### 1.1 `contacts` — 身份核心

```sql
CREATE TABLE contacts (
    id                 TEXT PRIMARY KEY,         -- uuid
    agent_id           TEXT NOT NULL,            -- 哪個 agent 管的(v1 單一綁定;v2 將遷移至 contact_agents)
    display_name       TEXT NOT NULL,
    role               TEXT NOT NULL DEFAULT 'unknown',  -- admin | client | unknown
    tags               TEXT NOT NULL DEFAULT '[]',       -- JSON array of strings
    forward_channel    TEXT,                     -- e.g. "discord:guild123/channel456"
    approval_required  INTEGER NOT NULL DEFAULT 1,       -- 預設要過審(決策 1)
    ai_paused          INTEGER NOT NULL DEFAULT 0,       -- 暫停 AI,純人工接手(決策 2)
    external_ref       TEXT NOT NULL DEFAULT '{}',       -- agent 自由 JSON: {"notion_page": "...", ...}
    metadata           TEXT NOT NULL DEFAULT '{}',       -- 慢變 profile(agent 寫入;CatClaw 不解讀)
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE INDEX idx_contacts_agent ON contacts(agent_id);
CREATE INDEX idx_contacts_role  ON contacts(agent_id, role);
```

設計重點:
- `external_ref` 是 agent 與外部系統(Notion / Linear / 自管 DB)的橋接點,CatClaw 不解讀。
- `metadata` 與 `external_ref` 區分:`metadata` 是 agent 想就近放的小型結構(過敏源、目標等),`external_ref` 專門指向外部系統的 ID/URL。
- `role` 三選一固定 enum;細節用 `tags` 自由表達。
- **多 agent 擴充預備**:所有讀取 `contact.agent_id` 的地方走 helper `contact.owning_agents() -> Vec<AgentId>`,未來改 helper 內部實作即可,呼叫端零變動。

### 1.2 `contact_channels` — 平台綁定(多對多)

```sql
CREATE TABLE contact_channels (
    contact_id         TEXT NOT NULL,
    platform           TEXT NOT NULL,            -- "line" | "telegram" | "discord" | "slack"
    platform_user_id   TEXT NOT NULL,            -- LINE userId / TG user_id / Discord id
    is_primary         INTEGER NOT NULL DEFAULT 0,
    last_active_at     INTEGER,
    created_at         INTEGER NOT NULL,
    PRIMARY KEY (platform, platform_user_id),
    FOREIGN KEY (contact_id) REFERENCES contacts(id) ON DELETE CASCADE
);
CREATE INDEX idx_contact_channels_contact ON contact_channels(contact_id);
```

回覆策略:`contacts_reply` 預設用 `last_active_at` 最新者;可被 `via=` 參數覆寫。

### 1.3 `contact_drafts` — Outbound 審核管線

仿 `social_drafts`,但 owner 是 contact 而非 social platform:

```sql
CREATE TABLE contact_drafts (
    id              TEXT PRIMARY KEY,
    contact_id      TEXT NOT NULL,
    agent_id        TEXT NOT NULL,
    via_platform    TEXT,                        -- 指定發送平台;null = last_active 策略
    payload         TEXT NOT NULL,               -- JSON: {type: "text"|"image"|"flex", ...}
    status          TEXT NOT NULL,               -- pending | awaiting_approval | revising | sent | ignored | failed
    forward_ref     TEXT,                        -- 鏡射到管理頻道的 message ref(Discord msg id 等)
    revision_note   TEXT,                        -- 管理者要求 AI 重寫時的指示(決策 2)
    error           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    sent_at         INTEGER,
    FOREIGN KEY (contact_id) REFERENCES contacts(id) ON DELETE CASCADE
);
CREATE INDEX idx_contact_drafts_status ON contact_drafts(status, created_at);
```

---

## 二、Outbound Pipeline(Forward & Approval)

### 2.1 流程

```
agent 呼叫 contacts_reply(contact_id, payload, via?)
        ↓
[1] 建 contact_draft (status=pending)
        ↓
[2] 解析 contact 設定:
      - forward_channel  → 是否鏡射至管理頻道
      - approval_required → 是否需審核
      - ai_paused → 若為 true,直接拒絕(agent 應接收到 ai_paused 提示,不該再嘗試發送)
        ↓
[3] 若有 forward_channel:
      在管理頻道發 work card(payload 預覽 + 完整操作元件)
      存 message ref 到 forward_ref
        ↓
[4] 分流:
      approval_required = false → 立刻送出(→ step 5)
      approval_required = true  → 等管理者操作 → approve/edit/discard/request_revision
        ↓
[5] 透過 channel adapter 發送(依 via 或 last_active 選平台)
        ↓
[6] 更新 status=sent / failed,更新 forward card 顯示結果
```

### 2.2 關鍵設計

- **agent 不准繞過 pipeline**:channel adapter 的 raw `send()` 不開放給 agent MCP;agent 只能透過 `contacts_reply` 發訊。確保審核一致性。
- **無限期等審核**:預設無 timeout;可在 contact 設 `approval_timeout_secs`(後續迭代)。
- **失敗自癒**:forward card edit 失敗 → 重發新 card 並更新 `forward_ref`,仿 social `forward::ensure_inbox_card_restored()`(CLAUDE.md lesson 15)。

---

## 三、管理頻道工作台(決策 2 核心設計)

管理頻道(如 Discord `#個案-小明`)不只是「看」,而是**完整工作介面**。

### 3.1 雙向鏡射內容

| 訊息類型 | 鏡射內容 |
|---|---|
| 個案入站訊息 | 文字 + 媒體完整鏡射(無按鈕,純通知) |
| Agent 出站草稿 | Work Card(含預覽 + 操作元件) |
| 出站發送結果 | 更新原 Work Card(顯示已發送 / 失敗) |
| Contact 狀態變更 | 系統訊息(role 變更、ai_paused 切換等) |

### 3.2 Work Card 操作元件

每個草稿在管理頻道呈現為 work card,提供:

| 操作 | UI 元件 | 後端動作 |
|---|---|---|
| 核准送出 | Button: ✅ Approve | `contacts_draft_approve(id)` |
| 修改後送出 | Button: ✏️ Edit → opens Modal | `contacts_draft_edit(id, new_payload)` → auto approve |
| 丟棄 | Button: 🗑 Discard | `contacts_draft_discard(id)` |
| 要求 AI 重寫 | Button: 🔄 Revise → opens Modal(輸入指示) | `contacts_draft_request_revision(id, note)` → 退回 agent |
| 暫停 AI | Button: ⏸ Pause AI | `contacts_ai_pause(contact_id)` |
| 恢復 AI | Button: ▶ Resume AI | `contacts_ai_resume(contact_id)` |

平台元件對照:
- **Discord**:Button + Modal(text input)
- **Slack**:Block Kit Button + Modal(view.open)
- **Telegram**:InlineKeyboardButton + 對話 state(收集 revision note)
- **LINE**:Flex Message + Postback(LINE 端為次要管道,主要在 DC/Slack 操作)

### 3.3 手動回覆模式

當管理者**直接在管理頻道打字**(非按鈕操作),應視為以 agent 名義回覆個案:

```
管理頻道入站訊息進 Router
    ↓
Router 檢測:此頻道是否為某 contact 的 forward_channel?
    ↓ 是
判斷訊息是否為按鈕互動 / bot 自身訊息 → 跳過
    ↓ 純文字/媒體
解析為「手動回覆」,直接走 outbound pipeline:
   - 建 contact_draft (sender=human, status=pending)
   - 若 approval_required=false → 立即送
   - 若 approval_required=true  → 仍需另一管理者(或自己)按核准(可選擇豁免)
    ↓
原管理頻道訊息可選擇自動加 reaction(✓)表示已轉送
```

設計要點:
- **手動回覆不派給 agent 處理**(否則會被 agent 改寫或回應)
- **ai_paused=true 時**,個案的訊息**完全不派給 agent**,只鏡射到管理頻道,等管理者手動回覆
- **ai_paused=false 時**,管理者仍可隨時手動插話,agent 與人類交替回覆都允許

### 3.4 Request Revision 流程

```
管理者按 🔄 Revise → 輸入「再溫和一點,提到他上週的進步」
    ↓
contacts_draft_request_revision(draft_id, note)
    ↓
draft.status = revising
    ↓
系統將原 draft 內容 + revision_note 作為新訊息推回 agent session:
   "[管理者要求重寫剛才給 王小明 的回覆,原文:...,指示:再溫和一點...]"
    ↓
Agent 重新生成 → 再次呼叫 contacts_reply → 走標準流程
    ↓
管理頻道更新原 work card 為「已要求重寫」,新 draft 送出新 card
```

### 3.5 對應 WS / MCP 方法新增

WS 方法:
- `contact.draft.edit(id, payload)`
- `contact.draft.request_revision(id, note)`
- `contact.ai_pause(contact_id)`
- `contact.ai_resume(contact_id)`

MCP tools(供 agent 自我管理):
- `contacts_ai_pause(id)` / `contacts_ai_resume(id)` — agent 也可主動暫停自己(例如判斷需要人工介入)

---

## 四、Inbound 鏡射

入站訊息流程(`src/router.rs` 注入點):

```
Channel Adapter 收訊
    ↓
MsgContext 進 Router
    ↓
Router 查 contact_channels → 找到 contact
    ↓
[A] 若 contact.forward_channel 有設:
      把訊息(含媒體)鏡射到管理頻道(純通知,無按鈕)
[B] 若 contact.ai_paused = true:
      ⛔ 不派給 agent,流程結束
    否則:
      把 contact 資訊塞進 system prompt 前綴:
        "[Contact: 王小明, role=client, tags=[糖尿病]]"
        "[external_ref: {...}]" (agent 自己解讀)
      照常派送給 agent
```

未綁 contact 的 sender → role=unknown,行為與目前一致(**對既有 Discord 用戶零影響**)。

---

## 五、MCP Tools(agent 操作介面)

### 5.1 Contacts(平台無關)

平台無關命名,全部加在 `mcp__catclaw__contacts_*`:

| Tool | 說明 |
|---|---|
| `contacts_create(name, role?, tags?, approval_required?)` | 建立 contact,回 id |
| `contacts_update(id, {role?, tags?, forward_channel?, approval_required?, metadata?, external_ref?})` | 部分更新 |
| `contacts_get(id_or_platform_user_id)` | 取單一 contact(含 channels) |
| `contacts_list(filter)` | 列表,支援 role/tag/agent filter |
| `contacts_bind_channel(id, platform, platform_user_id, is_primary?)` | 綁定平台帳號 |
| `contacts_unbind_channel(platform, platform_user_id)` | 解綁 |
| `contacts_reply(id, payload, via?)` | **唯一出口**,走 outbound pipeline |
| `contacts_ai_pause(id)` | 暫停 AI 自動回覆 |
| `contacts_ai_resume(id)` | 恢復 AI 自動回覆 |
| `contacts_drafts_list(filter)` | 列待審草稿 |
| `contacts_draft_approve(draft_id)` | 核准送出 |
| `contacts_draft_discard(draft_id)` | 丟棄 |

`contacts_reply` 的 `payload` 接 typed enum:`{type: "text", text: "..."}` / `{type: "image", url: "..."}` / `{type: "flex", contents: {...}}`。LINE 專屬格式(Flex)由 LINE adapter 負責序列化,跨平台訊息(text/image)透明轉換;不支援的 payload 在該平台返回 error。

### 5.2 LINE 專屬 Tools(決策 4)

LINE Rich Menu 是 LINE 專屬概念,命名為 `line_*`,**完全交給 agent 管理**:

| Tool | 說明 |
|---|---|
| `line_rich_menu_create(name, size, areas, chat_bar_text?)` | 建立 menu(回 menu_id) |
| `line_rich_menu_upload_image(menu_id, image_path_or_url)` | 上傳 menu 背景圖 |
| `line_rich_menu_list()` | 列出所有 menu |
| `line_rich_menu_delete(menu_id)` | 刪除 menu |
| `line_rich_menu_set_default(menu_id)` | 設為 OA 預設 menu(未綁定用戶看到的) |
| `line_rich_menu_link_user(menu_id, line_user_id)` | 套用至特定用戶 |
| `line_rich_menu_unlink_user(line_user_id)` | 取消用戶 menu |
| `line_get_quota()` | 查 push 配額 |
| `line_get_profile(line_user_id)` | 查 LINE 用戶 profile |

**CatClaw 不維護 role → menu 對應**。Agent 自行決定怎麼存(塞 `contacts.metadata`、`external_ref`、自己的 Notion 都行),自行決定何時呼叫 `line_rich_menu_link_user`。

工作流程範例:
> 營養師:「幫我做兩個 rich menu,一個給我(管理選單),一個給個案」
> agent:呼叫 `line_rich_menu_create` × 2 → 上傳圖 → 記住兩個 menu_id → 之後看到 contact role 變化主動呼叫 `line_rich_menu_link_user`

---

## 六、CLI(使用者操作介面)

仿現有 `catclaw social` / `catclaw agent` 風格:

```
catclaw contact add <name> [--role admin|client] [--tag ...] [--no-approval]
catclaw contact list [--agent ID] [--role ...]
catclaw contact show <id>
catclaw contact update <id> [--role ...] [--forward-channel ...] [--approval] [--no-approval]
catclaw contact bind <id> --platform line --user-id U123...
catclaw contact unbind --platform line --user-id U123...
catclaw contact pause <id>
catclaw contact resume <id>
catclaw contact draft list [--status ...]
catclaw contact draft approve <draft_id>
catclaw contact draft discard <draft_id>
```

**`contacts_reply` 不出 CLI**——使用者不會在終端打字回覆個案。

---

## 七、TUI(使用者操作介面)

新增 `src/tui/contacts.rs` 面板:

- 列表:依 agent 分組,欄位 name / role / tags / 平台數 / approval / forward channel / ai_paused 狀態
- 詳細頁:可編輯 role/tags/forward_channel/approval_required/ai_paused
- 子分頁「Drafts」:仿 social_drafts 面板,列待審草稿、approve / edit / discard / request_revision

---

## 八、LINE Adapter(`src/channel/line.rs`)

### 8.1 對標 Telegram,採 webhook 模式

依賴:`reqwest`(已有)+ `axum`(已有,與 social webhook 共用 runtime)。
不引入 LINE SDK;自建薄 client(避免 serenity/teloxide 級重依賴)。

### 8.2 Config 擴充

`src/config.rs` `ChannelConfig` 新增:

```toml
[[channels]]
type = "line"
token_env = "LINE_CHANNEL_ACCESS_TOKEN"
secret_env = "LINE_CHANNEL_SECRET"

[channels.line.webhook]
path = "/webhook/line"        # 與 social webhook 共用 axum
```

**移除 rich menu 相關 config**(決策 4 — agent 自管)。

### 8.3 ChannelAdapter 實作要點

- **入站**:webhook handler → HMAC-SHA256 驗章(用 `secret_env`) → 解析 events → 轉 `MsgContext`(`channel_type="line"`, `peer_id=userId`, `channel_id=groupId|roomId|userId`)
- **出站**:優先用 reply token(5 分鐘內免費),過期 fallback 到 push API
- **媒體**:圖片走 LINE Content API 下載 → 存暫存檔 → 塞進 `MsgContext.attachments`
- **Approval 按鈕**:Flex Message + Postback Action(管理者多用 Discord 鏡射頻道審核,LINE 端按鈕作為次要管道)
- **Follow / Unfollow**:webhook event 進 router;預設不自動建 contact(避免被陌生人灌爆),由 agent 透過 `contacts_create` + `contacts_bind_channel` 主動建立
- **Capabilities**:`streaming=false`、`message_editing=false`、`max_message_length=5000`

### 8.4 Execute Actions

LINE 專屬操作走 MCP tools(見 5.2),Adapter 內部 `execute()` 不對 agent 開放。

---

## 九、新增/修改檔案總覽

| 檔案 | 動作 | 預估行數 |
|---|---|---|
| `src/contacts/mod.rs` | 新增 — 核心型別 + CRUD + owning_agents helper | 450 |
| `src/contacts/pipeline.rs` | 新增 — outbound pipeline (forward + approval + revision) | 450 |
| `src/contacts/tools.rs` | 新增 — `contacts_*` MCP tool schemas + dispatch | 450 |
| `src/contacts/forward.rs` | 新增 — Work Card renderer(共用 helper 提至 social/forward) | 350 |
| `src/contacts/manual_reply.rs` | 新增 — 手動回覆模式偵測與轉送 | 150 |
| `src/state.rs` | 修改 — 新增三張表 schema + helper | +180 |
| `src/router.rs` | 修改 — inbound 注入 contact context + 鏡射 + ai_paused 判斷 + 手動回覆檢測 | +100 |
| `src/channel/line.rs` | 新增 — LINE adapter 全套 | 1100 |
| `src/channel/line_rich_menu.rs` | 新增 — LINE Rich Menu API client + MCP tools | 350 |
| `src/channel/mod.rs` | 修改 — 新增 capability flag(若需要) | +20 |
| `src/config.rs` | 修改 — LINE channel config + secret_env | +50 |
| `src/gateway.rs` | 修改 — line adapter 啟動分支、webhook 掛載 | +40 |
| `src/ws_server.rs` | 修改 — 新增 `contact.*` WS methods、MCP tool 註冊 | +280 |
| `src/main.rs` | 修改 — `catclaw contact ...` 子命令 | +280 |
| `src/tui/contacts.rs` | 新增 — Contacts panel + Drafts 子分頁 | 450 |
| `src/tui/mod.rs` | 修改 — 註冊新 panel + 導航 | +30 |
| `src/agent/loader.rs` | 修改 — `SKILL_CATCLAW` 新增 contacts + line 章節 | +180 |
| `README.md` | 修改 — Contacts / LINE 章節 | +200 |

---

## 十、實作順序(建議)

1. **schema + contacts CRUD + owning_agents helper**(資料層基礎)
   - state.db migrations
   - `src/contacts/mod.rs` 基本 CRUD
   - **owning_agents helper 從一開始就抽象**(為多 agent 擴充預備)
   - 不接 MCP / CLI / TUI,純資料層測試
2. **MCP tools + CLI 基本盤**
   - `contacts_create / get / list / update / bind / pause / resume`
   - 同步 catclaw skill
3. **Outbound pipeline + 完整 Work Card 操作**(用 Discord 測)
   - `contacts_reply` → draft → 透過現有 Discord adapter 發
   - forward_channel 鏡射(雙向)
   - Work Card:approve / edit / discard / request_revision / ai_pause/resume
   - 手動回覆模式偵測與轉送
   - 用 Discord 自己當「個案平台」+ 另一個 Discord 頻道當「管理頻道」做端對端測試
4. **LINE adapter MVP**
   - webhook + 文字收發 + 簽章驗證
   - bind LINE userId → contact
   - 接上 Outbound pipeline
5. **LINE 進階**
   - 圖片收發
   - Rich Menu 全套 MCP tools(`line_rich_menu_*`)
   - Flex Message approval card
   - follow/unfollow event
6. **TUI Contacts panel + Drafts 子分頁**
7. **CLAUDE.md 更新 + skill 完整版 + README**

每階段獨立可測、可 ship。

---

## 十一、與既有系統的關係

- **Social Inbox(`src/social/`)**:`forward::` 模組可抽共用 helper(card renderer / ensure_restored / notify_admin),但 contacts 的 outbound pipeline **獨立於 social**——social 是「外部來訊 → 草稿 → 平台」,contacts 是「agent → 草稿 → 平台」,方向相反,不強行合併。
- **Memory Palace**:contact 對話可選擇性 `wing=contact_id` 做 per-contact 記憶隔離。屬於 agent 自選,CatClaw 不強制。
- **AdapterFilter / Bindings**:LINE adapter 走相同機制,無特殊處理。
- **既有 Discord 使用者**:未綁 contact → role=unknown → router 行為與目前完全一致,**零回歸風險**。

---

## 十二、營養師 Skill 包(範例,非 CatClaw 程式碼)

CatClaw 不維護領域邏輯,但會提供範例 skill 包鼓勵生態:

```
skills/nutritionist/
  SKILL.md                  使用情境、流程、工具用法(含 contacts + line_rich_menu 用法)
  prompts/
    food_image_analysis.md  分析食物照片的 prompt
    daily_summary.md        產出每日報表
    rich_menu_setup.md      指導 agent 如何用 line_rich_menu_* 建管理/個案兩套 menu
  notion_template.md        建議的 Notion database 結構
```

營養師裝 CatClaw + Notion MCP + 此 skill 包即可開工,無需改 CatClaw 一行程式碼。

健身教練、心理師可 fork 此 skill 包改 50 行得到自己版本。**這才是真正的通用**。

---

## 十三、多 Agent 共享 Contact 的擴充路徑(v1 → v2)

v1 採單一綁定(`contacts.agent_id`),為 v2 預留以下平滑擴充能力:

### 13.1 v1 預備動作(必做)

- 所有讀取 `contact.agent_id` 的程式碼集中走 helper:
  ```rust
  impl Contact {
      pub fn owning_agents(&self) -> Vec<AgentId> {
          vec![self.agent_id.clone()]   // v1: 永遠回 1 筆
      }
  }
  ```
- 不在 SQL 寫死 `WHERE agent_id = ?`,全走 query helper
- Router 派送邏輯預留多 agent 分流接口(即使 v1 永遠只有 1 個)

### 13.2 v2 遷移步驟

```sql
CREATE TABLE contact_agents (
    contact_id TEXT NOT NULL,
    agent_id   TEXT NOT NULL,
    role       TEXT,                    -- 該 agent 對此 contact 的角色(可選)
    PRIMARY KEY (contact_id, agent_id)
);
INSERT INTO contact_agents(contact_id, agent_id)
  SELECT id, agent_id FROM contacts;
-- contacts.agent_id 保留為 primary_agent_id,或 drop
```

對應改動:
- `owning_agents()` 改 join `contact_agents` 回多筆
- 新增 MCP tools:`contacts_attach_agent(id, agent_id)` / `contacts_detach_agent(id, agent_id)`
- Router 派送邏輯處理多 agent 分流(round-robin / 主 agent / 全部派發,依需求)

**結論**:v1 設計已為 v2 鋪路,**遷移成本低**。

---

## 十四、工程量估算

| 階段 | 預估 |
|---|---|
| 1. schema + CRUD + owning_agents helper | 0.5 天 |
| 2. MCP tools + CLI | 0.5 天 |
| 3. Outbound pipeline + 完整 Work Card 操作 + 手動回覆 | **2 天** |
| 4. LINE adapter MVP | 1 天 |
| 5. LINE 進階(圖片 / Rich Menu MCP / Flex / follow) | 1.5 天 |
| 6. TUI panel + Drafts 子分頁 | 0.5 天 |
| 7. 文件 / skill / README | 0.5 天 |
| **總計** | **6.5 個工作天** |

(階段 3 與階段 5 因決策 2、4 而擴大)

---

## 十五、決策紀錄(2026-04-19)

| # | 決策 | 結論 |
|---|---|---|
| 1 | `approval_required` 預設值 | **預設 true**(安全優先,新 contact 一律先過審) |
| 2 | Forward 鏡射方向與操作 | **雙向鏡射**,管理頻道為完整工作台(approve/edit/discard/revise/pause/manual reply) |
| 3 | LINE OA 申請狀態 | 已有 OA + 正式網域;LINE 為**選用功能**(未啟用對既有功能零影響) |
| 4 | Rich Menu 管理 | **完全由 agent 管理**,提供 `line_rich_menu_*` MCP tools,CatClaw 不維護 role↔menu 對應 |
| 5 | 多 agent 共享 contact | **v1 單一綁定**,但透過 `owning_agents()` helper 預備 v2 平滑擴充 |
