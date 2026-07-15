# ADR-0009：版本化的跨语言 Operation Host 命令

- 状态：Accepted
- 日期：2026-07-15

## 背景

Operation Runner 位于 TypeScript 应用层，权威 Store 位于 Rust/SQLite。若 UI 直接拼装多条数据库命令，成功结果、失败结果、模型用量、ContextPacket、生命周期与 Artifact 可能产生部分写入；若 Rust 接收无约束 JSON，则协议字段漂移和意外的敏感字段都难以及时发现。

## 决策

在 Protocol 中定义两个显式版本化边界对象：

- `OperationPersistenceBundleV1`：一次 Operation 的最终运行记录、可选 ContextPacket、完整生命周期和可选 Artifact；
- `PersistReviewEventCommandV1`：带 expected revision/status 的单个审查事件命令。

Operation Runner 负责把成功或失败执行结果编译为 bundle，并用稳定 JSON 序列化。Rust Host 使用 `serde` 严格 DTO 解码，所有层级拒绝未知字段并检查 `schemaVersion`，之后才映射到 Store 类型。Host 不重写 Store invariant；事务性、外键、Artifact 不可变性和审查乐观并发仍由 `optimizer-store` 统一执行。

```text
OperationRunner result/error
        ↓ deterministic serializer
OperationPersistenceBundleV1 JSON
        ↓ strict Rust DTO + schema version
OperationCommandHost
        ↓ one Store transaction
SQLite audit graph
```

Host 同时提供聚合审计读取，返回 OperationRun、模型用量、失败信息、ContextPacket payload、生命周期、Artifact payload 与 Patch review 事件。数据库内部保存的 JSON 字符串不会直接泄漏到上层；Host 在返回前解析为 JSON value。

## 失败与安全语义

- review 状态必须具有 ContextPacket、response ID、finish reason 与 Artifact；
- failed/cancelled 必须具有安全化 failure，且不能携带 Artifact；
- API Key、credential value 与原始 HTTP Header 不属于任何持久化边界类型；Host 还会递归检查开放 JSON payload 中的敏感字段名；
- 未知嵌套字段、未知 enum 或不支持的 schema version 在第一次数据库写入前失败；
- Store 在后段校验失败时回滚整个 bundle，不产生孤立 ContextPacket、run 或 Artifact；
- 审查命令以 expected revision/status 提供乐观并发，事件、head、Operation 状态与附加生命周期在同一事务中更新。

## 兼容性验证

`packages/protocol/fixtures/operation-persistence-bundle.v1.json` 是 TypeScript 与 Rust 共用的契约夹具。Node 测试确认协议类型与序列化结果；Rust Host 测试读取同一文件并执行真实 SQLite 写入与审计读取。另有测试覆盖未知敏感字段、版本不匹配、无部分写入以及 review→ready→applied 状态同步。

## 取舍与后续

当前边界是无 Tauri 依赖的 Rust API，便于单元测试和未来复用。下一阶段只在外层注册 Tauri commands 和 capabilities，不在 command handler 中复制 DTO 或数据库规则。协议发生不兼容变化时新增 schema version，并保留旧版本迁移/拒绝策略，不静默接受语义不同的字段。
