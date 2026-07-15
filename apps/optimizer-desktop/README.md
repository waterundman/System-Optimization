# optimizer-desktop

Optimizer System 的 Tauri 2 桌面适配层。当前提供可编译的 IPC command 注册、main window capability、结构化错误、项目 Session 与 Secret Store 命令；前端编辑器将在后续阶段接入。

WebView 只允许：

- 创建、打开、关闭和查看一个已校验的本地项目会话；
- 原子写入 Operation bundle；
- 写入 Patch review 事件；
- 读取 Operation 审计；
- 写入、检查和删除 provider secret。

WebView 不存在“读取 secret 明文”命令。模型请求执行器后续应在 Rust 信任边界内解析 `credentialRef`，或使用一次性宿主网络命令，不能把 API Key 返回 JavaScript。

## 项目会话

- `create_project` 在安全父目录内原子创建 `.optimizer` 项目包并打开；
- `open_project` 校验 manifest、SQLite invariant、项目 ID 与主分支绑定后打开；
- `close_project` 释放当前数据库连接；
- `get_project_session` 返回当前项目、主分支、HEAD、revision 与数据库 schema；
- Operation 写入和审计命令在无项目时统一返回 `NO_PROJECT_OPEN`。

四个命令位于独立的 `allow-project-session` permission 中，只授权给本地 `main` window。前端项目选择器和编辑器壳仍待接入。
