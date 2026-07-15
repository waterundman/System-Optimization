# optimizer-desktop

Optimizer System 的 Tauri 2 桌面适配层。当前阶段提供可编译的 IPC command 注册、main window capability、结构化错误和 Secret Store 命令；前端编辑器与项目打开/创建流程将在下一阶段接入。

WebView 只允许：

- 原子写入 Operation bundle；
- 写入 Patch review 事件；
- 读取 Operation 审计；
- 写入、检查和删除 provider secret。

WebView 不存在“读取 secret 明文”命令。模型请求执行器后续应在 Rust 信任边界内解析 `credentialRef`，或使用一次性宿主网络命令，不能把 API Key 返回 JavaScript。
