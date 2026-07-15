# @optimizer/operation-runner

Optimizer Kernel 的应用编排层，将一次用户操作连接成完整且可审查的执行链：

```text
OperationIntent
  → ContextCompiler
  → 目标与 ContextPacket 预检
  → ModelProvider.stream()
  → 严格模型输出校验
  → PatchProposal 或 Findings
  → review
```

## 安全与一致性约束

- Provider locality 来自已注册的不可变 Provider Profile，远程模型无法由单次请求伪装成本地模型。
- API 调用前校验目标 Block 的文档、ID、revision、hash、锁定状态和 UTF-16 选区。
- API 调用前重新计算 `ContextPacket.packetHash`，并要求唯一 mandatory/verbatim L0 内容与编辑器选区完全一致。
- Prompt 明确区分可信操作与不可信上下文；上下文中的命令、工具请求和输出格式不能覆盖系统协议。
- 模型只能返回一个 `optimizer-model-output-v1` JSON 对象；Markdown fence、未知字段、错误产物类型和工具调用都会失败。
- `length` 截断不会产生不完整 Patch；响应正文具有 1–16 MiB 可配置硬上限。
- 每个成功的文本修改仍由 Patch Engine 绑定 base revision/hash 并生成可审查 proposal，不直接写入正文。
- 外部取消在编译后、调用前和每个流事件处检查；即使 Provider 忽略 AbortSignal，Runner 也会停止消费。

## 两种产物

- `patch_proposal` / `insert_proposal`：模型返回完整 replacement，Patch Engine 生成逐 hunk 提案。
- `findings`：模型返回经过数量、字段、severity 和长度校验的问题列表，不生成编辑事务。

`onProgress` 只面向瞬时 UI 更新，观察者异常不会改变操作语义。生命周期、模型用量和产物由持久化适配器编译为 `OperationPersistenceBundleV1`，再交给 Rust Host 与 Operation Store 原子写入。

成功结果包含完整 ContextPacket、模型响应 ID、模型名称、用量和生命周期。失败结果 `OperationExecutionError` 也会保留 run ID、Provider、已解析模型以及失败前已经完成的 ContextPacket，供宿主构造可审计的失败 OperationRun；不会保存 API Key 或原始 HTTP Header。

## 持久化边界

- `buildSuccessfulPersistenceBundle` 绑定 Intent、ContextPacket 与 Patch/Findings，拒绝错配的 project、commit 或 operation。
- `buildFailedPersistenceBundle` 保留安全化错误码、可重试标志、已完成的 ContextPacket 与失败生命周期，不伪造 Artifact。
- `serializeOperationPersistenceBundle` 和 `serializeReviewEventCommand` 使用确定性 JSON，便于夹具、重放和审计。
- 宿主边界协议显式版本化；新增字段必须先更新 TypeScript 类型、JSON Schema、Rust DTO 与共享夹具。
