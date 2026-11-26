# AGY Plus

面向 macOS 与 Windows 官方 Google Antigravity 桌面应用的无侵入 CDP 注入启动器（学习 `codex-plus` 架构）。无需修改 `app.asar` 或破坏官方签名，直接将原生风格的 Refresh Token 登录与多账号切换能力注入 Antigravity 界面，同时保持官方 `agy` CLI 凭据同步。

## 功能

- **未登录界面 RT 登录**：在官方登录卡片（`Continue with Google` / `Use Business Account` 下方）注入 `Continue with Refresh Token` 按钮；验证 Google Refresh Token 后自动写入系统凭据与共享 token 文件并重启进入登录态。若本地已有保存账号，也可在弹窗内直接选择登录。
- **Settings → Account 切换账号**：在 `Settings → Account` 的 `Email` 行 `Sign Out` 旁注入同规格 `Switch Account` 按钮：
  - 自动捕获并保存已登录账号（**官方 Google OAuth 登录与 RT 登录过的账号都会统一管理**）。
  - 一键切换账号：自动快照当前凭据 → 优雅停止 Antigravity → 同步写入系统凭据、`~/.gemini/antigravity-cli/antigravity-oauth-token` 和 `~/.gemini/jetski-standalone-oauth-token` → 重新以 CDP 模式启动 Antigravity。
  - 支持在弹窗内继续添加 RT 账号（立即切换或仅保存）、触发官方 Google 登录添加新账号，以及删除非当前本地快照。
- **契合 Antigravity 原生视觉**：直接继承 Antigravity 的主题 CSS 变量、字体与圆角规范，自动适配深浅主题与自定义配色；英文文案与原生界面一致。

## 数据目录

账号快照、锁与日志保存在平台应用数据目录（不写入源码仓库）：
- macOS：`~/Library/Application Support/agy-plus`
- Windows：`%LOCALAPPDATA%\agy-plus`
- Linux：`~/.local/share/agy-plus`

令牌仅保存在本机（Unix `0600`），不返回给页面脚本，也不写入日志。

## 构建与使用

### macOS

依赖固定代码签名证书 `Local Development Code Signing` 与固定 identifier `com.alexbeatsz.agy-plus`：

```bash
npm ci
npm run verify
npm run build:mac
```

构建产物：`target/release/agy-plus` 与 `target/portable/agy-plus-0.2.0-macos-arm64.tar.gz`。
- 手动启动：`open "macos/AGY Plus.app"` 或直接运行 `target/release/agy-plus`。
- 可选登录自动接管（LaunchAgent，含 `ProcessType=Interactive` 与 `AbandonProcessGroup=true`）：`bash macos/install.sh`（卸载：`bash macos/uninstall.sh`）。

### Windows x64

```powershell
npm ci
npm run build:win
```

脚本自动进入已安装的 MSVC x64 Developer Shell 构建，产物位于 `target\x86_64-pc-windows-msvc\release\agy-plus.exe` 与 `target\portable\agy-plus-0.2.0-windows-x64.zip`。
- 手动启动：直接运行 `agy-plus.exe`。
- 可选登录自动接管（`HKCU\...\Run`）：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows-startup.ps1`（卸载加 `-Uninstall`）。

### 开发者参数

- `--attach-existing` / `--attach-existing-port=<port>`：不重启 Antigravity，直接附加已有 CDP 端口（默认 `9239`）。
- `--login-startup`：后台静默等待用户启动官方 Antigravity 后再接管。
- `--version`：打印版本号。

## 来源

由 `AlexbeatsZ/agy-plus` 演进，凭据协议继承自 Cockpit Tools Clean 1.4.2，CDP 注入与生命周期架构参考 `codex-plus`。遵循 [原项目许可](LICENSE)，署名见 [NOTICE.md](NOTICE.md)。
