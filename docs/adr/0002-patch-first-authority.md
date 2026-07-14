# ADR-0002：AI 输出始终是 PatchProposal

- 状态：Accepted
- 日期：2026-07-14

## 决策

模型输出在用户接受前不属于权威正文。每个 PatchProposal 必须引用不可变 `baseCommitId`、目标 block、`baseRevision` 与 `baseHash`。应用前重新校验基线；不一致时进入冲突状态，禁止模糊匹配后静默覆盖。

接受操作必须在一个事务中写入正文、Commit 和审计记录。

具体 hunk 基线、提案完整性哈希和审查事务规则由 ADR-0005 的 PatchProposal v2 补充。

## 结果

用户可以逐项接受、拒绝、保留候选或创建分支；模型失败、断网和取消不会产生正文副作用。
