# ADR 0020：按作用域绑定 Commit 的摘要失效队列

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：分层摘要、失效合并、陈旧写回保护与桌面状态读取

## 背景

Context Compiler 已能使用章节摘要与项目摘要，但正文和结构发生变化后，存储层此前没有权威方式判断哪些摘要需要重算。仅在摘要记录上保存 `is_stale` 会丢失生成竞态：worker 读取旧正文开始生成，期间正文再次提交，旧结果仍可能清掉新失效状态。每次提交全量重建摘要又会放大模型费用，并让无关章节编辑使全部局部摘要失效。

摘要是可重建的派生数据，不属于 Commit DAG 的不可变事实；但“哪些作用域需要重建”必须与权威正文变更原子一致，并且不能由 WebView 自行维护。

## 决策

1. SQLite schema v5 增加 `summary_record` 与 `summary_invalidation`。作用域固定为 `project`、`document`、`block`，复合主键为项目、作用域类型和作用域 ID。
2. 正文编辑、AI Patch 接受、章节创建/改名/排序/归档/恢复和检查点恢复，在写入 Commit 的同一个 `IMMEDIATE` 事务中更新失效队列。任何 Commit 失败都会同时回滚摘要失效变化。
3. 同一作用域只保留一个队列项。再次失效以 upsert 替换 `source_commit_id`、原因和时间，并递增 `invalidation_count`，从而限制队列规模且保留抖动信号。
4. 失效按最小必要范围传播：Block 内容变化使该 Block、所属 Document 和 Project 失效；章节元数据变化使该 Document 和 Project 失效；创建增加新 Document/Block；归档移除不可见作用域；恢复重新登记 Document 及其所有活动 Block。
5. 摘要写回命令必须提交它开始生成时读取的 `expected_source_commit_id`。Store 在同一事务中比较该作用域当前队列项，写入或递增 `summary_record.revision`，并只删除完全匹配的队列项。生成期间若同一作用域再次失效，命令返回冲突且不改变摘要或队列。
6. `source_commit_id` 是作用域版本令牌，而不是全局 HEAD 锁。其他章节的提交不会让一个未变化 Block 的摘要失败；Document 和 Project 的传播规则仍确保其聚合摘要在相关下游变化时重新排队。
7. `summary_record` 是派生缓存，允许删除和受控 revision upsert；其作用域键不可在更新中改写。Commit、Operation 与 Review 的不可变审计规则不因此放宽。
8. 归档章节从活动 FTS 查询和摘要队列移除，正文与已有摘要记录仍保留。恢复章节会重新登记其 Block，优先保证不会漏算，接受可能的重复生成成本。
9. Tauri 新增只读 `list_summary_invalidations`，放入独立 `allow-summary-status-read` permission。返回值不包含正文和摘要文本，WebView 只显示待更新数量；消费和摘要写回不开放给页面。

## 失败与恢复语义

- 队列更新和内容 Commit 原子提交，不存在“正文已变但未失效”的成功状态。
- worker 超时或崩溃不会丢任务，因为读取不领取或删除队列项。
- 陈旧完成返回乐观冲突；worker 应丢弃结果并重新读取最新作用域。
- `verify_invariants` 检查队列 Commit 属于同一项目、Project scope ID 正确，并确认 Document/Block 仍处于活动结构中。

## 测试要求

- 种子项目产生 Project、Document、Block 三项初始失效。
- 连续编辑合并队列、更新作用域 source Commit 并递增计数。
- 陈旧摘要完成不能修改记录或消费新队列；匹配完成原子 upsert 并清除单项。
- 章节归档移除不可见 Document/Block 队列，恢复重新加入；归档正文不能被 FTS 命中。
- 迁移、Host DTO、Tauri permission/command manifest 和桌面 mock 必须覆盖 schema v5。

## 后果

- 后台 worker 可以按 Block → Document → Project 的依赖顺序消费队列，无需扫描全文寻找陈旧记录。
- 当前队列仍是单进程、非领取式模型；若未来允许多个 worker，应增加带租约的 claim 表，而不能在读取时直接删除失效项。
- 当前 Store 只保证作用域版本绑定；摘要内容质量、token 预算、重试和 provider 选择属于后续 worker 调度层。
