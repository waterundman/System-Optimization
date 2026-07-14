# ADR-0005：PatchProposal v2 与严格审查事务

- 状态：Accepted
- 日期：2026-07-14

## 决策

`PatchProposal` 升级为 schema v2。每个 hunk 除 from/to/replacement 外，必须携带对应的 `original` 原文和 `granularity`；整个提案必须携带 `proposalHash`。哈希对象是除 `proposalHash` 字段外的规范 JSON。

候选文本通过段落、句子、token 三层确定性 diff 编译为按 UTF-16 坐标排序且互不重叠的 hunk。中文汉字按字符处理，拉丁字母和数字按连续词组处理，代理对、组合符号与 ZWJ emoji 不在无效边界切开。

审查状态独立于不可变提案保存。每次决定携带 expected review revision；原子组中的 hunk 共享决定。只有全部 hunk 已决定的 review 才能编译为 Editor Bridge 事务。

## 应用前校验

- 重新计算并验证 proposal hash；
- 校验 stable protocol 与 hunk 坐标；
- 校验目标 Block revision、content hash 和锁定状态；
- 逐 hunk 比较当前位置原文与 `original`；
- 按坐标降序应用已接受 hunk；
- 结构化编辑器内容由显式 `BlockContentAdapter` 生成；
- 最终仍由 Editor Bridge 再次执行乐观并发校验。

任一基线不一致都返回冲突，不进行模糊重定位或静默覆盖。

## 协议版本

原 v1 没有 hunk 原文和提案哈希，无法满足应用前完整性校验。新增字段是破坏性变化，因此明确升级为 v2；早期仓库尚未发布持久化 v1 提案，不提供不安全的自动迁移。

## 复杂度边界

LCS 矩阵具有显式 cell 上限，单次输入和输出具有 UTF-16 长度上限，hunk 数量也有限制。超过 LCS 预算时生成带 `DIFF_COMPLEXITY_FALLBACK` 警告的粗粒度替换。
