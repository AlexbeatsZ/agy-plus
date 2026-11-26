# Goal

面向 macOS 与 Windows 官方 Google Antigravity 桌面应用的无侵入 CDP 注入启动器（学习 `codex-plus` 架构）：不修改 `app.asar`，直接将与 Antigravity 原生风格一致的 Refresh Token 登录（未登录页）与多账号切换（`Settings → Account`）注入官方界面，同时保持官方 `agy` CLI 凭据同步。

# Current State

- 分支 `rt-login`，版本 `0.2.0`；由原独立 Tauri 窗口重构为纯 Rust CDP 注入启动器 + 内嵌原生风格前端脚本（`assets/bridge.js`、`assets/renderer.js`）。
- 支持多窗口 CDP watchdog（默认端口 `9239`，`AddrInUse` 有界重试后回退动态端口；`Page.addScriptToEvaluateOnNewDocument` + 15s 确认回包心跳）、本地账号库（统一管理官方 Google OAuth 与 RT 登录过的账号，按 `id_token` `sub` 去重）、账号切换与新增 Google 登录重启事务（失败自动回滚）。
- 数据目录遵循平台规范（不写进仓库）：macOS `~/Library/Application Support/agy-plus`，Windows `%LOCALAPPDATA%\agy-plus`，Linux `~/.local/share/agy-plus`（测试可用 `AGY_PLUS_STATE_DIR` / `AGY_PLUS_HOME` / `AGY_PLUS_TEST_KEYRING_FILE` 隔离）。
- 凭据写入仍同步三处：系统凭据（macOS Keychain `gemini`/`antigravity`、Windows Credential Manager `gemini:antigravity`、Linux `secret-tool`）、`~/.gemini/antigravity-cli/antigravity-oauth-token`、`~/.gemini/jetski-standalone-oauth-token`。
- Mac（`/Users/meta/Developer/agy-plus`）：8 项 JS 测试、18 项 Rust 测试、arm64 固定证书签名构建（`Local Development Code Signing` / `com.alexbeatsz.agy-plus`，已验 `certificate leaf`）与隔离实时 CDP 冒烟测试（含真实 Google HTTP 400 错误流程）全部通过；未安装 LaunchAgent，未重启当前运行中的 Antigravity。
- ROG Windows（`C:\Users\Meta\source\repos\agy-plus`）：检测到官方 `Antigravity 2.29.0`（`%LOCALAPPDATA%\Programs\Antigravity\Antigravity.exe`）；8 项 JS 测试、16 项 Rust 测试、MSVC x64 release 构建（`agy-plus.exe` 0.2.0，`target/portable/agy-plus-0.2.0-windows-x64.zip`）与隔离实时 CDP 冒烟测试全部通过；未做 Authenticode 签名，未写入 `HKCU\...\Run`。

# Active Work

- 用户显式启动新版 `agy-plus`（或安装自启接管）后，使用真实 Google/RT 账号完成官方 Antigravity 实机切换与目视验收。
- Linux 编译与实机验收未执行。

# Build / Run / Test

- 完整验证：`npm ci && npm run verify`（含 JS 语法检查、JSDOM 测试与 `cargo test --locked`）。
- macOS 构建与签名：`npm run build:mac`（调用 `macos/release.sh` → 隔离 target 构建 → `macos/sign.sh` 固定证书签名 → 原子替换 `target/release/agy-plus` 并打包 `target/portable/agy-plus-0.2.0-macos-arm64.tar.gz`）。手动启动：`open "macos/AGY Plus.app"`；可选登录自启：`bash macos/install.sh`（卸载 `bash macos/uninstall.sh`）。
- Windows 构建：`npm run build:win`（调用 `scripts/build-windows.ps1`，自动进入 MSVC x64 Developer Shell，输出 `target/portable/agy-plus-0.2.0-windows-x64.zip`）。可选登录自启：`scripts/install-windows-startup.ps1`。

# Durable Lessons

- React 渲染的相邻 JSX 元素之间没有空白字符（例如 `Email` 行 `textContent` 为 `Emailalice@gmail.com`，带 `Last Used` 徽标的登录按钮为 `Continue with GoogleLast Used`）。DOM 匹配必须按独立子节点/叶节点提取标签与邮箱，不能对整列拼接文本做 `\b` 边界或邮箱正则匹配。
- Antigravity 官方 `compositeTokenStorage` 同时更新系统凭据与 `~/.gemini/jetski-standalone-oauth-token`。macOS 常规读取优先读较新的有效 token 文件，仅在文件缺失或与界面邮箱不一致且非弹窗元数据检查确认 Keychain 存在时才读 Keychain，避免日常打开弹窗触发授权提示。
- CLI 登录必须创建 `antigravity-cli/antigravity-oauth-token`，不能只更新桌面回退文件；多存储写入失败须报告部分成功或回滚。

- 旧版 0.1.0 Tauri 构建会在 `target/release/bundle/macos/AGY Plus.app` 留下 `.app`，Spotlight/LaunchServices 会优先索引它而打开独立窗口旧版；重构后须删除该目录并 `lsregister -u` 注销，Mac 唯一入口为 `macos/AGY Plus.app`（`com.alexbeatsz.agy-plus.app`）。

# Docs Index

- `docs/design/cdp-inject.md`：CDP 注入架构、数据目录、凭据同步协议与页面注入点设计。
