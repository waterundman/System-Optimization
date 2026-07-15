# ADR-0007：以受验证产物结束模型调用

- 状态：Accepted
- 日期：2026-07-15

## 决策

新增独立 `@optimizer/operation-runner` 应用编排层。它依赖 Kernel、Model Gateway、Patch Engine 和 Editor Bridge 的稳定端口，但这些底层包不反向依赖 Runner。

一次成功调用严格执行以下状态：

```text
draft → compiling → preflight → queued → streaming → validating → review
```

失败和取消进入现有 OperationLifecycle 的终态，并通过 `OperationExecutionError` 携带 run ID、终态、历史和安全化错误信息。

## 模型输出边界

模型文本不是可以直接应用的 PatchProposal。模型只返回内部 `optimizer-model-output-v1` JSON 信封：

- 修改类操作返回 `replacement`；
- 批评类操作返回 `findings`。

Runner 拒绝 Markdown 包裹、未知字段、产物类型不匹配、工具调用、重复/缺失流事件、响应过大和 token 截断。通过校验的 replacement 仍需交给 Patch Engine，根据可信编辑器快照重新计算 diff、hunk 和 proposal hash。

## 隐私与 TOCTOU

Context Compiler 根据 Provider Profile 的 locality 过滤 `never_send` 内容。发送前 Runner 重新校验 ContextPacket hash、Operation 绑定关系、唯一 L0 target 内容与当前 Block revision/hash，从而缩短“编译上下文”和“发起计费请求”之间的状态竞态窗口。

## 取舍

严格 JSON 会使部分模型的自然语言回答直接失败，但它避免从 Markdown 或混杂文本中猜测可执行内容。当前不自动修复或二次请求，因为这可能产生额外费用；上层未来可以在明确展示成本后发起新的 OperationRun。

`onProgress` 是非持久、异常隔离的 UI 通道，不承担审计职责。OperationRun、状态事件、用量和产物的事务性持久化留给下一阶段 Store 编排。
