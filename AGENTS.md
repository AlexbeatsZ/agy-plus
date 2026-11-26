# Goal

精简 AGY Plus，仅提供 Antigravity RT 登录官方 agy CLI。

# Current State

- 分支 rt-login，版本 0.1.0；从 Cockpit Tools Clean 1.4.2 提取凭据写入协议。
- Tauri 2 + 原生 TypeScript 单窗口；无 React、其他平台、账号库、代理、网关和后台任务。
- 写入系统凭据和两个 agy 共享凭据文件；会同时影响 App / 新版 IDE。
- Mac 当前 PATH 未检测到 agy；真实 RT 登录及 CLI 接受尚待用户验收。

# Active Work

- Mac arm64 构建通过；5 项模拟登录/文件写入测试通过；签名包含固定 certificate leaf。
- 浏览器 UI 输入/显隐/桌面环境提示已验证；签名桌面 app 启动及虚构 RT 的真实 Google HTTP 400 拒绝流程通过。
- Windows / Linux 编译与真机登录未验证。

# Build / Run / Test

- npm ci；npm run typecheck；npm run build。
- cargo test --locked -p agy-plus。
- npm run build:mac：固定证书 Local Development Code Signing，identifier com.alexbeatsz.agy-plus，验证 certificate leaf。

# Durable Lessons

- CLI 登录必须创建 antigravity-cli/antigravity-oauth-token，不能只更新桌面回退文件。
- 多存储写入失败要报告部分成功，禁止伪报全部登录成功。

# Docs Index

- docs/design/rt-login.md：协议、写入范围和验证边界。
