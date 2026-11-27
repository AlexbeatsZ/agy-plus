# AGY Plus CDP 注入架构与账号切换设计

## 目标与架构

`agy-plus` 是面向 macOS 与 Windows 官方 Google Antigravity 桌面应用的纯 Rust CDP 注入启动器（学习 `codex-plus` 模式），不修改 `app.asar`、不破坏官方应用签名，也没有独立管理窗口。

启动模式：
- 默认显式启动：停止现有 Antigravity / `agy-plus` 实例 → 获取独占锁 → 以 `--remote-debugging-port`（默认 `9239`，`AddrInUse` 有界重试后回退动态端口）启动官方 Antigravity → 启动多窗口 CDP watchdog 与常驻监控循环。
- `--login-startup`：仅持锁静默等待用户自行打开官方 Antigravity，发现无 CDP 参数的根进程后受控重启为 CDP 模式；不主动弹窗。
- `--attach-existing` / `--attach-existing-port=<port>`：不重启应用，直接附加到已有 CDP 端口。
- `--apply-pending-action`：内部事务 helper，按 `pending-action.json`（不含任何 token）执行账号切换或新增 Google 登录重启流程。
- `--web-login`：在触发“Add Google Account”后启动官方 Antigravity 登录页，并在后台捕获新登录凭据到账号库。

## 数据目录与凭据协议

- 应用状态目录遵循平台规范（可通过 `AGY_PLUS_STATE_DIR` 覆盖用于隔离测试）：
  - macOS：`~/Library/Application Support/agy-plus`
  - Windows：`%LOCALAPPDATA%\agy-plus`
  - Linux：`$XDG_DATA_HOME/agy-plus` 或 `~/.local/share/agy-plus`
- 账号快照：`accounts/<record_id>.json`（Unix `0600`，`fs2` 排他锁）。`record_id` 由 `id_token` JWT 的 `sub`（或 Google `userinfo.id`）派生 `sha256("google-sub:...")`，缺省时依次回退 `email`、`refresh_token`、`access_token`，确保同一 Google 账号通过官方 OAuth 和 RT 登录自动去重合并。
- 凭据同步范围：
  1. 系统凭据存储：macOS Keychain（`gemini`/`antigravity`，`go-keyring-base64:`，`-U` 原地更新不改动 ACL）、Windows Credential Manager（`gemini:antigravity`）、Linux `secret-tool`。
  2. CLI 会话：`~/.gemini/antigravity-cli/antigravity-oauth-token`（首次登录也会创建）。
  3. 桌面回退：`~/.gemini/jetski-standalone-oauth-token`。
- 读取当前实时凭据：Windows 直接无弹窗调用 `CredReadW` 并回退文件；macOS 优先读取较新的有效 token 文件，仅在文件缺失或与界面邮箱不一致且非弹窗元数据探测确认 Keychain 存在时才读取 Keychain，避免日常打开弹窗触发 Keychain 授权提示。
- 切换与新增登录事务：先快照当前实时凭据并保留三处存储的备份，停止 Antigravity（Unix 先发送 `SIGTERM` 等待优雅退出再强杀）后写入目标凭据或清空实时凭据；失败时按备份回滚并恢复启动。

## CDP 目标筛选与页面注入

- 目标筛选：匹配 `type == "page"` 且 URL 为 `https://127.0.0.1:<port>/...`（或测试用 `http://127.0.0.1:<port>/...`），排除 `data:`（curtain/catcher）、`devtools://`、外部 URL 与 `webview`；支持多窗口 `JoinSet` 并发会话。
- 预加载：每个注入 CDP 会话必须先 `Runtime.enable` + `Page.enable`，再注册 `Runtime.addBinding("agyPlusBridgeV1")` 与 `Page.addScriptToEvaluateOnNewDocument`（先移除旧注册 ID）。0.2.0 漏掉 `Page.enable`，真实 Chromium 实测注册成功但刷新后预加载不执行，首次手动 evaluate 的脚本随 document 消失。
- 页面生命周期：预加载运行时 `<html>` / `<head>` 可能尚不存在；观察器挂在稳定的 `document` 上，等 head 出现后补样式与按钮。观察相关节点的新增和删除，支持根节点替换及 React 清理注入节点；销毁时取消待扫描工作。脚本仅在 loopback Hub 顶层页面执行，不进入 iframe 或外部 OAuth 页面。
- 防假存活与自恢复：15 秒一次带命令 ID 确认回包的心跳（45 秒无回包判定、10 秒写超时）；每轮执行幂等 install，并校验 renderer、observer、bridge 真实健康。回包为空、JS 异常或桥丢失时结束会话，由 watchdog 重连并完整重注入，不能只因 CDP 有回包就判定健康。页面上下文清空后立即检查；桥响应绑定原始 `executionContextId`，不会把导航前的请求结果送入新页面。

- 样式与两个注入点（纯英文，直接复用 Antigravity 原生 CSS 变量 `--card`、`--background`、`--foreground`、`--muted-foreground`、`--border`、`--primary`、`--secondary`、`--error`、`--success`、`--scrim` 与 `--font-sans`）：
  1. 未登录页（`.entrance-auth-panel` / `#other-sign-in-options` 及 `Settings → Account` 的 `Not signed in` 行）：注入 `Continue with Refresh Token` / `Sign in with RT`，点击直接展示 RT 输入表单，并在下方列出已有本地保存账号供一键登录。
  2. `Settings → Account` 的 `Email` 行 `Sign Out` 旁：注入同高度（28px）`Switch Account` 按钮，点击弹出原生风格账号列表（同时包含普通 Google OAuth 登录和 RT 登录过的账号），支持切换、内联二次确认删除非当前账号、弹窗内新增 RT（支持立即切换或仅保存）与新增 Google 账号。
- SSH 远程窗口（`window.__APP_CONFIG__.isRemote` / `sshHost`）不注入本地账号入口。

## 注入生命周期回归

- `npm run verify`：JS 语法、JSDOM 与 Rust 单元/模拟 CDP 测试。
- `AGY_PLUS_TEST_ELECTRON=<已有 Electron 可执行文件> AGY_PLUS_TEST_LAUNCHER=<构建产物> npm run test:cdp`：在平台临时目录 `.agents/agy-plus-injection-*` 创建隐藏的隔离 Electron fixture，测试连续登录页/账号页导航、解析早期注入、刷新后 bridge、iframe 排除与丢失脚本后 watchdog 恢复。临时 home/state/keyring 与当前 Antigravity 完全独立，结束清理。
- `node tests/cdp-smoke.mjs --baseline` 可指定 0.2.0 二进制复现：初次注入成功，刷新后 renderer/bridge 都为 undefined；等待超过一轮心跳依然缺失。普通 JSDOM 与只返回健康对象的模拟 CDP 测试无法覆盖 `Page.enable` 漏配。
