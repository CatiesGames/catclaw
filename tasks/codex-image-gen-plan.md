# Feature: Codex agent 圖片生成(image_gen / gpt-image-2)— 自動隨 codex runtime 啟用

## 需求(使用者最終定調)
「像 agent 自己的 tool 與 skill 一樣 —— **只要 agent 接了 codex 就自動載入、自動知道怎麼用**。」
→ 不要 per-agent 開關。codex runtime = 自動具備生圖能力。

## 本機實測結論(2026-05-20,codex-cli 0.130,ChatGPT 登入)
全部親自跑過 `codex exec --json`,非推測:

1. **`image_gen` 是 codex 內建 tool,ChatGPT 登入即可用**(免 OPENAI_API_KEY)。codex 自報:
   `image_gen.imagegen`(生成/編輯 raster)、`view_image`、`web.image_query`。
2. **codex 已內建官方 `imagegen` skill**(`$CODEX_HOME/skills/.system/imagegen/SKILL.md`)。
   模型依 description 自動觸發 —— 這就是「自動知道怎麼用」,**catclaw 不必自寫生圖教學**。
3. **built-in 圖檔預設落 `$CODEX_HOME/generated_images/`** → catclaw per-agent = `<ws>/.codex-home/generated_images/`。
   但 skill 強制規定:**project-bound 圖要 move/copy 進 workspace**(workflow #15)+ **務必回報最終路徑**(#18)。
4. **開法:`--enable image_generation`**(= `-c features.image_generation=true`)。catclaw `-c` inline
   注入不受 `--ignore-user-config` 影響(已驗證 model/approval 同模式)。
5. **`codex exec --json` 無專屬 image event** —— 生圖經 `command_execution`(codex 寫檔/呼叫)+
   `agent_message`(回報路徑)。catclaw `codex.rs` parser **已認得這兩種**,transcript 不丟資訊。
6. 模型會在最終訊息「report final saved path」(skill #18 強制)→ **agent 100% 拿得到路徑**,
   路線 A(agent 自己 upload_file)成立,**不需 ls fallback**。

## 順帶修復(已完成)
使用者本機 `~/.codex/config.toml` 的 `[mcp_servers.context7]` 同時有 `command/args`(stdio)
與 `url`(http),codex 0.130 報 `url is not supported for stdio`。已刪 command/args 留 url
→ `codex login status` 恢復正常(`Logged in using ChatGPT`)。此為使用者全域 config,與 catclaw
的 codex agent 隔離(後者用獨立 .codex-home + --ignore-user-config),不影響 catclaw 行為。

---

## 設計:自動隨 codex runtime 啟用(無開關)

**核心改動極小:codex agent 一律注入 `features.image_generation=true` + SKILL 教 upload 慣例。**

### 1. agent/codex_args.rs — 自動注入 feature flag(唯一的能力啟用點)
`codex_args_from`(sandbox 設定後,~codex_args.rs:95)無條件加:
```rust
// Image generation (gpt-image-2) — auto-on for every codex agent, mirroring
// how codex's built-in `imagegen` skill is always available. ChatGPT login
// covers it (no OPENAI_API_KEY needed). Output lands in
// .codex-home/generated_images/; the agent moves it into the workspace and
// uploads via {platform}_upload_file (see SKILL).
args.push("-c".to_string());
args.push("features.image_generation=true".to_string());
```
無 per-agent gate、無 runtime gate(此函式僅 codex 呼叫)。**這是唯一啟用能力的地方。**

### 2. SKILL — 教 catclaw 特有的「生圖後送 channel」慣例(codex 已會生圖,缺的是 catclaw 的回傳約定)
codex 原生 imagegen skill 教「怎麼生圖」,但**不知道 catclaw 的 channel/upload_file 慣例**。
catclaw 要補的只有「生完圖後怎麼送出去」這一段,放在 codex runtime 看得到的地方。

SKILL_CATCLAW 加一小節(codex 專屬,簡短):
```
## 圖片生成(codex runtime)
你有 codex 內建 `image_gen` tool + `imagegen` skill,可生成/編輯照片級圖片(免額外設定)。
生圖後要送到 channel 時:
1. 生成的圖預設落在 .codex-home/generated_images/;若要給使用者,先確認最終檔案路徑
   (imagegen skill 會回報 saved path,或自己把圖移進 workspace)。
2. 用 `{platform}_upload_file(file_path=<絕對路徑>, text="...")` 送到對話頻道
   (discord_upload_file / telegram_upload_file / slack_upload_file)。
3. 送完圖後,最終訊息回 NO_REPLY(避免又送一段重複文字),除非還有話要說。
若對象是 contact,圖一樣經平台 upload_file 送到對方;審批/forward 規則見 Contacts 章節。
```
注意:SYSTEM_DIRECTIVES 已有 NO_REPLY + Attachment Protocol 慣例,銜接它,別重複。
(評估:此段是否該只在 runtime=codex 時注入?build_system_prompt 已知 runtime,可條件注入,
避免 claude agent 看到不適用的內容 —— 實作時確認 build_system_prompt 結構。)

### 3. README.md — 文件同步
Codex runtime 章節補一句:「codex agent 自動具備 gpt-image-2 圖片生成(內建 image_gen tool +
imagegen skill,ChatGPT 登入即可用);生成的圖透過平台 upload_file 送到 channel。」

### 4. CLAUDE.md — 新增 lesson
記錄實測事實(image_gen 隨 codex runtime 自動可用、`-c features.image_generation=true` 注入、
圖落 .codex-home/generated_images/、回傳走路線 A、codex --json 無專屬 image event、
ChatGPT 登入免 OPENAI_API_KEY)+ context7 stdio/url 互斥的修復備忘。

---

## 不需要做的(相對舊 plan 砍掉)
- ❌ per-agent `codex_image_gen` 開關(config/Agent struct/loader/CLI/WS/TUI/reload)—— 使用者要自動啟用,全砍。
- ❌ 自寫生圖教學 —— codex 原生 imagegen skill 已涵蓋。
- ❌ 改 codex.rs parser / OutboundMessage / 回傳鏈 —— 路線 A 不需要。

## 驗證
- `cargo check` + `cargo clippy --all-targets -- -D warnings` 零警告。
- 實機:建/用 codex agent,從 channel 要一張照片級圖,確認 (a) 生圖 (b) agent upload_file (c) channel 收到圖。
  本機 codex 已驗證能生圖 + 回報路徑,風險低。

## 範圍外
- 路線 B(catclaw 自動偵測圖檔附加)、額度成本 UI、LINE 一般訊息送圖(附件機制不同)、
  CLI/API fallback 模式(gpt-image-1.5 透明背景等,codex skill 已支援,使用者要時 agent 自會走)。
