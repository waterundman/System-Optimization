# ADR 0023：宿主本地摘要 worker 与目标绑定 Context 来源

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：摘要队列消费、默认生成器、ready summary 读取、桌面空闲调度与 Context 接入
- 扩展：ADR 0020 的失效与乐观写回协议

## 背景

schema v5 已能在正文/结构 Commit 的同一事务中登记项目、Document 和 Block 摘要失效，并拒绝基于旧 source Commit 的生成结果。但队列此前没有消费者，摘要也没有进入实际 ContextPacket：页面只能显示待更新数量。若直接让 WebView 读取正文、拼装生成请求并写回摘要，会重新引入路径、凭据、作用域与陈旧结果越过 Host 的问题；若在未获同意时静默调用云模型，则会产生费用和隐私外发风险。

## 决策

1. 在 `optimizer-host` 增加 `summary_worker`。`OpenedProject::refresh_summaries` 每批只接受 1..=64 的数量意图，正文读取、树作用域、source hash、时间戳和 `PutSummaryRecord` 全部由 Host 生成。
2. 默认生成器固定为 `optimizer-local/extractive-summary-v1`。它是确定性、零网络、零凭据、零额外费用的提取式基线：Block 摘要来自当前 Block；Document 摘要来自其活动子树；Project 摘要来自活动文档森林。输入预览和输出均有字符上限。
3. 摘要来源哈希与展示文本哈希分离：Block 使用权威 content hash，Document 使用子树结构/Block hash 的规范散列，Project 使用 root hash v2；`summary_hash` 始终是最终摘要文本的 SHA-256。记录继续保存 source Commit、provider、model 与 revision。
4. worker 显式按 Block → Document → Project 排序。同一批次内可先完成细粒度记录，再完成聚合记录；写回仍由 Store 比对队列中的 `expected_source_commit_id`，生成期间再次失效会整体拒绝该项，不会误删新队列。
5. Store 新增只读 `get_ready_summary_record`：只有对应 `summary_record` 存在且同作用域没有 `summary_invalidation` 时才返回。旧记录允许留在派生缓存表中，但不具备 Context 资格。
6. `OpenedProject::summary_context` 必须绑定当前项目 HEAD、目标 Block ID/revision/content hash，并重新遍历活动父链。它只返回当前 Document、祖先 Document 与 Project 的 ready summary；归档、失效、目标变化、父链异常或旧 HEAD 均不能返回上下文。
7. Host 返回的候选固定为 `summary` render mode、`local_sensitive` 敏感度；本地提取结果标记 `source_derived`，未来模型生成结果标记 `model_inferred`。source ref 含作用域与 source Commit，便于发送确认和 Operation 审计追踪。
8. Tauri 使用独立 `allow-summary-worker` permission 暴露 `refresh_summaries` 与 `get_summary_context`，不暴露任意摘要写入、SQLite、正文作用域或生成器配置。页面在 800ms 空闲后按 12 项批处理，成功时快速排空，失败时执行最高 30 秒的指数退避。
9. Context Source 在编译时领取目标绑定摘要，并在 JavaScript 中重新计算摘要内容 SHA-256；与 Host 返回的 `sourceHash` 不一致时，在任何 Provider 授权和网络请求之前失败。通过校验的 Document/祖先摘要进入 L2，Project 摘要进入 L3，仍由 Context Compiler 的层级预算和隐私策略决定是否入选。

## 安全与产品边界

- 后台 worker 绝不隐式调用 DeepSeek、Qwen、Kimi、MiniMax 或 Ollama，因此不复用一次性正文模型 capability，也不会在用户不知情时产生费用或发送内容。
- 云端或本地模型摘要属于后续可插拔生成器。启用前必须增加显式项目策略、费用/隐私同意、独立授权作用域、重试上限和失败隔离；不能只把 Provider 配置塞进当前后台命令。
- 页面仍能读取工作区正文用于当前编辑器，但不能决定摘要作用域、source Commit/hash 或直接消费队列。所有实际发送内容继续显示在 Context 确认弹窗中。

## 测试要求

- Store 覆盖 ready → invalidated → ready 的读取状态变化，以及旧 source Commit 写回冲突。
- Host 覆盖分批顺序、剩余计数、provider/model 元数据、当前摘要读取、正文更新后失效、旧 HEAD/Block 基线拒绝和刷新后恢复。
- Tauri 覆盖 request schema、批量上限、目标绑定与新 permission/command 清单。
- Node 覆盖摘要来源进入 ContextPacket、摘要哈希复算和在 Provider 授权之前完成领取。
- 浏览器回归等待“摘要已就绪”，并在发送确认中验证 L2/L3 及含 source Commit 的项目摘要来源。

## 后果

- 首个可运行 worker 优先保证无费用、可测试和失效安全，摘要质量只是提取式基线，不宣称等同于模型抽象总结。
- 对大项目逐作用域构造摘要仍可能重复遍历；批量上限、输入截断和空闲调度限制峰值。后续可在不改变 Store 协议的前提下加入增量层级物化与任务优先级。
- Context 资格已有 Host 入口，但局部正文、风格样本和完整 Packet 仍在页面侧编译；这些来源继续下沉是下一阶段安全工作。
