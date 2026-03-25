# Changelog

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
