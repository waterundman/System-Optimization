# optimizer-desktop

Optimizer System 的 Tauri 2 桌面应用。当前已经具备可运行的中文入口、项目创建/打开、文档树、Block 编辑与自动保存、冲突草稿保护、检查点、版本历史和恢复交互；所有持久化仍通过 Rust IPC 安全边界完成。

WebView 只允许：

- 创建、打开、关闭和查看一个已校验的本地项目会话；
- 原子写入 Operation bundle；
- 写入 Patch review 事件；
- 读取 Operation 审计；
- 写入、检查和删除 provider secret。
- 通过一次性、短期、绑定当前项目 HEAD/Context Packet/目标 Block 的 Rust capability，执行 DeepSeek、Qwen、Kimi、MiniMax 固定 HTTPS 端点，以及 Ollama 固定回环端点的流式请求与取消。
- 复用 Kernel/Operation Runner/Patch Engine 完成 AI 操作、Findings 与逐 hunk 审查。
- 原子应用已审查 Proposal，同时写入 `ai_accept` Commit 与完整 Operation/Review 审计。
- 读取文档树并用版本基线保存 Block；
- 建立检查点、浏览版本元数据并恢复为新 Commit。

WebView 不存在“读取 secret 明文”命令。云端模型请求由 Rust 信任边界解析 `credentialRef` 并执行，API Key 不会返回 JavaScript。

WebView 也不存在原始 `execute_model_stream` 命令。用户确认 Context 后，`authorize_model_request` 先由 Host 校验项目、HEAD、目标 Block、Provider locality 和完整模型请求，再返回最多 120 秒有效且只可消费一次的 capability；`execute_authorized_model_stream` 只接收 capability ID。项目变化、取消或关闭会使授权失效。

Ollama 是无凭据的显式本地 Provider：宿主只连接 `127.0.0.1:11434/v1/chat/completions` 并禁用系统代理，不接受页面提交的地址。模型需由用户预先在 Ollama 中拉取；运行失败不会自动把本地 Context Packet 发往云端。

用户可在 Provider 抽屉显式执行一次本地模型检测。`list_ollama_models` 仍在 Rust 内固定请求 `/v1/models`，限制响应体并校验 model ID；WebView 只获得安全的建议列表，不能提交探测 URL。

## 项目会话

- `create_project` 在安全父目录内原子创建 `.optimizer` 项目包并打开；
- `open_project` 校验 manifest、SQLite invariant、项目 ID 与主分支绑定后打开；
- `close_project` 释放当前数据库连接；
- `get_project_session` 返回当前项目、主分支、HEAD、revision 与数据库 schema；
- Operation 写入和审计命令在无项目时统一返回 `NO_PROJECT_OPEN`。

四个命令位于独立的 `allow-project-session` permission 中，只授权给本地 `main` window。入口表单只提交用户明确填写的绝对路径，创建与打开规则由 Host 再次校验。

## 工作区与版本

- `get_project_workspace` 返回文档树、结构化 Block、稳定 hash/revision 和当前 HEAD；
- `save_block` 使用 Block 与项目 HEAD 双层乐观并发，成功时原子生成 EditJournal 与 autosave Commit；
- `create_checkpoint` 为当前 HEAD 建立带 checksum 的物化快照；
- `get_version_history` 只返回 Commit/检查点元数据；
- `restore_checkpoint` 校验快照并“恢复为新 Commit”，不会覆盖历史节点。

工作区读写和版本读写分别拥有独立 permission。前端以 900ms 停顿触发串行保存，对 `CONFLICT` 保留本地草稿并重新加载权威工作区，对 `NO_CHANGES` 静默结束保存状态。未处理冲突会阻止建立检查点与恢复；关闭项目需要用户再次确认。

风格库使用独立 `allow-style-library` permission。选中的正文可固定为项目样本；启用样本作为 L4 Context 候选，归档样本和远程调用中的 `never_send` 样本由 Context Compiler 在模型调用前排除。实际入选内容始终出现在发送确认弹窗中。

文档侧栏可创建版本化章节。原生文件选择器可把最大 2 MiB 的 Markdown/Text 文件导入为一个章节；宿主不接受任意读取路径。导出由独立 `allow-project-export` permission 写入项目包 `exports/`，采用临时文件加原子 rename。检查点恢复支持在章节创建前后软归档与复活结构。

## 前端与运行

当前前端使用浏览器原生 DOM 与 `contenteditable="plaintext-only"`，没有第三方 npm 运行时依赖。`frontend-state.js` 是编辑器无关的状态/协议适配层，保存请求只包含 Block 基线和新内容，不允许 WebView 生成 Commit ID、时间戳或权威 hash。依赖源可用后，正文输入面可以替换为 Tiptap，而不改变 Host 命令和版本协议。

```powershell
npm.cmd run desktop:build
npm.cmd run desktop:dev
```

Tauri main window 由 Rust 从配置显式创建，并设置独立 WebView 数据目录。调试构建默认把浏览器缓存放在 `target` 内；正式构建使用系统应用数据目录。自动化或便携调试可通过绝对路径环境变量 `OPTIMIZER_WEBVIEW_DATA_DIR` 覆盖，系统会拒绝相对路径。
