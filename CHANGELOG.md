# Changelog

## [0.40.3](https://github.com/CatiesGames/catclaw/compare/v0.40.2...v0.40.3) (2026-04-14)


### Bug Fixes

* backend channel requires explicit binding, skip empty config fields ([0457f12](https://github.com/CatiesGames/catclaw/commit/0457f12ffbe00ed31629823b5269f2a50b46acea))

## [0.40.2](https://github.com/CatiesGames/catclaw/compare/v0.40.1...v0.40.2) (2026-04-14)


### Bug Fixes

* backend adapter token_env supports direct secret value ([e351152](https://github.com/CatiesGames/catclaw/commit/e3511520f7cb91774d29a09af252c63adf84dc43))

## [0.40.1](https://github.com/CatiesGames/catclaw/compare/v0.40.0...v0.40.1) (2026-04-14)


### Bug Fixes

* don't crash gateway when backend adapter config is missing ([3bdc034](https://github.com/CatiesGames/catclaw/commit/3bdc034b852bec4e0a6dc31a3b620d3104900a8f))

## [0.40.0](https://github.com/CatiesGames/catclaw/compare/v0.39.0...v0.40.0) (2026-04-14)


### Features

* **channel:** add backend channel adapter for multi-tenant agent routing ([e56f997](https://github.com/CatiesGames/catclaw/commit/e56f997df745bca40873ba07b36e5490ce2bcc70))

## [0.39.0](https://github.com/CatiesGames/catclaw/compare/v0.38.1...v0.39.0) (2026-04-14)


### Features

* [env] 子進程環境變數 + 社群發文審核流程改進 ([30f09e9](https://github.com/CatiesGames/catclaw/commit/30f09e97c0276923362b1719254df1e58cdefae5))
* add injection-guard built-in skill ([f263ad3](https://github.com/CatiesGames/catclaw/commit/f263ad323371fc74a9a7436e5e5875f7d1316dd8))
* catclaw memory status/reset/remigrate + update --version + system session 不寫 transcript ([f2ccafa](https://github.com/CatiesGames/catclaw/commit/f2ccafa2e8216b0fd37868fedefef9b60a156d1a))
* catclaw onboard replaces init, add Chinese README ([a8f1790](https://github.com/CatiesGames/catclaw/commit/a8f1790c01b1c8c2740793acc330060bc75d0582))
* config set token 後自動 exchange + TUI 顯示 token 到期日 ([73c9aeb](https://github.com/CatiesGames/catclaw/commit/73c9aebc80a8be11c5288dfdf18fcfb227ae0506))
* Discord reaction 狀態指示器 + 修復空 result 導致 session 斷裂 ([67ecc4c](https://github.com/CatiesGames/catclaw/commit/67ecc4c86e0cd4a3b98c9451207d89af3c2acd19))
* Discord/Telegram reply 時 agent 可看到被回覆的原文 ([e7e1821](https://github.com/CatiesGames/catclaw/commit/e7e182117834b3339ab6e2a91b38cc75179ef943))
* Discord/Telegram slash commands (/stop, /new) + 統一 diary extraction ([cb4cf9c](https://github.com/CatiesGames/catclaw/commit/cb4cf9cc811fb3b60f3882283f41ce5f4494c2a7))
* distribution, approval UX, attachment handling ([594a58e](https://github.com/CatiesGames/catclaw/commit/594a58ec949856bbf41ac5a461647ea9c34d0807))
* human-readable transcript filenames ([83ea1d4](https://github.com/CatiesGames/catclaw/commit/83ea1d406ed2268b742997a71024b98bc2faecc6))
* IG 發文/私訊、Threads 關鍵字搜尋、webhook 補齊、task get、config 預設值 ([751863a](https://github.com/CatiesGames/catclaw/commit/751863ab49a2b3848d9d6e98f1c9c1834298d9f4))
* IG/Threads carousel（多圖輪播）發文支援 + 修復草稿審核流程 ([a213648](https://github.com/CatiesGames/catclaw/commit/a2136485302fbf69277de86e2ff85c9f66b7ddc9))
* IG/Threads token 自動換長效 + 定期續期，修正 Issues panel，改善 debug 日誌 ([f2f9845](https://github.com/CatiesGames/catclaw/commit/f2f98456fe1903e57c403250a30e411f63a42f31))
* inject session context header into every agent message ([4ac1d3c](https://github.com/CatiesGames/catclaw/commit/4ac1d3c1c141cffab3c236b2ffa5d3b6f85c1a73))
* issue tracking + TUI Issues panel + Instagram/Threads MCP tools 顯示 ([065465b](https://github.com/CatiesGames/catclaw/commit/065465b6f315578a80ba7fe9c23b59cce673a734))
* local timezone display, task name lookup, one-shot auto-delete ([326ae9c](https://github.com/CatiesGames/catclaw/commit/326ae9c5961eb257dda8a027d361427268c9b4c6))
* macOS binary 用 Developer ID 簽名解決 TCC 權限彈窗 ([e15230f](https://github.com/CatiesGames/catclaw/commit/e15230ff81c2daf48c9299a3408675d2f46a951a))
* MCP env 管理 + MCP tools 自動探測 ([cbbc0f2](https://github.com/CatiesGames/catclaw/commit/cbbc0f2310c8b85287f36e5c807d5f3536b89370))
* poller debug log + TUI cursor 顯示 + 回覆卡原文 API fallback ([0782509](https://github.com/CatiesGames/catclaw/commit/07825095ba57ea8b2c93c327819405ef24c87ade))
* Slack channel adapter（Socket Mode + AI streaming） ([31d8e27](https://github.com/CatiesGames/catclaw/commit/31d8e27d30f7f2cd32264752ae0a5d2faab48143))
* Slack reaction 狀態指示 + 日記 timezone 修正 ([7a21c66](https://github.com/CatiesGames/catclaw/commit/7a21c6645a74a4f4df306e4bc184fed78d4ac2e1))
* Social Inbox — Instagram + Threads 整合 ([26a7627](https://github.com/CatiesGames/catclaw/commit/26a76277bccf21548978fd9a66e8e28857c4cda8))
* Social Inbox 全設定可透過 TUI/CLI 設定 ([660318c](https://github.com/CatiesGames/catclaw/commit/660318c2c9910cc472f1d1d42dafd85177af7bcf))
* social_drafts 系統 + 核准流程改造 + TUI Drafts 面板 ([b889ba9](https://github.com/CatiesGames/catclaw/commit/b889ba9d8b595da432a1811ebbe3c8ab8ebf3363))
* **social:** 統一 reply 卡片生命週期 + 修復 AI 重試殭屍 draft ([41a3f7b](https://github.com/CatiesGames/catclaw/commit/41a3f7b3a6d86649cfb1918e7e247b7544cc0c8c))
* task add --at 一次性排程 + agent scheduling 指引 ([2f00410](https://github.com/CatiesGames/catclaw/commit/2f00410e7202556e37860cacbe64e37bef248ca7))
* timezone 設定 + Skill tool 支援 + approval 說明修正 ([9ad0559](https://github.com/CatiesGames/catclaw/commit/9ad05591d35e75a3e84ba0e72babbfc86044eeaa))
* tool approval system + channel forwarding + TUI/CLI improvements ([b51b893](https://github.com/CatiesGames/catclaw/commit/b51b89303e8debcfd917e04b7364bf9b9b282c94))
* TUI Agents 加滾輪 + PageUp/Down/Home/End/g/G 導航 ([d8badbc](https://github.com/CatiesGames/catclaw/commit/d8badbc08b2ee84c530f61d06d2c02e0ba381004))
* TUI Skills 清單顯示 built-in 標記 ([a71c318](https://github.com/CatiesGames/catclaw/commit/a71c3188238647a287d8e5def3da62ff10fced31))
* unified tool permissions, MCP management, mouse scroll, UX improvements ([a862792](https://github.com/CatiesGames/catclaw/commit/a862792965812a7c64c4b2a47d45a62cc051b1f7))
* unify all runtime files under ~/.catclaw/ ([b9bc44e](https://github.com/CatiesGames/catclaw/commit/b9bc44ed8b725d81298d956c2c958a111b8ec302))
* update --notify + timezone 修正 + system session transcript ([77e5bdb](https://github.com/CatiesGames/catclaw/commit/77e5bdb14a021a8cdd3362233d3401e9827623be))
* upload_media 支援批次上傳 + 補齊 skill 多圖操作步驟 ([1a673fe](https://github.com/CatiesGames/catclaw/commit/1a673fe1c1366d32cf6bc965a80dcaa8895a56c4))
* 三平台 upload_file MCP action（Slack/Discord/Telegram） ([01ad6a6](https://github.com/CatiesGames/catclaw/commit/01ad6a6e946580d7f4615e000de11e835c07caa2))
* 排程任務預設每次開新 session，避免 context 污染 ([b495efd](https://github.com/CatiesGames/catclaw/commit/b495efde598a0d6bc3b5cf3439acf4b966e69c74))
* 新增 /health 端點 ([ea23da5](https://github.com/CatiesGames/catclaw/commit/ea23da5601bdbb799ceade11c937c8858ab6a3df))
* 發文失敗卡片顯示紅色 + 重試/捨棄按鈕（全平台一致） ([a80689b](https://github.com/CatiesGames/catclaw/commit/a80689bc827bd55cbb9f3190c8d3b5d21b806b03))
* 發文審核卡片狀態流轉 — 核准→發送中(橘)→成功(綠)/失敗(紅+重試) ([414d2ca](https://github.com/CatiesGames/catclaw/commit/414d2cad7cf2385d67ee9480a3687759083e8428))
* 社群 inbox 卡片增加「查看原文」按鈕，AI 回覆自動抓原文 context ([788af3e](https://github.com/CatiesGames/catclaw/commit/788af3eb0e111561bf5cbcd99bcf151b652eb66e))
* 社群卡片按鈕互動全面優化 + 建議 AI 回覆 ([7ea612e](https://github.com/CatiesGames/catclaw/commit/7ea612e34653c42a9fa13023bfba8745f2244e7c))
* 社群卡片與 TUI 顯示 platform_id（Threads/IG 原生 ID） ([1a0a6d2](https://github.com/CatiesGames/catclaw/commit/1a0a6d2b2a6e1bf7757362a5b87f6c155ef3753d))
* 社群圖片上傳自動轉換格式 ([f723afe](https://github.com/CatiesGames/catclaw/commit/f723afe764c9591c646752f37718efb1b7d5eb35))
* 自動記憶系統 — 日記提取與長期蒸餾 ([ad77581](https://github.com/CatiesGames/catclaw/commit/ad77581bb6b5ba6201d6696c13a9d6f63b59276a))
* 記憶宮殿系統 (MemPalace) — 取代 markdown 記憶機制 ([d4589a2](https://github.com/CatiesGames/catclaw/commit/d4589a28146e6174e20d4fafbafe797368f521a5))
* 記憶系統可針對指定 agent 關閉 ([f73a624](https://github.com/CatiesGames/catclaw/commit/f73a6243e7a56d1134e90e20897ecd603ba13bef))


### Bug Fixes

* aarch64-linux 改用 native ARM runner 避免 cross-compile ONNX 問題 ([dfc652d](https://github.com/CatiesGames/catclaw/commit/dfc652dc4504965e29d3bc9fff1662e9e4a54c46))
* agent 不用 MCP 回覆對話 + DM thread_ts 統一過濾 ([bc7db00](https://github.com/CatiesGames/catclaw/commit/bc7db0098968e1e42bf67f08f82e336c67d520ac))
* AI 回覆失敗時還原 forward card 讓使用者可重試 ([ec931b8](https://github.com/CatiesGames/catclaw/commit/ec931b8f1b7815bc5aeb4028bcc2a7898c6c8683))
* approval 發到正確 thread + 點擊後更新卡片 + 多項修復 ([9638e44](https://github.com/CatiesGames/catclaw/commit/9638e4436d3185b6f74580c292b74201d405d24c))
* auto_reply prompt 用 MCP tool 全名 + 強制呼叫 ([f3dec9f](https://github.com/CatiesGames/catclaw/commit/f3dec9f481aac8c2a2f7c067cf253015279fd26f))
* backfill 不分析 extraction nodes 避免無限循環 ([f917234](https://github.com/CatiesGames/catclaw/commit/f917234412fac1718f57b0f0f0aff5f851628418))
* backfill 分析失敗時標記 empty summary 避免無限重試 ([c7d1639](https://github.com/CatiesGames/catclaw/commit/c7d163902faef116a47dc5646730eb54cbf03339))
* backfill 加 10s 間隔 + 連續失敗暫停 + 區分錯誤類型 ([097d5d6](https://github.com/CatiesGames/catclaw/commit/097d5d69305a4a1f9d43cef8fc67705490ec8bf7))
* BGE-M3 下載失敗 — hf-hub 改回 native-tls ([7a1d9ae](https://github.com/CatiesGames/catclaw/commit/7a1d9aee5196af3ec710324d03902a47f54ea8d2))
* BGE-M3 模型 cache 改用絕對路徑 ~/.catclaw/models/ ([05d8b43](https://github.com/CatiesGames/catclaw/commit/05d8b436f0f3adc2f6952b1d2d13db955ee33637))
* BOOT.md 分支補上 active_handles 註冊，/stop 不再失效 ([40aa242](https://github.com/CatiesGames/catclaw/commit/40aa242faa4b0ec09cb3dbc4e33a8c3f85db563a))
* built-in skills 每次啟動覆寫為最新版 ([65beea5](https://github.com/CatiesGames/catclaw/commit/65beea57106ed0296add8f5a9cde3bf10997ad55))
* catclaw onboard 重複執行時進入 wizard 而非跳過；Config panel 選擇時自動捲動 ([9b65907](https://github.com/CatiesGames/catclaw/commit/9b659074bfc4abeb94105bdd6cedd3d5ae8f8808))
* catclaw skill 強調 self-update 必須用 --notify ([c2a1401](https://github.com/CatiesGames/catclaw/commit/c2a14016a207c8b45931302fd22ad1be8aca83d6))
* clippy unnecessary_unwrap in task add schedule display ([3bfc1e1](https://github.com/CatiesGames/catclaw/commit/3bfc1e19a935e6a06bfce8d92e1bc5f587ed3ea0))
* code review 第二輪修復 + TUI transcript 讀取 bug ([9ccf272](https://github.com/CatiesGames/catclaw/commit/9ccf272b12b2cca1765b197dda07d65e62e734ce))
* cron 任務明確標示 UTC，避免 agent 時區誤解 ([46bd1cf](https://github.com/CatiesGames/catclaw/commit/46bd1cfde4be6fa4f548bfe8b1e6db8a5a369160))
* diary subprocess 隔離所有工具避免 max-turns 超限 ([539ce8b](https://github.com/CatiesGames/catclaw/commit/539ce8b8ff7cfabdcc836f5719f39293e03a21e3))
* diary 生成時 MCP 配置傳入正確的 mcpServers 結構 ([3ac79f2](https://github.com/CatiesGames/catclaw/commit/3ac79f2097ad9a1df8dbf5f95c5913b0408d4db6))
* Discord reaction 更絲滑 — 先加後移 + terminal 直接移除 ([4ffac2f](https://github.com/CatiesGames/catclaw/commit/4ffac2fdc65d5b79a876308de38e024c49494908))
* Discord thread 建立事件誤觸發主頻道回覆 + thread 偵測邏輯修正 ([e6385c5](https://github.com/CatiesGames/catclaw/commit/e6385c5582e9446d86365c01e8ca434d0a23b9a5))
* Discord 審核卡片顯示附圖 + post 類型隱藏 From + TUI draft 欄寬調整 ([c9f3c74](https://github.com/CatiesGames/catclaw/commit/c9f3c7488e3d741567410cea8bd35c7534147aa7))
* draft 審核日誌 + poller messages 靜默跳過 + IG stage_post media_url 必填 ([96341d7](https://github.com/CatiesGames/catclaw/commit/96341d7ba615b0c1d43b96586f0269dc4c175a77))
* embedder 啟動初始化 + backfill 補齊分析和 embedding + 跳過空內容 ([3d49f5d](https://github.com/CatiesGames/catclaw/commit/3d49f5d0c5a0ee5702d20ca729af6d8ecf712f72))
* embedder 啟動時初始化 + backfill 補齊所有缺失資料 ([f364a47](https://github.com/CatiesGames/catclaw/commit/f364a4764ea3ca06be50f5342d1dfda189e5b805))
* enable kitty keyboard protocol for Shift+Enter newline ([9c909a3](https://github.com/CatiesGames/catclaw/commit/9c909a3429887506f3026b7845532809c435b774))
* failed draft 可重新核准（retry transient API errors） ([588fec2](https://github.com/CatiesGames/catclaw/commit/588fec28197338cb21be1c0b67c29915551c2fce))
* fastembed 改用 rustls 避免 cross-compile OpenSSL 依賴 ([34d3bee](https://github.com/CatiesGames/catclaw/commit/34d3beee72b59a9bab02b249ca158db2e1adfe05))
* forward card 按鈕在 AI 回覆後恢復的問題 ([21f32a1](https://github.com/CatiesGames/catclaw/commit/21f32a1336fd30329e63233b5b92c0d857d117bd))
* forward card 顯示原文 + auto_reply 傳遞原文 context ([a02d611](https://github.com/CatiesGames/catclaw/commit/a02d6110eb56374fd4b63a1763c2f3ccd09a951f))
* gateway 改為預設 bind 0.0.0.0，新增 bind_addr 設定 ([6a31a67](https://github.com/CatiesGames/catclaw/commit/6a31a6788261e2c78d435dcfb7be0c643993b7a6))
* HTTP MCP discovery 帶 auth headers + 變數替換 + 加日誌 ([0031425](https://github.com/CatiesGames/catclaw/commit/00314259c79480e25cb3a91c46d20d85488c85c1))
* HTTP MCP discovery 支援 Streamable HTTP (SSE) + Accept header ([d65579a](https://github.com/CatiesGames/catclaw/commit/d65579a4d08b841ad99f50510632507eec5517e9))
* HTTP MCP server headers 的 ${VAR} 由 CatClaw 預先展開 ([c4bf7f5](https://github.com/CatiesGames/catclaw/commit/c4bf7f51975e679832fd29fa4a4c1530b06a6b9c))
* Instagram Login token 支援（IG... 前綴 → graph.instagram.com） ([84e19f1](https://github.com/CatiesGames/catclaw/commit/84e19f18912b41b49f44f1333acc3b22a778f4d9))
* kg_add_triple 加 source_node_id + remigrate 全清 KG ([c8cb7a7](https://github.com/CatiesGames/catclaw/commit/c8cb7a70552946ce93c4bdae4f424b75d1d7cf8a))
* macOS update 後 ad-hoc codesign 減少 TCC 權限彈窗 ([9fad35f](https://github.com/CatiesGames/catclaw/commit/9fad35ff3ba4ed6f28816935923f37fe8cef843a))
* multi-server session key 碰撞 + TUI Slack MCP tools 缺失 ([9fb6137](https://github.com/CatiesGames/catclaw/commit/9fb6137bbcacbb6d4e8dce90b803ce920e6f3c5e))
* per-session 訊息佇列 + 修復 Slack 附件下載認證 ([a09ef04](https://github.com/CatiesGames/catclaw/commit/a09ef04bf7cc093099cdb6288eef872efec2c51b))
* poller cursor 改用 timestamp + threads_reply 參數改名 reply_to_id ([e267bdb](https://github.com/CatiesGames/catclaw/commit/e267bdb63f9c0035b9e6e4c29067717157e3b7ef))
* polling 的 ID 比較改為數字比較，修復漏抓留言 ([6b8d78a](https://github.com/CatiesGames/catclaw/commit/6b8d78ad840f199ec9a9fccc198ba1ccf88a61f5))
* remove openssl dependency, gate xml_escape with cfg(macos) ([f3b1d46](https://github.com/CatiesGames/catclaw/commit/f3b1d4689e4e27eabe386b2e1110dd9333179768))
* resolve all clippy warnings, switch reqwest to rustls-tls ([0eb9bd1](https://github.com/CatiesGames/catclaw/commit/0eb9bd1fc8ed553813ae71360915671b363f3697))
* resolve relative paths against config file directory ([87d5585](https://github.com/CatiesGames/catclaw/commit/87d5585c4f33333b4d82412ca3e184d2ccb947ee))
* retry Discord slash command registration on transient HTTP errors ([79b0154](https://github.com/CatiesGames/catclaw/commit/79b015411ff059c1c9fecc4076db0a6dea5d9bb7))
* session 建立時記錄 channel metadata 到 transcript ([767f646](https://github.com/CatiesGames/catclaw/commit/767f6463b43e0dca64dac9ec8ff92da55b470959))
* shell-quote session-key in hook command 避免中文字/空格解析錯誤 ([eea2ee5](https://github.com/CatiesGames/catclaw/commit/eea2ee54242202e2b44c72f1dc3e985a8fe4d170))
* skip transcript for system sessions, use open_existing for diary ([b7f7ed5](https://github.com/CatiesGames/catclaw/commit/b7f7ed596dc8fc074d4cd70136f2cb2170e5ddaf))
* Slack API 統一改 form-encoded，修復 users.info user_not_found ([6c17b2c](https://github.com/CatiesGames/catclaw/commit/6c17b2cee95e77a64f14525f50dcd78555035100))
* Slack DM 不帶 thread_ts 防止開新 thread ([bd1e07e](https://github.com/CatiesGames/catclaw/commit/bd1e07e7e6258bd7d17b78bbc646a0bb53b5358f))
* Slack manifest 參考 OpenClaw 補齊缺少的設定 ([a5f16f0](https://github.com/CatiesGames/catclaw/commit/a5f16f09e8ba285e3325f8c49f218dcbde0e4ef3))
* Slack manifest 改用 JSON 格式 + 移除邊框方便複製 ([12bd990](https://github.com/CatiesGames/catclaw/commit/12bd990f1a35dadd8a7e4575fa9a5c7ed3c69724))
* Slack manifest 補齊 app_home、app_mentions:read、files:read ([5448b16](https://github.com/CatiesGames/catclaw/commit/5448b1605039d517059f56d026eac6033f29b0a6))
* Slack onboard 改用 App Manifest 簡化設定流程 ([01fc257](https://github.com/CatiesGames/catclaw/commit/01fc25795f36bddb9f56240f8f149463b51a3ccd))
* Slack onboard 補充 App-Level Token scope 說明 ([d6b792a](https://github.com/CatiesGames/catclaw/commit/d6b792a368a0982d5c4cbea444d23777b6b60173))
* Slack Socket Mode 訊息去重，防止 gateway restart 導致重複處理 ([4c6bae8](https://github.com/CatiesGames/catclaw/commit/4c6bae89bf7c42312a78dfc9ca46d3da6e35b924))
* Slack thinking status 時機修正 + user_not_found fallback ([f9f562e](https://github.com/CatiesGames/catclaw/commit/f9f562e4ca4ce0bfcd04aa27eb430e38e6e4b0dc))
* Slack thread_ts 邏輯修正 — DM 必須帶、channel root 不帶 ([1a149a1](https://github.com/CatiesGames/catclaw/commit/1a149a11c4c64a0de145f70bebfc5434c319174f))
* Slack upload_file DM channel_not_found — completeUploadExternal 改 form-encoded ([df44082](https://github.com/CatiesGames/catclaw/commit/df440825ff21c15a1365544da6a51cf3795a0b42))
* Slack upload_file 改用 form-encoded 呼叫 getUploadURLExternal ([49bdb4b](https://github.com/CatiesGames/catclaw/commit/49bdb4bf46751141b134a8680d17c2924bc1ded3))
* Slack 斜線命令解析頻道名稱 + MCP discovery summary log ([c13419a](https://github.com/CatiesGames/catclaw/commit/c13419aabbba23031929db6c24c2e26e24918099))
* Slack 核准後保留原始 tool/input 內容，只替換 actions block ([365e740](https://github.com/CatiesGames/catclaw/commit/365e7404ba31359f8b06894acdba1c93252c9d5f))
* **social:** 修復 Discord 草稿捨棄無反應 + 結構化錯誤診斷 ([819ac1d](https://github.com/CatiesGames/catclaw/commit/819ac1d04268a3e94082d21a97d92c39db478654))
* system prompt 加 User MCP Tools 指令，避免 agent 用 Bash/curl 打 MCP ([217e480](https://github.com/CatiesGames/catclaw/commit/217e4805315836203a114c55c11881b7a63c4b13))
* systemd service Restart=always + 自動啟用 loginctl linger ([391c07f](https://github.com/CatiesGames/catclaw/commit/391c07fa8a4637b0c4d00f6ba0f47293a88ed94f))
* timezone config set 即時 hot-reload 到所有 agents ([f08c3ff](https://github.com/CatiesGames/catclaw/commit/f08c3ff15dccf9a02675e2452778bc879f34f3f8))
* tokio-tungstenite 啟用 TLS + ToolSearch 加入預設 allowed tools ([53fec49](https://github.com/CatiesGames/catclaw/commit/53fec4977d7c2c2c75169f3c342008154dd00cf5))
* TUI Agents &gt; Tools 正確顯示 Instagram/Threads MCP tools ([065465b](https://github.com/CatiesGames/catclaw/commit/065465b6f315578a80ba7fe9c23b59cce673a734))
* TUI chat 中文字串換行切割 panic ([b2c74f2](https://github.com/CatiesGames/catclaw/commit/b2c74f2ce19ca19ea1d7d21b85ebd3c1f5473f8c))
* TUI Config panel ↑↓ 選擇補全選項時同步填入輸入格 ([ff1f9d2](https://github.com/CatiesGames/catclaw/commit/ff1f9d2adaa1fb7ac601b511c80d9e6ed64cc012))
* TUI Social/Drafts Esc 返回 + 快捷鍵粉色 + filter 改 [/] + Threads token 到期日 ([9ddc25c](https://github.com/CatiesGames/catclaw/commit/9ddc25cf1e3f3fdef6f879f82e1d043a8ab21b85))
* TUI token 編輯後即時更新 + 到期日內嵌顯示 + config set 自動 exchange ([e13d1f4](https://github.com/CatiesGames/catclaw/commit/e13d1f46b76f461b935d20607e59c09a0a9eb2c9))
* use launchctl bootstrap/bootout instead of load/unload ([3a81938](https://github.com/CatiesGames/catclaw/commit/3a8193871ed5633234b316dbda95cd7d2a0c2488))
* User MCP Tools 指引改為通用寫法，移除 ToolSearch 誤導 ([41a8717](https://github.com/CatiesGames/catclaw/commit/41a8717e7978ac3def7a295e8a5aea25147ab9ef))
* write transcript with tool_use details, log user message immediately ([6ee15e8](https://github.com/CatiesGames/catclaw/commit/6ee15e8ca924055b6b58861c2e66b3a5ad40e8ac))
* 容忍 agent 傳字串化 JSON array 作為 image_urls/media_urls ([4c571c0](https://github.com/CatiesGames/catclaw/commit/4c571c0084e91a1c4e8d693f76f9938d8b03e360))
* 審核卡片轉發失敗時增加 warning log ([0bdc327](https://github.com/CatiesGames/catclaw/commit/0bdc32740d4254ef844e9fabe653fcf69cd166ec))
* 將 release build 整合進 release-please workflow ([1ca1675](https://github.com/CatiesGames/catclaw/commit/1ca1675518997aeb90606237f3ac3de5a17eb47f))
* 排程任務的社群發文工具必須走審核流程 ([554e217](https://github.com/CatiesGames/catclaw/commit/554e21742cfaa52276cb9c2919b9d47209bfc091))
* 攔截 NO_REPLY 回覆 + Slack file_share 缺少 team_id ([bc08607](https://github.com/CatiesGames/catclaw/commit/bc086072279c0f96c8575deb6686a25ddf0b20bb))
* 啟用 kitty keyboard protocol 後按鍵重複輸入 ([8717823](https://github.com/CatiesGames/catclaw/commit/8717823289f0a41e34a37919cfc2b2be9a3091d6))
* 核准卡片轉發失敗時不再靜默吞錯 ([02844ec](https://github.com/CatiesGames/catclaw/commit/02844ec4f82e0fb8b27071ae4000e4c0494f98c5))
* 永遠顯示 app_secret 和 webhook_verify_token 設定欄位 ([3a059b8](https://github.com/CatiesGames/catclaw/commit/3a059b8280c4989f8736d27c68d7b58b26750f03))
* 版本號動態化、新增 version 子命令、輸入框動態高度、三層焦點模式 ([87a7cb9](https://github.com/CatiesGames/catclaw/commit/87a7cb927fe51e72d46989122ad46256b0fb1219))
* 發文核准後保留 media_tmp 圖片，避免審核卡片圖片掛掉 ([3487a32](https://github.com/CatiesGames/catclaw/commit/3487a3206553fd06c7cc22797bced3cd3698263b))
* 發文核准按鈕不再排隊等待 + Threads 追蹤子回覆 ([d42ea14](https://github.com/CatiesGames/catclaw/commit/d42ea14ba501b7e9482965ea56fb8eaabcb037c1))
* 社群 onboard webhook 排第一；Config panel 成功訊息不被覆蓋；admin_channel 提示說明 ID 格式 ([0f216ed](https://github.com/CatiesGames/catclaw/commit/0f216ede574583682d74713fb6d16dcd5f480ec4))
* 空內容由 Haiku 判斷，回傳空 summary 時刪除節點 ([cbd7477](https://github.com/CatiesGames/catclaw/commit/cbd7477c44d2eb545674afc96d2f8283ef8fb1b5))
* 背景執行時關閉 BGE-M3 下載進度條避免失敗 ([0c81ac3](https://github.com/CatiesGames/catclaw/commit/0c81ac38dbbc181d988a9fe0e440bfc2d32c7248))
* 首次審核卡片顯示「核准發送」，失敗後才顯示「重試發送」 ([7c6264b](https://github.com/CatiesGames/catclaw/commit/7c6264b754f7f80a753f6017224e0b1e8528779a))
* 驗證 Haiku room 名稱必須 kebab-case + 加 memory fix-rooms 命令 ([d6be264](https://github.com/CatiesGames/catclaw/commit/d6be264b90da620a75400da5e4fefa77bcaf9c70))

## [0.38.1](https://github.com/CatiesGames/catclaw/compare/v0.38.0...v0.38.1) (2026-04-14)


### Bug Fixes

* **social:** 修復 Discord 草稿捨棄無反應 + 結構化錯誤診斷 ([819ac1d](https://github.com/CatiesGames/catclaw/commit/819ac1d04268a3e94082d21a97d92c39db478654))

## [0.38.0](https://github.com/CatiesGames/catclaw/compare/v0.37.3...v0.38.0) (2026-04-13)


### Features

* **social:** 統一 reply 卡片生命週期 + 修復 AI 重試殭屍 draft ([41a3f7b](https://github.com/CatiesGames/catclaw/commit/41a3f7b3a6d86649cfb1918e7e247b7544cc0c8c))

## [0.37.3](https://github.com/CatiesGames/catclaw/compare/v0.37.2...v0.37.3) (2026-04-11)


### Bug Fixes

* 驗證 Haiku room 名稱必須 kebab-case + 加 memory fix-rooms 命令 ([d6be264](https://github.com/CatiesGames/catclaw/commit/d6be264b90da620a75400da5e4fefa77bcaf9c70))

## [0.37.2](https://github.com/CatiesGames/catclaw/compare/v0.37.1...v0.37.2) (2026-04-11)


### Bug Fixes

* kg_add_triple 加 source_node_id + remigrate 全清 KG ([c8cb7a7](https://github.com/CatiesGames/catclaw/commit/c8cb7a70552946ce93c4bdae4f424b75d1d7cf8a))

## [0.37.1](https://github.com/CatiesGames/catclaw/compare/v0.37.0...v0.37.1) (2026-04-11)


### Bug Fixes

* backfill 不分析 extraction nodes 避免無限循環 ([f917234](https://github.com/CatiesGames/catclaw/commit/f917234412fac1718f57b0f0f0aff5f851628418))

## [0.37.0](https://github.com/CatiesGames/catclaw/compare/v0.36.1...v0.37.0) (2026-04-11)


### Features

* catclaw memory status/reset/remigrate + update --version + system session 不寫 transcript ([f2ccafa](https://github.com/CatiesGames/catclaw/commit/f2ccafa2e8216b0fd37868fedefef9b60a156d1a))

## [0.36.1](https://github.com/CatiesGames/catclaw/compare/v0.36.0...v0.36.1) (2026-04-10)


### Bug Fixes

* backfill 加 10s 間隔 + 連續失敗暫停 + 區分錯誤類型 ([097d5d6](https://github.com/CatiesGames/catclaw/commit/097d5d69305a4a1f9d43cef8fc67705490ec8bf7))

## [0.36.0](https://github.com/CatiesGames/catclaw/compare/v0.35.7...v0.36.0) (2026-04-10)


### Features

* macOS binary 用 Developer ID 簽名解決 TCC 權限彈窗 ([e15230f](https://github.com/CatiesGames/catclaw/commit/e15230ff81c2daf48c9299a3408675d2f46a951a))


### Bug Fixes

* BGE-M3 模型 cache 改用絕對路徑 ~/.catclaw/models/ ([05d8b43](https://github.com/CatiesGames/catclaw/commit/05d8b436f0f3adc2f6952b1d2d13db955ee33637))

## [0.35.7](https://github.com/CatiesGames/catclaw/compare/v0.35.6...v0.35.7) (2026-04-10)


### Bug Fixes

* 背景執行時關閉 BGE-M3 下載進度條避免失敗 ([0c81ac3](https://github.com/CatiesGames/catclaw/commit/0c81ac38dbbc181d988a9fe0e440bfc2d32c7248))

## [0.35.6](https://github.com/CatiesGames/catclaw/compare/v0.35.5...v0.35.6) (2026-04-10)


### Bug Fixes

* backfill 分析失敗時標記 empty summary 避免無限重試 ([c7d1639](https://github.com/CatiesGames/catclaw/commit/c7d163902faef116a47dc5646730eb54cbf03339))
* BGE-M3 下載失敗 — hf-hub 改回 native-tls ([7a1d9ae](https://github.com/CatiesGames/catclaw/commit/7a1d9aee5196af3ec710324d03902a47f54ea8d2))
* macOS update 後 ad-hoc codesign 減少 TCC 權限彈窗 ([9fad35f](https://github.com/CatiesGames/catclaw/commit/9fad35ff3ba4ed6f28816935923f37fe8cef843a))

## [0.35.5](https://github.com/CatiesGames/catclaw/compare/v0.35.4...v0.35.5) (2026-04-10)


### Bug Fixes

* 空內容由 Haiku 判斷，回傳空 summary 時刪除節點 ([cbd7477](https://github.com/CatiesGames/catclaw/commit/cbd7477c44d2eb545674afc96d2f8283ef8fb1b5))

## [0.35.4](https://github.com/CatiesGames/catclaw/compare/v0.35.3...v0.35.4) (2026-04-10)


### Bug Fixes

* embedder 啟動初始化 + backfill 補齊分析和 embedding + 跳過空內容 ([3d49f5d](https://github.com/CatiesGames/catclaw/commit/3d49f5d0c5a0ee5702d20ca729af6d8ecf712f72))

## [0.35.3](https://github.com/CatiesGames/catclaw/compare/v0.35.2...v0.35.3) (2026-04-10)


### Bug Fixes

* embedder 啟動時初始化 + backfill 補齊所有缺失資料 ([f364a47](https://github.com/CatiesGames/catclaw/commit/f364a4764ea3ca06be50f5342d1dfda189e5b805))

## [0.35.2](https://github.com/CatiesGames/catclaw/compare/v0.35.1...v0.35.2) (2026-04-10)


### Bug Fixes

* aarch64-linux 改用 native ARM runner 避免 cross-compile ONNX 問題 ([dfc652d](https://github.com/CatiesGames/catclaw/commit/dfc652dc4504965e29d3bc9fff1662e9e4a54c46))

## [0.35.1](https://github.com/CatiesGames/catclaw/compare/v0.35.0...v0.35.1) (2026-04-10)


### Bug Fixes

* fastembed 改用 rustls 避免 cross-compile OpenSSL 依賴 ([34d3bee](https://github.com/CatiesGames/catclaw/commit/34d3beee72b59a9bab02b249ca158db2e1adfe05))

## [0.35.0](https://github.com/CatiesGames/catclaw/compare/v0.34.1...v0.35.0) (2026-04-10)


### Features

* 記憶系統可針對指定 agent 關閉 ([f73a624](https://github.com/CatiesGames/catclaw/commit/f73a6243e7a56d1134e90e20897ecd603ba13bef))

## [0.34.1](https://github.com/CatiesGames/catclaw/compare/v0.34.0...v0.34.1) (2026-04-10)


### Bug Fixes

* auto_reply prompt 用 MCP tool 全名 + 強制呼叫 ([f3dec9f](https://github.com/CatiesGames/catclaw/commit/f3dec9f481aac8c2a2f7c067cf253015279fd26f))
* forward card 按鈕在 AI 回覆後恢復的問題 ([21f32a1](https://github.com/CatiesGames/catclaw/commit/21f32a1336fd30329e63233b5b92c0d857d117bd))
* forward card 顯示原文 + auto_reply 傳遞原文 context ([a02d611](https://github.com/CatiesGames/catclaw/commit/a02d6110eb56374fd4b63a1763c2f3ccd09a951f))

## [0.34.0](https://github.com/CatiesGames/catclaw/compare/v0.33.2...v0.34.0) (2026-04-09)


### Features

* 記憶宮殿系統 (MemPalace) — 取代 markdown 記憶機制 ([d4589a2](https://github.com/CatiesGames/catclaw/commit/d4589a28146e6174e20d4fafbafe797368f521a5))

## [0.33.2](https://github.com/CatiesGames/catclaw/compare/v0.33.1...v0.33.2) (2026-04-07)


### Bug Fixes

* AI 回覆失敗時還原 forward card 讓使用者可重試 ([ec931b8](https://github.com/CatiesGames/catclaw/commit/ec931b8f1b7815bc5aeb4028bcc2a7898c6c8683))

## [0.33.1](https://github.com/CatiesGames/catclaw/compare/v0.33.0...v0.33.1) (2026-04-04)


### Bug Fixes

* poller cursor 改用 timestamp + threads_reply 參數改名 reply_to_id ([e267bdb](https://github.com/CatiesGames/catclaw/commit/e267bdb63f9c0035b9e6e4c29067717157e3b7ef))

## [0.33.0](https://github.com/CatiesGames/catclaw/compare/v0.32.1...v0.33.0) (2026-04-04)


### Features

* poller debug log + TUI cursor 顯示 + 回覆卡原文 API fallback ([0782509](https://github.com/CatiesGames/catclaw/commit/07825095ba57ea8b2c93c327819405ef24c87ade))

## [0.32.1](https://github.com/CatiesGames/catclaw/compare/v0.32.0...v0.32.1) (2026-04-02)


### Bug Fixes

* 容忍 agent 傳字串化 JSON array 作為 image_urls/media_urls ([4c571c0](https://github.com/CatiesGames/catclaw/commit/4c571c0084e91a1c4e8d693f76f9938d8b03e360))

## [0.32.0](https://github.com/CatiesGames/catclaw/compare/v0.31.0...v0.32.0) (2026-04-02)


### Features

* upload_media 支援批次上傳 + 補齊 skill 多圖操作步驟 ([1a673fe](https://github.com/CatiesGames/catclaw/commit/1a673fe1c1366d32cf6bc965a80dcaa8895a56c4))

## [0.31.0](https://github.com/CatiesGames/catclaw/compare/v0.30.0...v0.31.0) (2026-04-02)


### Features

* IG/Threads carousel（多圖輪播）發文支援 + 修復草稿審核流程 ([a213648](https://github.com/CatiesGames/catclaw/commit/a2136485302fbf69277de86e2ff85c9f66b7ddc9))

## [0.30.0](https://github.com/CatiesGames/catclaw/compare/v0.29.0...v0.30.0) (2026-04-02)


### Features

* 社群卡片與 TUI 顯示 platform_id（Threads/IG 原生 ID） ([1a0a6d2](https://github.com/CatiesGames/catclaw/commit/1a0a6d2b2a6e1bf7757362a5b87f6c155ef3753d))


### Bug Fixes

* 核准卡片轉發失敗時不再靜默吞錯 ([02844ec](https://github.com/CatiesGames/catclaw/commit/02844ec4f82e0fb8b27071ae4000e4c0494f98c5))

## [0.29.0](https://github.com/CatiesGames/catclaw/compare/v0.28.2...v0.29.0) (2026-04-01)


### Features

* 社群 inbox 卡片增加「查看原文」按鈕，AI 回覆自動抓原文 context ([788af3e](https://github.com/CatiesGames/catclaw/commit/788af3eb0e111561bf5cbcd99bcf151b652eb66e))


### Bug Fixes

* 發文核准按鈕不再排隊等待 + Threads 追蹤子回覆 ([d42ea14](https://github.com/CatiesGames/catclaw/commit/d42ea14ba501b7e9482965ea56fb8eaabcb037c1))

## [0.28.2](https://github.com/CatiesGames/catclaw/compare/v0.28.1...v0.28.2) (2026-04-01)


### Bug Fixes

* polling 的 ID 比較改為數字比較，修復漏抓留言 ([6b8d78a](https://github.com/CatiesGames/catclaw/commit/6b8d78ad840f199ec9a9fccc198ba1ccf88a61f5))

## [0.28.1](https://github.com/CatiesGames/catclaw/compare/v0.28.0...v0.28.1) (2026-03-31)


### Bug Fixes

* 審核卡片轉發失敗時增加 warning log ([0bdc327](https://github.com/CatiesGames/catclaw/commit/0bdc32740d4254ef844e9fabe653fcf69cd166ec))

## [0.28.0](https://github.com/CatiesGames/catclaw/compare/v0.27.1...v0.28.0) (2026-03-29)


### Features

* 排程任務預設每次開新 session，避免 context 污染 ([b495efd](https://github.com/CatiesGames/catclaw/commit/b495efde598a0d6bc3b5cf3439acf4b966e69c74))


### Bug Fixes

* 發文核准後保留 media_tmp 圖片，避免審核卡片圖片掛掉 ([3487a32](https://github.com/CatiesGames/catclaw/commit/3487a3206553fd06c7cc22797bced3cd3698263b))

## [0.27.1](https://github.com/CatiesGames/catclaw/compare/v0.27.0...v0.27.1) (2026-03-28)


### Bug Fixes

* 排程任務的社群發文工具必須走審核流程 ([554e217](https://github.com/CatiesGames/catclaw/commit/554e21742cfaa52276cb9c2919b9d47209bfc091))

## [0.27.0](https://github.com/CatiesGames/catclaw/compare/v0.26.0...v0.27.0) (2026-03-27)


### Features

* Discord/Telegram reply 時 agent 可看到被回覆的原文 ([e7e1821](https://github.com/CatiesGames/catclaw/commit/e7e182117834b3339ab6e2a91b38cc75179ef943))
* 發文審核卡片狀態流轉 — 核准→發送中(橘)→成功(綠)/失敗(紅+重試) ([414d2ca](https://github.com/CatiesGames/catclaw/commit/414d2cad7cf2385d67ee9480a3687759083e8428))


### Bug Fixes

* 首次審核卡片顯示「核准發送」，失敗後才顯示「重試發送」 ([7c6264b](https://github.com/CatiesGames/catclaw/commit/7c6264b754f7f80a753f6017224e0b1e8528779a))

## [0.26.0](https://github.com/CatiesGames/catclaw/compare/v0.25.2...v0.26.0) (2026-03-27)


### Features

* 發文失敗卡片顯示紅色 + 重試/捨棄按鈕（全平台一致） ([a80689b](https://github.com/CatiesGames/catclaw/commit/a80689bc827bd55cbb9f3190c8d3b5d21b806b03))


### Bug Fixes

* failed draft 可重新核准（retry transient API errors） ([588fec2](https://github.com/CatiesGames/catclaw/commit/588fec28197338cb21be1c0b67c29915551c2fce))

## [0.25.2](https://github.com/CatiesGames/catclaw/compare/v0.25.1...v0.25.2) (2026-03-27)


### Bug Fixes

* draft 審核日誌 + poller messages 靜默跳過 + IG stage_post media_url 必填 ([96341d7](https://github.com/CatiesGames/catclaw/commit/96341d7ba615b0c1d43b96586f0269dc4c175a77))

## [0.25.1](https://github.com/CatiesGames/catclaw/compare/v0.25.0...v0.25.1) (2026-03-27)


### Bug Fixes

* TUI Social/Drafts Esc 返回 + 快捷鍵粉色 + filter 改 [/] + Threads token 到期日 ([9ddc25c](https://github.com/CatiesGames/catclaw/commit/9ddc25cf1e3f3fdef6f879f82e1d043a8ab21b85))

## [0.25.0](https://github.com/CatiesGames/catclaw/compare/v0.24.1...v0.25.0) (2026-03-27)


### Features

* 社群圖片上傳自動轉換格式 ([f723afe](https://github.com/CatiesGames/catclaw/commit/f723afe764c9591c646752f37718efb1b7d5eb35))

## [0.24.1](https://github.com/CatiesGames/catclaw/compare/v0.24.0...v0.24.1) (2026-03-27)


### Bug Fixes

* Discord 審核卡片顯示附圖 + post 類型隱藏 From + TUI draft 欄寬調整 ([c9f3c74](https://github.com/CatiesGames/catclaw/commit/c9f3c7488e3d741567410cea8bd35c7534147aa7))

## [0.24.0](https://github.com/CatiesGames/catclaw/compare/v0.23.1...v0.24.0) (2026-03-26)


### Features

* [env] 子進程環境變數 + 社群發文審核流程改進 ([30f09e9](https://github.com/CatiesGames/catclaw/commit/30f09e97c0276923362b1719254df1e58cdefae5))

## [0.23.1](https://github.com/CatiesGames/catclaw/compare/v0.23.0...v0.23.1) (2026-03-26)


### Bug Fixes

* Instagram Login token 支援（IG... 前綴 → graph.instagram.com） ([84e19f1](https://github.com/CatiesGames/catclaw/commit/84e19f18912b41b49f44f1333acc3b22a778f4d9))
* TUI token 編輯後即時更新 + 到期日內嵌顯示 + config set 自動 exchange ([e13d1f4](https://github.com/CatiesGames/catclaw/commit/e13d1f46b76f461b935d20607e59c09a0a9eb2c9))

## [0.23.0](https://github.com/CatiesGames/catclaw/compare/v0.22.0...v0.23.0) (2026-03-26)


### Features

* config set token 後自動 exchange + TUI 顯示 token 到期日 ([73c9aeb](https://github.com/CatiesGames/catclaw/commit/73c9aebc80a8be11c5288dfdf18fcfb227ae0506))

## [0.22.0](https://github.com/CatiesGames/catclaw/compare/v0.21.0...v0.22.0) (2026-03-26)


### Features

* social_drafts 系統 + 核准流程改造 + TUI Drafts 面板 ([b889ba9](https://github.com/CatiesGames/catclaw/commit/b889ba9d8b595da432a1811ebbe3c8ab8ebf3363))
* 社群卡片按鈕互動全面優化 + 建議 AI 回覆 ([7ea612e](https://github.com/CatiesGames/catclaw/commit/7ea612e34653c42a9fa13023bfba8745f2244e7c))

## [0.21.0](https://github.com/CatiesGames/catclaw/compare/v0.20.2...v0.21.0) (2026-03-26)


### Features

* IG 發文/私訊、Threads 關鍵字搜尋、webhook 補齊、task get、config 預設值 ([751863a](https://github.com/CatiesGames/catclaw/commit/751863ab49a2b3848d9d6e98f1c9c1834298d9f4))
* IG/Threads token 自動換長效 + 定期續期，修正 Issues panel，改善 debug 日誌 ([f2f9845](https://github.com/CatiesGames/catclaw/commit/f2f98456fe1903e57c403250a30e411f63a42f31))

## [0.20.2](https://github.com/CatiesGames/catclaw/compare/v0.20.1...v0.20.2) (2026-03-26)


### Bug Fixes

* 永遠顯示 app_secret 和 webhook_verify_token 設定欄位 ([3a059b8](https://github.com/CatiesGames/catclaw/commit/3a059b8280c4989f8736d27c68d7b58b26750f03))

## [0.20.1](https://github.com/CatiesGames/catclaw/compare/v0.20.0...v0.20.1) (2026-03-26)


### Bug Fixes

* clippy unnecessary_unwrap in task add schedule display ([3bfc1e1](https://github.com/CatiesGames/catclaw/commit/3bfc1e19a935e6a06bfce8d92e1bc5f587ed3ea0))
* cron 任務明確標示 UTC，避免 agent 時區誤解 ([46bd1cf](https://github.com/CatiesGames/catclaw/commit/46bd1cfde4be6fa4f548bfe8b1e6db8a5a369160))

## [0.20.0](https://github.com/CatiesGames/catclaw/compare/v0.19.0...v0.20.0) (2026-03-25)


### Features

* issue tracking + TUI Issues panel + Instagram/Threads MCP tools 顯示 ([065465b](https://github.com/CatiesGames/catclaw/commit/065465b6f315578a80ba7fe9c23b59cce673a734))


### Bug Fixes

* TUI Agents &gt; Tools 正確顯示 Instagram/Threads MCP tools ([065465b](https://github.com/CatiesGames/catclaw/commit/065465b6f315578a80ba7fe9c23b59cce673a734))

## [0.19.0](https://github.com/CatiesGames/catclaw/compare/v0.18.2...v0.19.0) (2026-03-25)


### Features

* 新增 /health 端點 ([ea23da5](https://github.com/CatiesGames/catclaw/commit/ea23da5601bdbb799ceade11c937c8858ab6a3df))


### Bug Fixes

* gateway 改為預設 bind 0.0.0.0，新增 bind_addr 設定 ([6a31a67](https://github.com/CatiesGames/catclaw/commit/6a31a6788261e2c78d435dcfb7be0c643993b7a6))

## [0.18.2](https://github.com/CatiesGames/catclaw/compare/v0.18.1...v0.18.2) (2026-03-25)


### Bug Fixes

* TUI Config panel ↑↓ 選擇補全選項時同步填入輸入格 ([ff1f9d2](https://github.com/CatiesGames/catclaw/commit/ff1f9d2adaa1fb7ac601b511c80d9e6ed64cc012))
* 社群 onboard webhook 排第一；Config panel 成功訊息不被覆蓋；admin_channel 提示說明 ID 格式 ([0f216ed](https://github.com/CatiesGames/catclaw/commit/0f216ede574583682d74713fb6d16dcd5f480ec4))

## [0.18.1](https://github.com/CatiesGames/catclaw/compare/v0.18.0...v0.18.1) (2026-03-25)


### Bug Fixes

* catclaw onboard 重複執行時進入 wizard 而非跳過；Config panel 選擇時自動捲動 ([9b65907](https://github.com/CatiesGames/catclaw/commit/9b659074bfc4abeb94105bdd6cedd3d5ae8f8808))

## [0.18.0](https://github.com/CatiesGames/catclaw/compare/v0.17.1...v0.18.0) (2026-03-25)


### Features

* Social Inbox 全設定可透過 TUI/CLI 設定 ([660318c](https://github.com/CatiesGames/catclaw/commit/660318c2c9910cc472f1d1d42dafd85177af7bcf))
* TUI Skills 清單顯示 built-in 標記 ([a71c318](https://github.com/CatiesGames/catclaw/commit/a71c3188238647a287d8e5def3da62ff10fced31))

## [0.17.1](https://github.com/CatiesGames/catclaw/compare/v0.17.0...v0.17.1) (2026-03-25)


### Bug Fixes

* diary 生成時 MCP 配置傳入正確的 mcpServers 結構 ([3ac79f2](https://github.com/CatiesGames/catclaw/commit/3ac79f2097ad9a1df8dbf5f95c5913b0408d4db6))

## [0.17.0](https://github.com/CatiesGames/catclaw/compare/v0.16.5...v0.17.0) (2026-03-25)


### Features

* Social Inbox — Instagram + Threads 整合 ([26a7627](https://github.com/CatiesGames/catclaw/commit/26a76277bccf21548978fd9a66e8e28857c4cda8))

## [0.16.5](https://github.com/CatiesGames/catclaw/compare/v0.16.4...v0.16.5) (2026-03-24)


### Bug Fixes

* Slack 核准後保留原始 tool/input 內容，只替換 actions block ([365e740](https://github.com/CatiesGames/catclaw/commit/365e7404ba31359f8b06894acdba1c93252c9d5f))

## [0.16.4](https://github.com/CatiesGames/catclaw/compare/v0.16.3...v0.16.4) (2026-03-24)


### Bug Fixes

* shell-quote session-key in hook command 避免中文字/空格解析錯誤 ([eea2ee5](https://github.com/CatiesGames/catclaw/commit/eea2ee54242202e2b44c72f1dc3e985a8fe4d170))

## [0.16.3](https://github.com/CatiesGames/catclaw/compare/v0.16.2...v0.16.3) (2026-03-21)


### Bug Fixes

* diary subprocess 隔離所有工具避免 max-turns 超限 ([539ce8b](https://github.com/CatiesGames/catclaw/commit/539ce8b8ff7cfabdcc836f5719f39293e03a21e3))

## [0.16.2](https://github.com/CatiesGames/catclaw/compare/v0.16.1...v0.16.2) (2026-03-21)


### Bug Fixes

* Slack 斜線命令解析頻道名稱 + MCP discovery summary log ([c13419a](https://github.com/CatiesGames/catclaw/commit/c13419aabbba23031929db6c24c2e26e24918099))

## [0.16.1](https://github.com/CatiesGames/catclaw/compare/v0.16.0...v0.16.1) (2026-03-21)


### Bug Fixes

* HTTP MCP server headers 的 ${VAR} 由 CatClaw 預先展開 ([c4bf7f5](https://github.com/CatiesGames/catclaw/commit/c4bf7f51975e679832fd29fa4a4c1530b06a6b9c))
* User MCP Tools 指引改為通用寫法，移除 ToolSearch 誤導 ([41a8717](https://github.com/CatiesGames/catclaw/commit/41a8717e7978ac3def7a295e8a5aea25147ab9ef))

## [0.16.0](https://github.com/CatiesGames/catclaw/compare/v0.15.2...v0.16.0) (2026-03-21)


### Features

* TUI Agents 加滾輪 + PageUp/Down/Home/End/g/G 導航 ([d8badbc](https://github.com/CatiesGames/catclaw/commit/d8badbc08b2ee84c530f61d06d2c02e0ba381004))


### Bug Fixes

* system prompt 加 User MCP Tools 指令，避免 agent 用 Bash/curl 打 MCP ([217e480](https://github.com/CatiesGames/catclaw/commit/217e4805315836203a114c55c11881b7a63c4b13))

## [0.15.2](https://github.com/CatiesGames/catclaw/compare/v0.15.1...v0.15.2) (2026-03-21)


### Bug Fixes

* HTTP MCP discovery 支援 Streamable HTTP (SSE) + Accept header ([d65579a](https://github.com/CatiesGames/catclaw/commit/d65579a4d08b841ad99f50510632507eec5517e9))

## [0.15.1](https://github.com/CatiesGames/catclaw/compare/v0.15.0...v0.15.1) (2026-03-21)


### Bug Fixes

* HTTP MCP discovery 帶 auth headers + 變數替換 + 加日誌 ([0031425](https://github.com/CatiesGames/catclaw/commit/00314259c79480e25cb3a91c46d20d85488c85c1))

## [0.15.0](https://github.com/CatiesGames/catclaw/compare/v0.14.1...v0.15.0) (2026-03-21)


### Features

* MCP env 管理 + MCP tools 自動探測 ([cbbc0f2](https://github.com/CatiesGames/catclaw/commit/cbbc0f2310c8b85287f36e5c807d5f3536b89370))


### Bug Fixes

* BOOT.md 分支補上 active_handles 註冊，/stop 不再失效 ([40aa242](https://github.com/CatiesGames/catclaw/commit/40aa242faa4b0ec09cb3dbc4e33a8c3f85db563a))
* Discord thread 建立事件誤觸發主頻道回覆 + thread 偵測邏輯修正 ([e6385c5](https://github.com/CatiesGames/catclaw/commit/e6385c5582e9446d86365c01e8ca434d0a23b9a5))

## [0.14.1](https://github.com/CatiesGames/catclaw/compare/v0.14.0...v0.14.1) (2026-03-20)


### Bug Fixes

* built-in skills 每次啟動覆寫為最新版 ([65beea5](https://github.com/CatiesGames/catclaw/commit/65beea57106ed0296add8f5a9cde3bf10997ad55))

## [0.14.0](https://github.com/CatiesGames/catclaw/compare/v0.13.3...v0.14.0) (2026-03-20)


### Features

* Slack reaction 狀態指示 + 日記 timezone 修正 ([7a21c66](https://github.com/CatiesGames/catclaw/commit/7a21c6645a74a4f4df306e4bc184fed78d4ac2e1))

## [0.13.3](https://github.com/CatiesGames/catclaw/compare/v0.13.2...v0.13.3) (2026-03-20)


### Bug Fixes

* timezone config set 即時 hot-reload 到所有 agents ([f08c3ff](https://github.com/CatiesGames/catclaw/commit/f08c3ff15dccf9a02675e2452778bc879f34f3f8))

## [0.13.2](https://github.com/CatiesGames/catclaw/compare/v0.13.1...v0.13.2) (2026-03-19)


### Bug Fixes

* catclaw skill 強調 self-update 必須用 --notify ([c2a1401](https://github.com/CatiesGames/catclaw/commit/c2a14016a207c8b45931302fd22ad1be8aca83d6))

## [0.13.1](https://github.com/CatiesGames/catclaw/compare/v0.13.0...v0.13.1) (2026-03-19)


### Bug Fixes

* Discord reaction 更絲滑 — 先加後移 + terminal 直接移除 ([4ffac2f](https://github.com/CatiesGames/catclaw/commit/4ffac2fdc65d5b79a876308de38e024c49494908))

## [0.13.0](https://github.com/CatiesGames/catclaw/compare/v0.12.1...v0.13.0) (2026-03-19)


### Features

* Discord reaction 狀態指示器 + 修復空 result 導致 session 斷裂 ([67ecc4c](https://github.com/CatiesGames/catclaw/commit/67ecc4c86e0cd4a3b98c9451207d89af3c2acd19))

## [0.12.1](https://github.com/CatiesGames/catclaw/compare/v0.12.0...v0.12.1) (2026-03-19)


### Bug Fixes

* Slack API 統一改 form-encoded，修復 users.info user_not_found ([6c17b2c](https://github.com/CatiesGames/catclaw/commit/6c17b2cee95e77a64f14525f50dcd78555035100))

## [0.12.0](https://github.com/CatiesGames/catclaw/compare/v0.11.2...v0.12.0) (2026-03-19)


### Features

* update --notify + timezone 修正 + system session transcript ([77e5bdb](https://github.com/CatiesGames/catclaw/commit/77e5bdb14a021a8cdd3362233d3401e9827623be))


### Bug Fixes

* Slack upload_file DM channel_not_found — completeUploadExternal 改 form-encoded ([df44082](https://github.com/CatiesGames/catclaw/commit/df440825ff21c15a1365544da6a51cf3795a0b42))
* systemd service Restart=always + 自動啟用 loginctl linger ([391c07f](https://github.com/CatiesGames/catclaw/commit/391c07fa8a4637b0c4d00f6ba0f47293a88ed94f))

## [0.11.2](https://github.com/CatiesGames/catclaw/compare/v0.11.1...v0.11.2) (2026-03-19)


### Bug Fixes

* Slack Socket Mode 訊息去重，防止 gateway restart 導致重複處理 ([4c6bae8](https://github.com/CatiesGames/catclaw/commit/4c6bae89bf7c42312a78dfc9ca46d3da6e35b924))

## [0.11.1](https://github.com/CatiesGames/catclaw/compare/v0.11.0...v0.11.1) (2026-03-19)


### Bug Fixes

* Slack upload_file 改用 form-encoded 呼叫 getUploadURLExternal ([49bdb4b](https://github.com/CatiesGames/catclaw/commit/49bdb4bf46751141b134a8680d17c2924bc1ded3))

## [0.11.0](https://github.com/CatiesGames/catclaw/compare/v0.10.3...v0.11.0) (2026-03-19)


### Features

* 三平台 upload_file MCP action（Slack/Discord/Telegram） ([01ad6a6](https://github.com/CatiesGames/catclaw/commit/01ad6a6e946580d7f4615e000de11e835c07caa2))

## [0.10.3](https://github.com/CatiesGames/catclaw/compare/v0.10.2...v0.10.3) (2026-03-19)


### Bug Fixes

* TUI chat 中文字串換行切割 panic ([b2c74f2](https://github.com/CatiesGames/catclaw/commit/b2c74f2ce19ca19ea1d7d21b85ebd3c1f5473f8c))
* 攔截 NO_REPLY 回覆 + Slack file_share 缺少 team_id ([bc08607](https://github.com/CatiesGames/catclaw/commit/bc086072279c0f96c8575deb6686a25ddf0b20bb))

## [0.10.2](https://github.com/CatiesGames/catclaw/compare/v0.10.1...v0.10.2) (2026-03-19)


### Bug Fixes

* per-session 訊息佇列 + 修復 Slack 附件下載認證 ([a09ef04](https://github.com/CatiesGames/catclaw/commit/a09ef04bf7cc093099cdb6288eef872efec2c51b))

## [0.10.1](https://github.com/CatiesGames/catclaw/compare/v0.10.0...v0.10.1) (2026-03-19)


### Bug Fixes

* Slack thread_ts 邏輯修正 — DM 必須帶、channel root 不帶 ([1a149a1](https://github.com/CatiesGames/catclaw/commit/1a149a11c4c64a0de145f70bebfc5434c319174f))

## [0.10.0](https://github.com/CatiesGames/catclaw/compare/v0.9.3...v0.10.0) (2026-03-19)


### Features

* add injection-guard built-in skill ([f263ad3](https://github.com/CatiesGames/catclaw/commit/f263ad323371fc74a9a7436e5e5875f7d1316dd8))
* catclaw onboard replaces init, add Chinese README ([a8f1790](https://github.com/CatiesGames/catclaw/commit/a8f1790c01b1c8c2740793acc330060bc75d0582))
* Discord/Telegram slash commands (/stop, /new) + 統一 diary extraction ([cb4cf9c](https://github.com/CatiesGames/catclaw/commit/cb4cf9cc811fb3b60f3882283f41ce5f4494c2a7))
* distribution, approval UX, attachment handling ([594a58e](https://github.com/CatiesGames/catclaw/commit/594a58ec949856bbf41ac5a461647ea9c34d0807))
* human-readable transcript filenames ([83ea1d4](https://github.com/CatiesGames/catclaw/commit/83ea1d406ed2268b742997a71024b98bc2faecc6))
* inject session context header into every agent message ([4ac1d3c](https://github.com/CatiesGames/catclaw/commit/4ac1d3c1c141cffab3c236b2ffa5d3b6f85c1a73))
* local timezone display, task name lookup, one-shot auto-delete ([326ae9c](https://github.com/CatiesGames/catclaw/commit/326ae9c5961eb257dda8a027d361427268c9b4c6))
* Slack channel adapter（Socket Mode + AI streaming） ([31d8e27](https://github.com/CatiesGames/catclaw/commit/31d8e27d30f7f2cd32264752ae0a5d2faab48143))
* task add --at 一次性排程 + agent scheduling 指引 ([2f00410](https://github.com/CatiesGames/catclaw/commit/2f00410e7202556e37860cacbe64e37bef248ca7))
* timezone 設定 + Skill tool 支援 + approval 說明修正 ([9ad0559](https://github.com/CatiesGames/catclaw/commit/9ad05591d35e75a3e84ba0e72babbfc86044eeaa))
* tool approval system + channel forwarding + TUI/CLI improvements ([b51b893](https://github.com/CatiesGames/catclaw/commit/b51b89303e8debcfd917e04b7364bf9b9b282c94))
* unified tool permissions, MCP management, mouse scroll, UX improvements ([a862792](https://github.com/CatiesGames/catclaw/commit/a862792965812a7c64c4b2a47d45a62cc051b1f7))
* unify all runtime files under ~/.catclaw/ ([b9bc44e](https://github.com/CatiesGames/catclaw/commit/b9bc44ed8b725d81298d956c2c958a111b8ec302))
* 自動記憶系統 — 日記提取與長期蒸餾 ([ad77581](https://github.com/CatiesGames/catclaw/commit/ad77581bb6b5ba6201d6696c13a9d6f63b59276a))


### Bug Fixes

* agent 不用 MCP 回覆對話 + DM thread_ts 統一過濾 ([bc7db00](https://github.com/CatiesGames/catclaw/commit/bc7db0098968e1e42bf67f08f82e336c67d520ac))
* approval 發到正確 thread + 點擊後更新卡片 + 多項修復 ([9638e44](https://github.com/CatiesGames/catclaw/commit/9638e4436d3185b6f74580c292b74201d405d24c))
* code review 第二輪修復 + TUI transcript 讀取 bug ([9ccf272](https://github.com/CatiesGames/catclaw/commit/9ccf272b12b2cca1765b197dda07d65e62e734ce))
* enable kitty keyboard protocol for Shift+Enter newline ([9c909a3](https://github.com/CatiesGames/catclaw/commit/9c909a3429887506f3026b7845532809c435b774))
* multi-server session key 碰撞 + TUI Slack MCP tools 缺失 ([9fb6137](https://github.com/CatiesGames/catclaw/commit/9fb6137bbcacbb6d4e8dce90b803ce920e6f3c5e))
* remove openssl dependency, gate xml_escape with cfg(macos) ([f3b1d46](https://github.com/CatiesGames/catclaw/commit/f3b1d4689e4e27eabe386b2e1110dd9333179768))
* resolve all clippy warnings, switch reqwest to rustls-tls ([0eb9bd1](https://github.com/CatiesGames/catclaw/commit/0eb9bd1fc8ed553813ae71360915671b363f3697))
* resolve relative paths against config file directory ([87d5585](https://github.com/CatiesGames/catclaw/commit/87d5585c4f33333b4d82412ca3e184d2ccb947ee))
* retry Discord slash command registration on transient HTTP errors ([79b0154](https://github.com/CatiesGames/catclaw/commit/79b015411ff059c1c9fecc4076db0a6dea5d9bb7))
* session 建立時記錄 channel metadata 到 transcript ([767f646](https://github.com/CatiesGames/catclaw/commit/767f6463b43e0dca64dac9ec8ff92da55b470959))
* skip transcript for system sessions, use open_existing for diary ([b7f7ed5](https://github.com/CatiesGames/catclaw/commit/b7f7ed596dc8fc074d4cd70136f2cb2170e5ddaf))
* Slack DM 不帶 thread_ts 防止開新 thread ([bd1e07e](https://github.com/CatiesGames/catclaw/commit/bd1e07e7e6258bd7d17b78bbc646a0bb53b5358f))
* Slack manifest 參考 OpenClaw 補齊缺少的設定 ([a5f16f0](https://github.com/CatiesGames/catclaw/commit/a5f16f09e8ba285e3325f8c49f218dcbde0e4ef3))
* Slack manifest 改用 JSON 格式 + 移除邊框方便複製 ([12bd990](https://github.com/CatiesGames/catclaw/commit/12bd990f1a35dadd8a7e4575fa9a5c7ed3c69724))
* Slack manifest 補齊 app_home、app_mentions:read、files:read ([5448b16](https://github.com/CatiesGames/catclaw/commit/5448b1605039d517059f56d026eac6033f29b0a6))
* Slack onboard 改用 App Manifest 簡化設定流程 ([01fc257](https://github.com/CatiesGames/catclaw/commit/01fc25795f36bddb9f56240f8f149463b51a3ccd))
* Slack onboard 補充 App-Level Token scope 說明 ([d6b792a](https://github.com/CatiesGames/catclaw/commit/d6b792a368a0982d5c4cbea444d23777b6b60173))
* Slack thinking status 時機修正 + user_not_found fallback ([f9f562e](https://github.com/CatiesGames/catclaw/commit/f9f562e4ca4ce0bfcd04aa27eb430e38e6e4b0dc))
* tokio-tungstenite 啟用 TLS + ToolSearch 加入預設 allowed tools ([53fec49](https://github.com/CatiesGames/catclaw/commit/53fec4977d7c2c2c75169f3c342008154dd00cf5))
* use launchctl bootstrap/bootout instead of load/unload ([3a81938](https://github.com/CatiesGames/catclaw/commit/3a8193871ed5633234b316dbda95cd7d2a0c2488))
* write transcript with tool_use details, log user message immediately ([6ee15e8](https://github.com/CatiesGames/catclaw/commit/6ee15e8ca924055b6b58861c2e66b3a5ad40e8ac))
* 將 release build 整合進 release-please workflow ([1ca1675](https://github.com/CatiesGames/catclaw/commit/1ca1675518997aeb90606237f3ac3de5a17eb47f))
* 啟用 kitty keyboard protocol 後按鍵重複輸入 ([8717823](https://github.com/CatiesGames/catclaw/commit/8717823289f0a41e34a37919cfc2b2be9a3091d6))
* 版本號動態化、新增 version 子命令、輸入框動態高度、三層焦點模式 ([87a7cb9](https://github.com/CatiesGames/catclaw/commit/87a7cb927fe51e72d46989122ad46256b0fb1219))

## [0.9.3](https://github.com/CatiesGames/catclaw/compare/v0.9.2...v0.9.3) (2026-03-19)


### Bug Fixes

* agent 不用 MCP 回覆對話 + DM thread_ts 統一過濾 ([bc7db00](https://github.com/CatiesGames/catclaw/commit/bc7db0098968e1e42bf67f08f82e336c67d520ac))

## [0.9.2](https://github.com/CatiesGames/catclaw/compare/v0.9.1...v0.9.2) (2026-03-19)


### Bug Fixes

* Slack DM 不帶 thread_ts 防止開新 thread ([bd1e07e](https://github.com/CatiesGames/catclaw/commit/bd1e07e7e6258bd7d17b78bbc646a0bb53b5358f))

## [0.9.1](https://github.com/CatiesGames/catclaw/compare/v0.9.0...v0.9.1) (2026-03-19)


### Bug Fixes

* approval 發到正確 thread + 點擊後更新卡片 + 多項修復 ([9638e44](https://github.com/CatiesGames/catclaw/commit/9638e4436d3185b6f74580c292b74201d405d24c))
* multi-server session key 碰撞 + TUI Slack MCP tools 缺失 ([9fb6137](https://github.com/CatiesGames/catclaw/commit/9fb6137bbcacbb6d4e8dce90b803ce920e6f3c5e))

## [0.9.0](https://github.com/CatiesGames/catclaw/compare/v0.8.3...v0.9.0) (2026-03-19)


### Features

* add injection-guard built-in skill ([f263ad3](https://github.com/CatiesGames/catclaw/commit/f263ad323371fc74a9a7436e5e5875f7d1316dd8))
* catclaw onboard replaces init, add Chinese README ([a8f1790](https://github.com/CatiesGames/catclaw/commit/a8f1790c01b1c8c2740793acc330060bc75d0582))
* Discord/Telegram slash commands (/stop, /new) + 統一 diary extraction ([cb4cf9c](https://github.com/CatiesGames/catclaw/commit/cb4cf9cc811fb3b60f3882283f41ce5f4494c2a7))
* distribution, approval UX, attachment handling ([594a58e](https://github.com/CatiesGames/catclaw/commit/594a58ec949856bbf41ac5a461647ea9c34d0807))
* human-readable transcript filenames ([83ea1d4](https://github.com/CatiesGames/catclaw/commit/83ea1d406ed2268b742997a71024b98bc2faecc6))
* inject session context header into every agent message ([4ac1d3c](https://github.com/CatiesGames/catclaw/commit/4ac1d3c1c141cffab3c236b2ffa5d3b6f85c1a73))
* local timezone display, task name lookup, one-shot auto-delete ([326ae9c](https://github.com/CatiesGames/catclaw/commit/326ae9c5961eb257dda8a027d361427268c9b4c6))
* Slack channel adapter（Socket Mode + AI streaming） ([31d8e27](https://github.com/CatiesGames/catclaw/commit/31d8e27d30f7f2cd32264752ae0a5d2faab48143))
* task add --at 一次性排程 + agent scheduling 指引 ([2f00410](https://github.com/CatiesGames/catclaw/commit/2f00410e7202556e37860cacbe64e37bef248ca7))
* timezone 設定 + Skill tool 支援 + approval 說明修正 ([9ad0559](https://github.com/CatiesGames/catclaw/commit/9ad05591d35e75a3e84ba0e72babbfc86044eeaa))
* tool approval system + channel forwarding + TUI/CLI improvements ([b51b893](https://github.com/CatiesGames/catclaw/commit/b51b89303e8debcfd917e04b7364bf9b9b282c94))
* unified tool permissions, MCP management, mouse scroll, UX improvements ([a862792](https://github.com/CatiesGames/catclaw/commit/a862792965812a7c64c4b2a47d45a62cc051b1f7))
* unify all runtime files under ~/.catclaw/ ([b9bc44e](https://github.com/CatiesGames/catclaw/commit/b9bc44ed8b725d81298d956c2c958a111b8ec302))
* 自動記憶系統 — 日記提取與長期蒸餾 ([ad77581](https://github.com/CatiesGames/catclaw/commit/ad77581bb6b5ba6201d6696c13a9d6f63b59276a))


### Bug Fixes

* code review 第二輪修復 + TUI transcript 讀取 bug ([9ccf272](https://github.com/CatiesGames/catclaw/commit/9ccf272b12b2cca1765b197dda07d65e62e734ce))
* enable kitty keyboard protocol for Shift+Enter newline ([9c909a3](https://github.com/CatiesGames/catclaw/commit/9c909a3429887506f3026b7845532809c435b774))
* remove openssl dependency, gate xml_escape with cfg(macos) ([f3b1d46](https://github.com/CatiesGames/catclaw/commit/f3b1d4689e4e27eabe386b2e1110dd9333179768))
* resolve all clippy warnings, switch reqwest to rustls-tls ([0eb9bd1](https://github.com/CatiesGames/catclaw/commit/0eb9bd1fc8ed553813ae71360915671b363f3697))
* resolve relative paths against config file directory ([87d5585](https://github.com/CatiesGames/catclaw/commit/87d5585c4f33333b4d82412ca3e184d2ccb947ee))
* retry Discord slash command registration on transient HTTP errors ([79b0154](https://github.com/CatiesGames/catclaw/commit/79b015411ff059c1c9fecc4076db0a6dea5d9bb7))
* session 建立時記錄 channel metadata 到 transcript ([767f646](https://github.com/CatiesGames/catclaw/commit/767f6463b43e0dca64dac9ec8ff92da55b470959))
* skip transcript for system sessions, use open_existing for diary ([b7f7ed5](https://github.com/CatiesGames/catclaw/commit/b7f7ed596dc8fc074d4cd70136f2cb2170e5ddaf))
* Slack manifest 參考 OpenClaw 補齊缺少的設定 ([a5f16f0](https://github.com/CatiesGames/catclaw/commit/a5f16f09e8ba285e3325f8c49f218dcbde0e4ef3))
* Slack manifest 改用 JSON 格式 + 移除邊框方便複製 ([12bd990](https://github.com/CatiesGames/catclaw/commit/12bd990f1a35dadd8a7e4575fa9a5c7ed3c69724))
* Slack manifest 補齊 app_home、app_mentions:read、files:read ([5448b16](https://github.com/CatiesGames/catclaw/commit/5448b1605039d517059f56d026eac6033f29b0a6))
* Slack onboard 改用 App Manifest 簡化設定流程 ([01fc257](https://github.com/CatiesGames/catclaw/commit/01fc25795f36bddb9f56240f8f149463b51a3ccd))
* Slack onboard 補充 App-Level Token scope 說明 ([d6b792a](https://github.com/CatiesGames/catclaw/commit/d6b792a368a0982d5c4cbea444d23777b6b60173))
* Slack thinking status 時機修正 + user_not_found fallback ([f9f562e](https://github.com/CatiesGames/catclaw/commit/f9f562e4ca4ce0bfcd04aa27eb430e38e6e4b0dc))
* tokio-tungstenite 啟用 TLS + ToolSearch 加入預設 allowed tools ([53fec49](https://github.com/CatiesGames/catclaw/commit/53fec4977d7c2c2c75169f3c342008154dd00cf5))
* use launchctl bootstrap/bootout instead of load/unload ([3a81938](https://github.com/CatiesGames/catclaw/commit/3a8193871ed5633234b316dbda95cd7d2a0c2488))
* write transcript with tool_use details, log user message immediately ([6ee15e8](https://github.com/CatiesGames/catclaw/commit/6ee15e8ca924055b6b58861c2e66b3a5ad40e8ac))
* 將 release build 整合進 release-please workflow ([1ca1675](https://github.com/CatiesGames/catclaw/commit/1ca1675518997aeb90606237f3ac3de5a17eb47f))
* 啟用 kitty keyboard protocol 後按鍵重複輸入 ([8717823](https://github.com/CatiesGames/catclaw/commit/8717823289f0a41e34a37919cfc2b2be9a3091d6))
* 版本號動態化、新增 version 子命令、輸入框動態高度、三層焦點模式 ([87a7cb9](https://github.com/CatiesGames/catclaw/commit/87a7cb927fe51e72d46989122ad46256b0fb1219))

## [0.8.3](https://github.com/CatiesGames/catclaw/compare/v0.8.2...v0.8.3) (2026-03-19)


### Bug Fixes

* Slack thinking status 時機修正 + user_not_found fallback ([f9f562e](https://github.com/CatiesGames/catclaw/commit/f9f562e4ca4ce0bfcd04aa27eb430e38e6e4b0dc))

## [0.8.2](https://github.com/CatiesGames/catclaw/compare/v0.8.1...v0.8.2) (2026-03-19)


### Bug Fixes

* Slack manifest 改用 JSON 格式 + 移除邊框方便複製 ([12bd990](https://github.com/CatiesGames/catclaw/commit/12bd990f1a35dadd8a7e4575fa9a5c7ed3c69724))
* Slack onboard 補充 App-Level Token scope 說明 ([d6b792a](https://github.com/CatiesGames/catclaw/commit/d6b792a368a0982d5c4cbea444d23777b6b60173))
* tokio-tungstenite 啟用 TLS + ToolSearch 加入預設 allowed tools ([53fec49](https://github.com/CatiesGames/catclaw/commit/53fec4977d7c2c2c75169f3c342008154dd00cf5))

## [0.8.1](https://github.com/CatiesGames/catclaw/compare/v0.8.0...v0.8.1) (2026-03-19)


### Bug Fixes

* Slack manifest 參考 OpenClaw 補齊缺少的設定 ([a5f16f0](https://github.com/CatiesGames/catclaw/commit/a5f16f09e8ba285e3325f8c49f218dcbde0e4ef3))
* Slack manifest 補齊 app_home、app_mentions:read、files:read ([5448b16](https://github.com/CatiesGames/catclaw/commit/5448b1605039d517059f56d026eac6033f29b0a6))
* Slack onboard 改用 App Manifest 簡化設定流程 ([01fc257](https://github.com/CatiesGames/catclaw/commit/01fc25795f36bddb9f56240f8f149463b51a3ccd))

## [0.8.0](https://github.com/CatiesGames/catclaw/compare/v0.7.0...v0.8.0) (2026-03-19)


### Features

* Slack channel adapter（Socket Mode + AI streaming） ([31d8e27](https://github.com/CatiesGames/catclaw/commit/31d8e27d30f7f2cd32264752ae0a5d2faab48143))

## [0.7.0](https://github.com/CatiesGames/catclaw/compare/v0.6.1...v0.7.0) (2026-03-18)


### Features

* local timezone display, task name lookup, one-shot auto-delete ([326ae9c](https://github.com/CatiesGames/catclaw/commit/326ae9c5961eb257dda8a027d361427268c9b4c6))

## [0.6.1](https://github.com/CatiesGames/catclaw/compare/v0.6.0...v0.6.1) (2026-03-18)


### Bug Fixes

* skip transcript for system sessions, use open_existing for diary ([b7f7ed5](https://github.com/CatiesGames/catclaw/commit/b7f7ed596dc8fc074d4cd70136f2cb2170e5ddaf))

## [0.6.0](https://github.com/CatiesGames/catclaw/compare/v0.5.1...v0.6.0) (2026-03-18)


### Features

* human-readable transcript filenames ([83ea1d4](https://github.com/CatiesGames/catclaw/commit/83ea1d406ed2268b742997a71024b98bc2faecc6))

## [0.5.1](https://github.com/CatiesGames/catclaw/compare/v0.5.0...v0.5.1) (2026-03-18)


### Bug Fixes

* retry Discord slash command registration on transient HTTP errors ([79b0154](https://github.com/CatiesGames/catclaw/commit/79b015411ff059c1c9fecc4076db0a6dea5d9bb7))

## [0.5.0](https://github.com/CatiesGames/catclaw/compare/v0.4.0...v0.5.0) (2026-03-17)


### Features

* timezone 設定 + Skill tool 支援 + approval 說明修正 ([9ad0559](https://github.com/CatiesGames/catclaw/commit/9ad05591d35e75a3e84ba0e72babbfc86044eeaa))

## [0.4.0](https://github.com/CatiesGames/catclaw/compare/v0.3.3...v0.4.0) (2026-03-17)


### Features

* Discord/Telegram slash commands (/stop, /new) + 統一 diary extraction ([cb4cf9c](https://github.com/CatiesGames/catclaw/commit/cb4cf9cc811fb3b60f3882283f41ce5f4494c2a7))
* task add --at 一次性排程 + agent scheduling 指引 ([2f00410](https://github.com/CatiesGames/catclaw/commit/2f00410e7202556e37860cacbe64e37bef248ca7))

## [0.3.3](https://github.com/CatiesGames/catclaw/compare/v0.3.2...v0.3.3) (2026-03-17)


### Bug Fixes

* 啟用 kitty keyboard protocol 後按鍵重複輸入 ([8717823](https://github.com/CatiesGames/catclaw/commit/8717823289f0a41e34a37919cfc2b2be9a3091d6))

## [0.3.2](https://github.com/CatiesGames/catclaw/compare/v0.3.1...v0.3.2) (2026-03-17)


### Bug Fixes

* 版本號動態化、新增 version 子命令、輸入框動態高度、三層焦點模式 ([87a7cb9](https://github.com/CatiesGames/catclaw/commit/87a7cb927fe51e72d46989122ad46256b0fb1219))

## [0.3.1](https://github.com/CatiesGames/catclaw/compare/v0.3.0...v0.3.1) (2026-03-17)


### Bug Fixes

* 將 release build 整合進 release-please workflow ([1ca1675](https://github.com/CatiesGames/catclaw/commit/1ca1675518997aeb90606237f3ac3de5a17eb47f))

## [0.3.0](https://github.com/CatiesGames/catclaw/compare/v0.2.2...v0.3.0) (2026-03-17)


### Features

* 自動記憶系統 — 日記提取與長期蒸餾 ([ad77581](https://github.com/CatiesGames/catclaw/commit/ad77581bb6b5ba6201d6696c13a9d6f63b59276a))


### Bug Fixes

* enable kitty keyboard protocol for Shift+Enter newline ([9c909a3](https://github.com/CatiesGames/catclaw/commit/9c909a3429887506f3026b7845532809c435b774))
* session 建立時記錄 channel metadata 到 transcript ([767f646](https://github.com/CatiesGames/catclaw/commit/767f6463b43e0dca64dac9ec8ff92da55b470959))
* write transcript with tool_use details, log user message immediately ([6ee15e8](https://github.com/CatiesGames/catclaw/commit/6ee15e8ca924055b6b58861c2e66b3a5ad40e8ac))
