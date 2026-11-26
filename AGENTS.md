# Goal

精简 AGY Plus，仅提供 Antigravity RT 登录官方 agy CLI。

# Current State

- 分支 rt-login，版本 0.1.0；从 Cockpit Tools Clean 1.4.2 提取凭据写入协议。
- Tauri 2 + 原生 TypeScript 单窗口；无 React、其他平台、账号库、代理、网关和后台任务。
- 写入系统凭据和两个 agy 共享凭据文件；会同时影响 App / 新版 IDE。
- Mac 源码：`/Users/meta/Developer/agy-plus`；ROG 源码：`C:\Users\Meta\source\repos\agy-plus`，均使用 rt-login。
- Mac 当前 PATH 未检测到 agy；ROG 检测到 agy 1.5.18；真实 RT 登录及 CLI 接受尚待用户验收。
- Mac arm64 构建、5 项模拟登录/文件写入测试、固定 certificate leaf 签名、桌面启动及无效 RT 的真实 Google HTTP 400 错误流程已通过。

- ROG Windows x64 release 构建、5 项测试通过；EXE 0.1.0 启动后进程存活、输入空闲和窗口创建检查通过。
- Windows 便携包：target/portable/agy-plus-0.1.0-windows-x64.zip；未做 Authenticode 签名，依赖系统 WebView2。
# Active Work

- Linux 编译、各端真实 RT 登录未验证。

# Build / Run / Test

- npm ci；npm run typecheck；npm run build。
- cargo test --locked -p agy-plus。
- Windows：npm run build:win，复用已安装 MSVC；输出 target/portable/*.zip；需要已有 WebView2 Runtime。
- npm run build:mac：固定证书 Local Development Code Signing，identifier com.alexbeatsz.agy-plus，验证 certificate leaf。

# Durable Lessons

- CLI 登录必须创建 antigravity-cli/antigravity-oauth-token，不能只更新桌面回退文件。
- 多存储写入失败要报告部分成功，禁止伪报全部登录成功。

# Docs Index

- docs/design/rt-login.md：协议、写入范围和验证边界。
