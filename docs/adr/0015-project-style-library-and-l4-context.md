# ADR 0015：项目风格库与 L4 Context

- 状态：Accepted
- 日期：2026-07-15
- 决策范围：M2 风格记忆与 Context Compiler

## 背景

MVP 要求作者能够固定少量自己认可的文本作为风格参考，同时必须避免把废弃样本重新召回、把仅本地样本发往云端，或在用户不知情时增加请求内容。风格样本不同于当前正文：它不应出现在文档树中，也不能因为语义相似就被当成事实或剧情。

## 决策

1. SQLite schema v3 增加项目级 `style_sample` 表，独立保存标题、正文、宿主计算的 SHA-256、`canonical/archived` 状态、`local_sensitive/never_send` 策略与乐观并发 revision。
2. 样本创建只接受当前项目 Session 中的显式用户选区。宿主限制标题为 1–120 个字符、正文为 1–65536 bytes，并对正文重新计算哈希。
3. 桌面端通过专用 `allow-style-library` capability 调用列出、创建和状态更新命令；WebView 没有任意 SQL 或项目文件读写能力。
4. `canonical` 样本作为 `L4_STYLE_GLOBAL`、`user_confirmed`、`PINNED_STYLE_SAMPLE` 候选交给 Context Compiler；`archived` 仍可管理，但编译时以 `INELIGIBLE_STATUS` 排除。
5. `never_send` 样本允许未来本地模型使用；远程 Provider 编译时以 `POLICY_DENIED` 排除。排除记录只包含引用和哈希，不包含正文。
6. 任何真正入选的风格样本都会出现在发送前 Context Packet 预览中。用户取消确认时不调用模型宿主。
7. 样本归档和恢复使用 `expectedRevision`，冲突时重新读取权威列表，不能用陈旧页面状态覆盖其他修改。

## 结果

- 风格、正文、结构与事实保持类型隔离；AI 可以模仿句法和节奏，而不会把样本人物或情节误当成当前作品事实。
- 废弃与仅本地内容由 Kernel 策略过滤，而不是依赖提示词要求模型忽略。
- 浏览器回归覆盖风格抽屉与 L4 发送预览；Node 测试覆盖 canonical、archived 和 `never_send` 三种编译结果；Rust 测试覆盖迁移、项目隔离和乐观并发。

## 代价与后续

- 风格样本当前是项目资产 revision，不属于正文 Commit/Checkpoint 图；恢复旧正文检查点不会回滚风格库。后续统一资产版本协议时，应把风格、事实和素材状态纳入独立资产提交图，而不是挤入 Block 快照。
- MVP 默认新样本为 `local_sensitive`。后续设置界面应允许创建前选择“可发送到云端”或“仅本地”，并支持编辑标题、内容与用途标签。
- 当前按用户固定顺序和统一相关度参与 L4 预算。样本增多后再加入操作类型标签、作者评分与去重，不在此阶段引入向量检索。

## 回滚

可以撤销 `allow-style-library` capability 和桌面入口，使现有 v3 数据保持不可见但不丢失；Context Compiler 会在没有 style source 时继续按 L0–L3 正常工作。
