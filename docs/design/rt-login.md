# agy CLI RT 登录

入口 login_with_rt 先清理外部空白，拒绝多行/超长 RT，然后向 Google OAuth token endpoint 发送 refresh_token grant。保留原工程 Antigravity client id / native public client secret（公开桌面客户端标识，不是用户密钥）。无浏览器 OAuth、配额探测或模型调用。

有效响应要求 access_token 非空、expires_in 在 1 秒到 7 天之间；保留 rotation 后 RT 与 id_token。userinfo 查询失败不阻止有效凭据写入。所有网络错误与上游错误正文经过收敛，避免回显凭据。

写入顺序：系统凭据 → CLI 会话 → 桌面回退。使用原平台实现；macOS 保留 Keychain access policy，不额外授予任意程序访问。子进程最长 15 秒。文件使用同目录临时文件 + persist 原子替换，Unix 0600。全流程无应用账号库、持久日志、代理设置和进程重启。

CLI 会话目录固定为默认 HOME/.gemini/antigravity-cli，且第一次登录必须创建。自定义 app_data_dir 与旧版 SQLite IDE 不处理。系统存储与两个文件无法原子提交；任何后续失败返回明确的部分成功信息，用户可重试。

测试只用临时目录和 localhost 模拟 Google，系统写入注入 mock。禁止测试覆盖用户的真实凭据。真实 CLI 接受需要用户有效 RT 和已安装的官方 agy，未完成前不能声称登录实测成功。
