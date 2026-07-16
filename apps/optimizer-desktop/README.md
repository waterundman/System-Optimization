# optimizer-desktop

Optimizer System 的 Tauri 2 桌面应用。当前已经具备可运行的中文入口、项目创建/打开、文档树、Block 编辑与自动保存、冲突草稿保护、检查点、版本历史和恢复交互；所有持久化仍通过 Rust IPC 安全边界完成。

WebView 只允许：

- 创建、打开、关闭和查看一个已校验的本地项目会话；
- 列出、按稳定项目 ID 重新打开或移除宿主用户配置区中的最近项目记录；
- 原子写入 Operation bundle；
- 写入 Patch review 事件；
- 读取 Operation 审计；
- 写入、检查和删除 provider secret。
- 通过一次性、短期、绑定当前项目 HEAD/完整 Context Packet/受控 Prompt/目标 Block 的 Rust capability，执行 DeepSeek、Qwen、Kimi、MiniMax 固定 HTTPS 端点，以及 Ollama 固定回环端点的流式请求与取消。
- 复用 Kernel/Operation Runner/Patch Engine 完成 AI 操作、Findings、逐 hunk 与全部接受/拒绝审查；批量 decision 仍逐 revision 写入 Host 审计。
- 列出并按需重新加载持久审查候选；候选详情由 Host 校验不可变 Proposal、hash 和全部 review event 后返回，页面不能提交自造 decision。
- 将 ready 候选保存为独立分支 Commit 与物化快照，不移动当前主分支 HEAD，也不改变当前正文或审查状态。
- 原子应用已审查 Proposal，同时写入 `ai_accept` Commit 与完整 Operation/Review 审计。
- 读取文档树并用版本基线保存 Block；
- 建立检查点、浏览版本元数据并恢复为新 Commit。

WebView 不存在“读取 secret 明文”命令。云端模型请求由 Rust 信任边界解析 `credentialRef` 并执行，API Key 不会返回 JavaScript。

WebView 也不存在原始 `execute_model_stream` 命令。用户确认 Context 后，`authorize_model_request` 会重新读取当前 HEAD 的全部候选，复算 Packet stable hash、token、评分、预算和 `never_send` 等策略，并验证实际 system/user Prompt 逐项等于确认 Packet；通过后才返回最多 120 秒有效且只可消费一次的 capability。该 capability 在 Host 内持有完整不可变 Packet，但调试输出只暴露 ID/hash/字节数；`execute_authorized_model_stream` 只接收 capability ID。项目变化、取消或关闭会使授权失效。

五种内置 AI 操作由同一注册表驱动工具栏、Block 右键、`Alt+1…5` 快捷键和 `Ctrl/⌘+Shift+P` 命令面板。普通右键打开 AI 菜单，`Shift + 右键` 保留系统菜单；所有入口共享当前 UTF-16 选区与并发禁用规则，快捷键不能绕过正在执行或审查中的状态。

候选中心显示进行中与历史审查。未完成候选从 Host 的 artifact/event 真相重新构造，可继续逐项或批量决策；ready 候选可保存一次独立分支。分支入口要求当前 HEAD 与 operation base、目标 Block revision/hash 完全一致，发生冲突时保持只读审计，不做隐式 rebase。候选分支快照不出现在用户检查点列表。

Ollama 是无凭据的显式本地 Provider：宿主只连接 `127.0.0.1:11434/v1/chat/completions` 并禁用系统代理，不接受页面提交的地址。模型需由用户预先在 Ollama 中拉取；运行失败不会自动把本地 Context Packet 发往云端。

用户可在 Provider 抽屉显式执行一次本地模型检测。`list_ollama_models` 仍在 Rust 内固定请求 `/v1/models`，限制响应体并校验 model ID；WebView 只获得安全的建议列表，不能提交探测 URL。

## 项目会话

- `create_project` 在安全父目录内原子创建 `.optimizer` 项目包并打开；
- `open_project` 校验 manifest、SQLite invariant、项目 ID 与主分支绑定后打开；
- `close_project` 释放当前数据库连接；
- `get_project_session` 返回当前项目、主分支、HEAD、revision 与数据库 schema；
- Operation 写入和审计命令在无项目时统一返回 `NO_PROJECT_OPEN`。

四个命令位于独立的 `allow-project-session` permission 中，只授权给本地 `main` window。入口表单只提交用户明确填写的绝对路径，创建与打开规则由 Host 再次校验。

最近项目使用独立 `allow-recent-projects` permission。创建或成功打开项目后，Host 把安全元数据写入应用本地数据目录的严格 JSON 注册表，最多保留 12 项。欢迎页快速打开只提交 `projectId`，Host 解析已登记路径后仍执行完整项目包校验，并确认当前位置的项目 ID 未被替换。移除操作只删除最近记录，不触碰 `.optimizer` 目录。

## 工作区与版本

