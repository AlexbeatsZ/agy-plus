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
- 预加载与防假存活：使用 `Runtime.addBinding("agyPlusBridgeV1")` + `Page.addScriptToEvaluateOnNewDocument`（先移除旧注册 ID），保证语言服务器自重启重载页面后自动重注入；15 秒一次带命令 ID 确认回包的心跳（45 秒失活判定、10 秒写超时）应对 macOS 睡眠唤醒假存活。
- 样式与两个注入点（纯英文，直接复用 Antigravity 原生 CSS 变量 `--card`、`--background`、`--foreground`、`--muted-foreground`、`--border`、`--primary`、`--secondary`、`--error`、`--success`、`--scrim` 与 `--font-sans`）：
  1. 未登录页（`.entrance-auth-panel` / `#other-sign-in-options` 及 `Settings → Account` 的 `Not signed in` 行）：注入 `Continue with Refresh Token` / `Sign in with RT`，点击直接展示 RT 输入表单，并在下方列出已有本地保存账号供一键登录。
  2. `Settings → Account` 的 `Email` 行 `Sign Out` 旁：注入同高度（28px）`Switch Account` 按钮，点击弹出原生风格账号列表（同时包含普通 Google OAuth 登录和 RT 登录过的账号），支持切换、内联二次确认删除非当前账号、弹窗内新增 RT（支持立即切换或仅保存）与新增 Google 账号。
- SSH 远程窗口（`window.__APP_CONFIG__.isRemote` / `sshHost`）不注入本地账号入口。
