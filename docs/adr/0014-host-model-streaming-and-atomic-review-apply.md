# ADR 0014：宿主模型流与原子 Patch 应用

- 状态：Accepted
- 日期：2026-07-15
- 决策范围：M2 桌面 AI 操作闭环

## 背景

桌面工作区已经具备版本化编辑、Secret Store、Context Compiler、Operation Runner、Patch Engine 与 Operation 审计存储，但模型网络请求仍需要进入 Rust 信任边界，AI 工具栏也尚未形成可用闭环。

必须同时满足：API Key 不返回 WebView、供应商地址不可由页面注入、流式生成可取消、Operation 产物先审查后落盘，以及正文编辑与 Review/Operation 状态不能出现部分成功。

## 决策

1. Rust `optimizer-host` 实现统一 `ModelExecutionHost`，固定支持 DeepSeek、Qwen、Kimi、MiniMax 的官方 HTTPS 端点与方言映射。WebView 只能提交 `credentialRef`，宿主从 Secret Store 解析明文。
2. Windows 传输使用 WinHTTP，同步请求在 blocking task 中执行；活动请求保存可关闭的 request handle，因此取消和超时可中断底层 I/O。Authorization 的 UTF-16 临时缓冲在发送后清零。
3. Tauri 使用 `Channel<ModelStreamEvent>` 传递 start、text/reasoning delta、usage 和 finish；命令错误只暴露供应商、HTTP 状态、远端错误码、request id、retry-after 与 retriable，不包含请求头或密钥。
4. 桌面构建利用 Node 24 的类型剥离能力，把仓库中现有 TypeScript Kernel、Context Compiler、Operation Runner 和 Patch Engine 转成浏览器 ES 模块。UI 不另写一套提示词、状态机或 diff 逻辑。
5. 非敏感供应商偏好（模型名、Qwen 区域、启用状态）保存在 WebView 的 machine-local storage；API Key 只写入操作系统凭据库。Rust 强制 `credentialRef` 精确匹配 `secret://providers/{provider}/default`，避免跨供应商 confused-deputy。
6. `apply_reviewed_proposal` 由宿主重新读取不可变 Proposal、校验 SHA-256、目标基线、UTF-16 anchor、hunk 源文本、全部审查决策与 atomic group，然后在同一 SQLite transaction 中写入 Block/EditJournal、`ai_accept` Commit、Review apply 事件和 Operation accepted 状态。
7. 关闭项目时取消全部活动模型请求；打开项目、写入审查决策和应用 Proposal 都继续使用单项目 Session 与 optimistic concurrency。
8. Context Compiler 完成后，桌面端必须先向用户展示实际 Context Packet（层级、来源、内容、必需标记、排除项和估算 token）。只有显式确认后才能调用 `execute_model_stream`；取消确认会中止整个 Operation，且不产生云端请求。

WinHTTP 的 request handle 生命周期遵循微软文档；Tauri Channel 的传输方式遵循 Tauri v2 IPC 文档：

- https://learn.microsoft.com/en-us/windows/win32/api/winhttp/nf-winhttp-winhttpopenrequest
- https://learn.microsoft.com/en-us/windows/win32/winhttp/hinternet-handles-in-winhttp
- https://docs.rs/tauri/latest/tauri/ipc/struct.Channel.html

## 结果

- API Key 不进入 Operation bundle、ContextPacket、日志、Channel 或 CommandError。
- 四家供应商共享统一请求、事件、取消、超时和错误模型，同时保留各自 reasoning、token 字段、Qwen region/workspace 与 MiniMax cumulative stream 差异。
- AI 操作已形成“选区/Block → ContextPacket → 流式模型 → 严格 JSON → Patch/Findings → 决策 → 原子应用”的桌面闭环。
- 用户在付费和数据出站前能审查实际编译结果；预览测试断言确认发生在宿主模型调用之前，浏览器回归覆盖预览确认路径。
- 正文修改和审计状态不会因进程中断或后半段失败而分叉。

## 代价与后续

- 当前 Operation Runner 与 Context Compiler 运行在受 CSP 限制的 bundled WebView；模型网络与密钥在 Rust，但 WebView 仍能调用 main-window 的 model command。后续应把“编译 ContextPacket + 生成模型请求”的授权收紧为宿主 Operation capability，或整体迁入 Rust，以便对 compromised WebView 也强制执行 `never_send` 策略。
- Node `stripTypeScriptTypes` 仍会输出 experimental warning，因此 Node 24 是明确的构建前提；如 API 发生变化，回退到锁定版本的离线 bundler。
- 当前正文输入仍是 `contenteditable`。逐 hunk 审查使用稳定 Block/UTF-16 anchor，但复杂富文本 decoration 等待 Tiptap 适配器。

## 回滚

可以撤销 `allow-model-execution` capability 与桌面 AI 工具栏，保留 Host/Store 数据结构和已有 Operation 审计；普通编辑、自动保存、检查点与恢复不依赖模型执行链路。
