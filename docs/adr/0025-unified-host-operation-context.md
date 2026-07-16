# ADR 0025：统一的 Host Operation Context 来源

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：FR-05 Context 来源资格与信任边界

## 背景

摘要和事实/约束已经由 Host 校验后返回，但目标选区、相邻正文、章节结构和风格样本仍由 WebView 从工作区缓存自行拼装。这样同一个 Context Packet 同时依赖权威 Store 与页面快照，无法用一次 HEAD/目标绑定证明所有来源来自同一时刻；页面也可能因为 UTF-16 选区、归档状态或内容哈希处理差异构造出错误候选。

## 决策

1. 新增只读 `get_operation_context`，一次提交当前 HEAD、目标 Block ID/revision/hash 以及 UTF-16 `from/to`。
2. Host 在一个已打开的项目 Store 快照中收集 L0 目标、L1 前后文、L2 章节结构、已就绪摘要、L3 canonical 事实/约束与 L4 风格样本。
3. Host 拒绝旧 HEAD、旧 Block 基线、反向选区、越界或切开代理对的 UTF-16 偏移。局部窗口最多 2,000 UTF-16 单元，并始终落在 Unicode 标量边界。
4. 每个候选都返回 source Commit、稳定 source revision/ref、Host 计算的 SHA-256、canonical policy、authority、sensitivity、render mode、reason codes、mandatory/selected 标志和评分信号。
5. L0 必须且只能有一个，并精确绑定本次目标；非 L0 不能被 Host 标为 mandatory。归档风格仍作为非合格候选返回，让 Kernel 产生可审计的 `INELIGIBLE_STATUS` 排除记录。
6. WebView 不再构造正文、结构或风格候选。它严格校验候选集合、目标身份、source Commit、枚举和评分范围，并为每项重新计算 SHA-256 后再交给 Kernel Context Compiler。
7. Tauri 使用独立 `allow-operation-context` permission。原有摘要/知识命令保留为可测试的窄接口，但桌面 AI 主链只使用统一命令。

## 结果

- 一次 AI 操作的全部 Context 来源来自同一个 Host 权威基线；页面缓存只用于再次核对 L0，不再决定来源资格。
- Kernel 仍负责确定性预算、优先级、Provider locality 策略、去重和 Packet hash，保持跨宿主复用。
- ADR 0026 已完成后续闭环：模型授权会重新收集本 ADR 定义的候选，复算最终 Packet 与实际 Prompt，并把完整确认载荷纳入短期单次 Host capability。
