# Optimizer System

基于《Optimizer Kernel 文本优化器工程设计文档》的实际工程仓库。

当前实现已完成 M0/M1 基线并推进 M2：稳定协议、纯领域内核、宿主安全边界、SQLite 版本存储、可校验快照恢复、编辑器桥接、Patch 审查闭环与多模型网关已经可测试运行。

## 已实现

- `@optimizer/protocol`：OperationIntent、ContextPacket、PatchProposal、EventEnvelope、ModelProviderConfiguration 类型与 JSON Schema。
- `@optimizer/kernel`：操作状态机、端口接口、确定性 Context Compiler、预算与隐私过滤。
- `kernel-lab`：无需第三方依赖即可运行的上下文编译示例。
- `optimizer-host`：Rust 宿主路径边界与目录逃逸防护基线。
- `optimizer-store`：SQLite 3.51.3、Block、编辑日志、Commit DAG、快照、FTS5 与在线备份。
- `@optimizer/editor-bridge`：稳定 Block ID、UTF-16 选区映射、乐观并发编辑事务与 Tiptap 快照适配。
- `@optimizer/patch-engine`：中文分层 diff、PatchProposal v2、逐 hunk 审查、原子决策与冲突检测。
- `@optimizer/model-gateway`：DeepSeek、Qwen、Kimi、MiniMax，支持流式输出、取消、超时与统一错误。
- 确定性 `optimizer-json+zstd` 快照编码、SHA-256 完整性校验与“恢复为新 Commit”。
- 架构依赖检查、Schema 解析检查、Node 测试与 Rust 测试。
- ADR 与工程设计 DOCX。

## 运行

要求 Node.js 24+ 与 Rust stable。当前阶段没有第三方 npm 依赖。

```powershell
npm.cmd run check
cargo test --workspace
npm.cmd run lab
```

## 目录

```text
apps/kernel-lab/       Context Compiler 可执行实验台
packages/protocol/     稳定协议与 JSON Schema
packages/kernel/       纯领域内核
packages/editor-bridge/ 编辑器无关的选区与事务桥接
packages/patch-engine/ AI 修改提案、分层 diff 与审查事务
packages/model-gateway/多厂商模型 API、SSE 与路由
packages/test-kit/     固定时钟、哈希器与测试夹具
crates/optimizer-host/ Rust 宿主安全边界
crates/optimizer-store/SQLite 权威存储与恢复
docs/architecture/     工程设计文档
docs/adr/              架构决策记录
scripts/               工程约束检查
```

## 下一步

1. Operation 执行器：ContextPacket → Provider → PatchProposal 全链路。
2. Ollama 本地 Provider 与模型可用性探测。
3. Tiptap 审查 UI 与 Kernel Lab 可视化。
4. PatchProposal、OperationRun 与审查事件持久化。
