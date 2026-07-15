# Optimizer System

基于《Optimizer Kernel 文本优化器工程设计文档》的实际工程仓库。

当前实现已完成 M0/M1 基线并推进 M2：稳定协议、纯领域内核、宿主安全边界、SQLite 版本与 Operation 审计存储、可校验快照恢复、编辑器桥接、Patch 审查闭环、多模型网关、Operation 执行闭环、跨语言宿主持久化链路、Tauri IPC 安全边界与可运行桌面工作区已经可测试运行。

## 已实现

- `@optimizer/protocol`：OperationIntent、ContextPacket、PatchProposal、EventEnvelope、ModelProviderConfiguration 类型与 JSON Schema。
- `@optimizer/kernel`：操作状态机、端口接口、确定性 Context Compiler、预算与隐私过滤。
- `kernel-lab`：无需第三方依赖即可运行的上下文编译示例。
- `optimizer-host`：Rust 宿主路径边界、原子 `.optimizer` 项目包、严格 JSON 命令适配、Operation/Review 持久化、审计读取、Secret Store，以及四家供应商的固定端点原生流式执行。
- `optimizer-desktop`：Tauri 2 桌面入口、单项目 Session、文档树、版本化自动保存、冲突草稿、检查点/恢复、模型设置、流式 AI 操作、取消与 Patch/Findings 审查界面。
- `optimizer-store`：SQLite 3.51.3、Block/Commit/快照、Operation/Artifact/Review 审计日志、FTS5 与迁移前在线备份。
- 项目风格库：SQLite schema v3 中独立保存、归档和恢复固定样本；canonical 样本以 L4 Context 参与操作，`never_send` 对远程模型强制排除。
- `@optimizer/editor-bridge`：稳定 Block ID、UTF-16 选区映射、乐观并发编辑事务与 Tiptap 快照适配。
- `@optimizer/patch-engine`：中文分层 diff、PatchProposal v2、逐 hunk 审查、原子决策与冲突检测。
- `@optimizer/model-gateway`：DeepSeek、Qwen、Kimi、MiniMax，支持流式输出、取消、超时与统一错误。
- `@optimizer/operation-runner`：ContextPacket → Provider → 严格输出校验 → PatchProposal/Findings，并生成版本化 Store bundle。
- 确定性 `optimizer-json+zstd` 快照编码、SHA-256 完整性校验与“恢复为新 Commit”。
- 版本化章节创建与结构恢复：Document/Block 创建在单事务中形成 Commit；检查点可跨章节创建前后归档或复活结构。
- 用户显式选文件的 Markdown 导入，以及限制在项目包 `exports/` 内的原子 Markdown 导出。
- 架构依赖检查、Schema 解析检查、Node 测试与 Rust 测试。
- ADR 与工程设计 DOCX。

## 运行

要求 Node.js 24+ 与 Rust stable。当前阶段没有第三方 npm 依赖。

```powershell
npm.cmd run check
cargo test --workspace
npm.cmd run lab
npm.cmd run desktop:dev
```

## 目录

```text
apps/kernel-lab/       Context Compiler 可执行实验台
apps/optimizer-desktop/Tauri 2 桌面 IPC 与权限适配层
packages/protocol/     稳定协议与 JSON Schema
packages/kernel/       纯领域内核
packages/editor-bridge/ 编辑器无关的选区与事务桥接
packages/patch-engine/ AI 修改提案、分层 diff 与审查事务
packages/model-gateway/多厂商模型 API、SSE 与路由
packages/operation-runner/模型调用与受验证产物的应用编排
packages/test-kit/     固定时钟、哈希器与测试夹具
crates/optimizer-host/ Rust 宿主安全边界
crates/optimizer-store/SQLite 权威存储与恢复
docs/architecture/     工程设计文档
docs/adr/              架构决策记录
scripts/               工程约束检查
```

## 下一步

1. 将 Context 编译与模型请求授权进一步收紧为宿主 Operation capability，强制 compromised WebView 也不能绕过 `never_send`。
2. Ollama 本地 Provider、模型可用性探测与本地/云端数据策略。
3. 依赖源可用后将正文输入适配器替换为 Tiptap，并提供行内 decoration 审查。
4. 显式成本策略下的重试、Provider fallback 与费用上限。
5. 最近项目、章节改名/移动/删除，以及结构化摘要失效队列。

## 最新迭代：可运行桌面工作区

- 欢迎页可创建或打开经 Host 校验的 `.optimizer` 项目包，项目凭据不会进入 WebView。
- 工作区显示文档树、结构化 Block、保存状态、有效字符数与版本抽屉。
- 900ms 串行自动保存绑定 Block revision/hash；冲突时保留本地草稿并重新加载权威版本。
- 检查点与恢复均通过 Host，恢复创建新 Commit，不覆盖历史。
- 静态前端没有第三方 npm 运行时依赖和远程资源；Tauri CSP 只开放本地资源与 IPC。
- Tiptap 被隔离在编辑器适配边界之后，依赖可用时替换，不阻塞当前可运行闭环。

## 最新迭代：宿主模型执行与 AI 审查闭环

- DeepSeek、Qwen、Kimi、MiniMax 通过 Rust WinHTTP 固定官方端点执行；Qwen 支持区域/workspace，四家 reasoning、token 与流式差异统一为同一事件协议。
- API Key 只在 Rust 中从系统 Secret Store 解析，WebView 只能管理 opaque `credentialRef` 的写入、存在性和删除。
- AI 工具栏支持续写、润色、压缩、扩写、批评；当前选区为空时使用当前 Block，续写使用光标位置。
- 用户可把正文选区固定为项目风格样本，在专用抽屉中归档或恢复；风格与正文/事实分层，不会把归档样本重新召回。
- Context Compiler、Operation Runner 与 Patch Engine 复用仓库同一份实现；模型输出经过严格 JSON 校验后生成不可变 PatchProposal 或 Findings。
- 任何云端模型请求发出前都会展示实际编译后的 Context Packet、来源、层级、必需标记、排除项和估算 token；用户确认前不会调用模型宿主命令。
- 每个 hunk 可接受或拒绝；最终应用在一个 SQLite transaction 内同时创建 `ai_accept` Commit、编辑日志、Review apply 事件和 Operation accepted 状态。
- 本地浏览器验收覆盖模型设置、发送前 Context 预览、流式续写、Patch 审查和应用完成路径；没有控制台或页面错误。
