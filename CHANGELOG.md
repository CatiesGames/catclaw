# Changelog

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