- `get_project_workspace` 返回文档树、结构化 Block、稳定 hash/revision 和当前 HEAD；
- `list_summary_invalidations` 只返回项目/章节/Block 摘要失效元数据；页面显示待更新数量，不接触摘要生成凭据、正文或 SQLite；
- `refresh_summaries` 只触发 Host 内置的本地提取式 worker，最多处理 64 项且不发起网络请求；页面默认按 12 项后台批处理，失败指数退避；
- `get_summary_context` 必须绑定当前 HEAD、目标 Block revision 和 content hash，只返回没有失效项的当前章节/祖先/项目摘要候选；
- `get_operation_context` 在同一 Host 基线上统一收集目标/局部正文、章节结构、摘要、事实/约束和风格，严格校验 UTF-16 选区与每个 source hash；桌面 AI 主链不再由 WebView 拼装来源；
- `save_block` 使用 Block 与项目 HEAD 双层乐观并发，成功时原子生成 EditJournal 与 autosave Commit；
- `create_checkpoint` 为当前 HEAD 建立带 checksum 的物化快照；
- `get_version_history` 只返回 Commit/检查点元数据；
- `restore_checkpoint` 校验快照并“恢复为新 Commit”，不会覆盖历史节点。

工作区读写和版本读写分别拥有独立 permission。前端以 900ms 停顿触发串行保存，对 `CONFLICT` 保留本地草稿并重新加载权威工作区，对 `NO_CHANGES` 静默结束保存状态。未处理冲突会阻止建立检查点与恢复；关闭项目需要用户再次确认。

风格库使用独立 `allow-style-library` permission。选中的正文可固定为项目样本；启用样本作为 L4 Context 候选，归档样本和远程调用中的 `never_send` 样本由 Context Compiler 在模型调用前排除。实际入选内容始终出现在发送确认弹窗中。

事实与约束使用独立 `allow-knowledge-library` permission。用户可创建事实或硬/软约束，并把条目标记为 canonical、archived 或 rejected；页面不能提交 authority，人工录入统一由 Host 标记为 `user_confirmed`。`get_knowledge_context` 同时校验当前 HEAD 和目标 Block 的 ID/revision/hash，只返回 canonical L3 候选。WebView 会复算候选内容 SHA-256；云模型排除 `never_send`，本地 Ollama 仍需在发送预览中确认。

文档侧栏按 `parentId + orderKey` 渲染真正的父子树，可创建顶层或子章节、在同一父节点内移动、缩进到上一兄弟节点、移出到上一层，并重命名。所有命令绑定 Document revision 与项目 HEAD；归档父节点会原子归档整个活动子树，恢复子节点前必须先恢复父节点。原生文件选择器可把最大 2 MiB 的 Markdown/Text 文件导入为顶层章节；宿主不接受任意读取路径。导出由独立 `allow-project-export` permission 写入项目包 `exports/`，采用临时文件加原子 rename。检查点恢复可恢复父子关系、同级顺序与归档状态。

摘要状态读取使用独立的 `allow-summary-status-read` permission。正文自动保存、AI 接受、章节生命周期与检查点恢复会在对应 Store 事务中合并更新作用域失效项；页面只在权威写入完成后刷新计数。`allow-summary-worker` 仅允许页面触发 Host 内置的零外发 worker，以及按当前目标领取已就绪摘要；正文读取、作用域构造、source hash、写回和队列消费仍保留在 Rust 信任边界。返回的摘要内容在加入 Context Compiler 前由页面复算 SHA-256，并完整展示在发送确认弹窗中。

统一 Context 读取使用独立的 `allow-operation-context` permission。所有候选均带当前 source Commit、稳定 source ref、policy 和评分信号；页面校验集合中恰有一个匹配当前选区的 L0、没有重复 ID/source ref，且非 L0 不得被标记为 mandatory。Kernel 负责跨宿主的预算、去重、远程 `never_send` 排除和最终 Packet hash；模型授权入口再由 Host 独立复算同一套确定性结果，并要求 Packet 对每个 Host 候选恰好给出入选或受控排除结果。

## 前端与运行

当前前端使用浏览器原生 DOM 与 `contenteditable="plaintext-only"`，没有第三方 npm 运行时依赖。`frontend-state.js` 是编辑器无关的状态/协议适配层，保存请求只包含 Block 基线和新内容，不允许 WebView 生成 Commit ID、时间戳或权威 hash。依赖源可用后，正文输入面可以替换为 Tiptap，而不改变 Host 命令和版本协议。

```powershell
npm.cmd run desktop:build
npm.cmd run desktop:dev
```

Tauri main window 由 Rust 从配置显式创建，并设置独立 WebView 数据目录。调试构建默认把浏览器缓存放在 `target` 内；正式构建使用系统应用数据目录。自动化或便携调试可通过绝对路径环境变量 `OPTIMIZER_WEBVIEW_DATA_DIR` 覆盖，系统会拒绝相对路径。
