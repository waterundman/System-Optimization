# ADR-0008：Operation 产物与审查历史事务性持久化

- 状态：Accepted
- 日期：2026-07-15

## 决策

SQLite schema 升级到 v2，新增以下数据组：

```text
context_packet_record
        ↓
operation_run → operation_lifecycle_event
        ↓
operation_artifact (PatchProposal / Findings)
        ↓ PatchProposal only
patch_review_head → patch_review_event
```

`persist_operation_bundle` 在一个 `BEGIN IMMEDIATE` 事务中写入 ContextPacket、OperationRun、模型用量、生命周期、Artifact 和初始 Patch review head。任一 JSON、外键、唯一键或 invariant 失败都会回滚整个 bundle。

## 不可变数据与可变 Head

ContextPacket、Artifact、生命周期事件和审查事件具有数据库触发器保护，禁止 UPDATE/DELETE。OperationRun 只允许按照 Kernel 生命周期从 review/conflicted 向后转换；其他字段不可修改。

Patch review head 只保存 revision、status 和更新时间。每个命令必须提供 expected revision/status；事件插入、head 更新、Operation 状态更新和新增生命周期事件在同一事务中完成。过期命令返回 `StateConflict`，不会产生部分事件。

## 失败记录

失败和取消的 OperationRun 必须带安全化 failure code/message，可以没有 ContextPacket，但不能伪造模型 Artifact。成功进入 review 的运行必须具有 ContextPacket、响应 ID、finish reason 和恰好一个 Artifact。

## 迁移与回滚

v1→v2 是只新增表、索引和触发器的事务性迁移。文件数据库升级前使用 SQLite Online Backup 创建独立备份并执行 `quick_check`；备份不会被自动覆盖或删除，路径通过 StoreDiagnostics 暴露。旧程序无法打开更高 schema，因此降级必须恢复该备份，不能手工修改 `user_version`。

## 取舍

Artifact JSON 同时保留领域协议的完整表达，常用关联、状态和 token 用量则规范化为列。这样会增加少量重复数据，但避免审计时依赖当时版本的 TypeScript 反序列化器。

Findings 当前作为不可变 Artifact 存储，不创建 Patch review head；其逐条确认模型将在批评 UI 进入实现阶段后单独定义。
