# AGY Plus

只做一件事：使用 Antigravity Refresh Token（RT）登录官方 `agy` CLI。

打开桌面程序，粘贴单个 RT，点击「登录 agy」。验证成功后显示账号邮箱（若 Google 返回），清空输入框。重新打开 agy CLI 会话以加载新登录。程序不会启动 CLI、执行推理或消耗模型请求额度。

## 登录范围

沿用 Cockpit Tools 的 Antigravity OAuth client 和凭据格式，调用 Google token 接口验证 RT，再写入：

- macOS Keychain：service `gemini` / account `antigravity`，使用 `go-keyring-base64:` 编码。
- Windows Credential Manager：`gemini:antigravity`。
- Linux Secret Service：通过已安装的 `secret-tool` 写入。
- CLI 文件：`~/.gemini/antigravity-cli/antigravity-oauth-token`，首次登录也创建。
- 桌面回退文件：`~/.gemini/jetski-standalone-oauth-token`。

Antigravity App 和采用共享登录的新版 IDE 也会受影响。已经运行的进程可能缓存旧账号，需要重新打开。仅接受与 Antigravity OAuth client 对应的 Google RT；不支持其他客户端签发的 RT、API Key 或企业专用认证。使用自定义 HOME / app_data_dir 的 CLI 实例不在此版本范围内。

凭据写入使用 agy 已有目录。应用无账号库、RT 历史、后台服务、代理设置或自动更新。RT 不写入日志；Unix 凭据文件以 0600 原子替换。系统凭据与两个文件不能跨存储原子提交，部分失败会明确提示重试。

## 开发

建议用 fnm 管理 Node.js、rustup 管理 Rust。本次构建复用 Mac 已有 Node.js，未安装或切换全局运行时。使用现有 lockfile：

```bash
npm ci
npm run dev
npm run tauri -- dev
npm run typecheck
cargo test --locked -p agy-plus
```

macOS 构建要求固定证书 `Local Development Code Signing`，固定 bundle identifier `com.alexbeatsz.agy-plus`：

```bash
npm run build:mac
```

产物为 `target/release/bundle/macos/AGY Plus.app`。脚本验证签名及 `certificate leaf`。本地证书签名未经过 Apple 公证，跨 Mac 分发需自行信任签名。

Windows / Linux 源码路径保留，但需要在对应设备上构建和验收；Linux 依赖系统 WebKitGTK 与 libsecret-tools。

## 来源

由 AlexbeatsZ/agy-plus 的 Cockpit Tools Clean 1.4.2 精简，保留 Antigravity 凭据协议、平台写入实现和受限超时进程工具。遵循 [原项目许可](LICENSE)，署名见 [NOTICE.md](NOTICE.md)。
