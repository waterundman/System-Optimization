# @optimizer/protocol

> 对应版本 v0.2.0

## 定位

共享类型与 schema 层：在 TypeScript 侧定义 Optimizer 系统跨包共享的 branded ID、领域枚举、不可变快照结构与 JSON Schema 契约，是 kernel / patch-engine / apps 的公共依赖。

## 包结构

- `src/index.ts`：单一入口，re-export 全部子模块。
- `src/types.ts`：branded ID 类型、领域枚举、操作 / 上下文 / 补丁 / 事件 / Provider 配置等核心类型。
- `src/storage.ts`：文档与块快照、提交描述符、物化快照描述符。
- `src/stable-json.ts`：稳定 JSON 序列化（v0.2.0 新增）。
- `src/validation.ts`：与 `schemas/` 下 JSON Schema 对齐的运行时校验。
- `schemas/`：JSON Schema 文件，作为跨语言（TypeScript ↔ Rust）契约的单一真理源。
- `fixtures/`：用于跨包联调的 v1 持久化 bundle 样例。
- `test/`：`protocol.test.ts` 与 `stable-json.test.ts`。

## 主要导出

### 来自 `src/types.ts`

- Branded ID：`WorkspaceId` / `ProjectId` / `DocumentId` / `BlockId` / `CommitId` / `OperationIntentId` / `OperationRunId` / `ContextPacketId` / `PatchProposalId` / `EventId` / `CredentialRef`。
- 操作语义：`OperationType`（`polish` / `continue_scene` / `compress` / `expand` / `critique`）、`OperationStrength`、`OutputKind`、`ProviderLocality`。
- `OperationIntent`：携带 `schemaVersion: 1` 的操作意图。
- 上下文编译：`ContextTier`（L0–L4）、`CanonicalStatus`、`Authority`、`Sensitivity`、`RenderMode`、`ContextItem` / `ContextExclusion` / `ContextBudget` / `ContextPacket`。
- 补丁：`PatchProposalStatus`、`DiffGranularity`、`PatchHunk`、`PatchProposal`（`schemaVersion: 2`）。
- 事件：`EventEnvelope`。
- Provider 配置：`ModelProviderId`（`deepseek` / `qwen` / `kimi` / `minimax` / `ollama` / `openai_compatible`）、`QwenDeploymentRegion`、`ModelProviderConfiguration`。
- 持久化：`PersistedModelUsage` / `PersistedOperationFailure` / `PersistedOperationAttempt` / `OperationPersistenceBundleV1`、`PersistedReviewStatus`、`PersistReviewEventCommandV1`。
- `OperationState` 枚举（与 kernel 状态机共享）。

### 来自 `src/storage.ts`

- `DocumentKind`（`folder` / `document` / `chapter` / `scene` / `note`）。
- `BlockKind`（`paragraph` / `heading` / `quote` / `dialogue` / `list` / `table` / `scene_break` / `locked`）。
- `DocumentSnapshot` / `BlockSnapshot`：物化快照读取用的不可变视图。
- `CommitDescriptor`：提交元数据，`reason` 覆盖 `initial` / `autosave` / `pre_ai` / `ai_accept` / `manual` / `restore` / `import` / `migration`。
- `MaterializedSnapshotDescriptor`：物化快照定位符（含 `codec` / `codecVersion` / `checksum`）。

### 来自 `src/stable-json.ts`（v0.2.0 新增）

- `stableStringify(value: unknown): string`：递归排序对象键、剔除 `undefined` 字段、拒绝非有限数值与不可序列化类型，输出确定性 JSON。用于跨进程哈希一致性与 `packetHash` / `proposalHash` 等绑定计算。

## Schema 版本契约

- `OperationIntent`、`ContextPacket`、`PatchProposal`、`EventEnvelope`、`ModelProviderConfiguration`、`OperationPersistenceBundleV1`、`PersistReviewEventCommandV1` 均携带 `schemaVersion` 字段；当前均为 `1`（`PatchProposal` 为 `2`）。
- 物化快照 schema 版本由 Rust 侧常量 `SNAPSHOT_SCHEMA_VERSION` 守护，定义于 `crates/optimizer-store/src/snapshot.rs:9`，当前值为 `1`。任何破坏性变更必须 bump 版本号，并在 Rust / TypeScript 两侧同步更新 `schemas/` 下的 JSON Schema 文件与运行时校验逻辑。

## 依赖关系

- 本包无运行时第三方依赖，仅依赖 TypeScript 标准库。
- 被以下包消费：`packages/kernel`、`packages/patch-engine`、`packages/operation-runner`、`packages/editor-bridge`、`packages/model-gateway`、`apps/optimizer-desktop`（通过 fixture 与 schema 校验），以及 Rust 侧 `crates/optimizer-store` / `crates/optimizer-host`（通过 JSON Schema 对齐）。

## 开发命令

```pwsh
# 在 packages/protocol 目录下
node --test            # 运行 test/ 目录下的所有测试
node --test test/protocol.test.ts
node --test test/stable-json.test.ts
```
