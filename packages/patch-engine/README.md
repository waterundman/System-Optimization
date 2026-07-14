# @optimizer/patch-engine

将模型候选文本转换为可审查、可校验、可事务应用的 `PatchProposal v2`。

当前能力：

- 段落 → 句子 → 中文字符/单词 token 的确定性分层 diff；
- UTF-16 绝对坐标、原文基线、差异粒度与可选原子组；
- 规范 JSON 的 proposal SHA-256 完整性绑定；
- 逐 hunk 接受/拒绝和原子组决策传播；
- review revision 乐观并发；
- Block revision/hash、锁定状态与 hunk 原文的二次冲突检测；
- 将已接受 hunk 编译为 Editor Bridge 事务，但不直接修改正文。

复杂度超过预算时返回显式 `DIFF_COMPLEXITY_FALLBACK` 警告并生成安全的较粗粒度替换，不进行不受控的高内存计算。
