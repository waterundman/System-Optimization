# ADR 0028：持久审查候选与快照分支

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：FR-07 跨会话候选、审查恢复与候选分支

## 背景

Patch Proposal、review head 和不可变 review event 已经持久化，但桌面端只持有当前内存中的审查会话。关闭抽屉或重新打开项目后，用户无法继续未完成审查；想保留一个已接受部分的版本时，也只能立即应用到主工作区或放弃整个候选。

候选恢复不能信任 WebView 缓存的正文、decision 或 hash。候选分支也不能通过临时覆盖主工作区再创建检查点来实现，否则会移动主分支 HEAD、触发无关摘要失效，并在崩溃窗口内暴露半应用状态。

## 决策

1. `model_artifact` 中的不可变 Patch Proposal、`patch_review_head` 和按 revision 排序的不可变 review event 共同构成候选的权威来源。WebView 不持久化可恢复审查真相。
2. Host 提供候选元数据列表和按 proposal ID 加载详情两个入口。列表最多返回 100 项，不包含完整 artifact payload；详情按需读取 payload，复算 SHA-256、校验 artifact metadata，并重放全部 review event。
3. 重放 decision 时，以 Patch Proposal 的 atomic group 为边界传播一个审查事件。一个组只需要一条权威事件，恢复结果必须覆盖每个 hunk，且与 review status 一致；缺项、重复键、非法状态或 hash 不一致均失败关闭。
4. 桌面端“候选中心”区分进行中和历史候选。进行中候选可以重新打开；终态候选保留审计可见性。发生基线冲突的候选保持只读，自动 rebase 另行设计。
5. schema v7 增加不可变 `patch_candidate_branch` 映射，将一个 proposal 绑定到唯一 branch、Commit 和 snapshot。更新和删除由数据库触发器拒绝。
6. “保存为分支”只允许 ready、至少一个 accepted hunk、当前项目 HEAD 与 operation base 完全一致、目标 Block revision/hash 未变化且未锁定的候选。Host 从当前权威快照在内存中应用候选正文，再创建独立 Commit、branch 和确定性物化 snapshot。
7. 候选 Commit 的 parent 是 operation base；创建候选分支不移动 `main` HEAD，不写全局 Block/Document revision，不写 edit journal，不改变 review status，也不触发主工作区摘要失效。
8. 候选快照是分支实现细节，`latest_snapshot` 与用户检查点列表必须排除它；Commit DAG 和候选中心仍可追踪对应 branch/Commit/snapshot ID。
9. 每个 proposal 只允许创建一个候选分支。重复创建、旧基线、已变更目标或不完整审查均返回确定性错误，不做隐式重命名、重放或覆盖。

## 结果

- 未完成审查可以跨抽屉、跨项目会话恢复，decision 与 revision 仍以 Host 审计为准。
- 用户可以保留候选版本而不触碰当前正文或主分支历史。
- 候选列表读取成本与 Proposal 大小解耦；完整正文只在用户打开单个候选时跨 IPC。
- 当前分支创建采用严格 exact-base 语义。主工作区变化后的 rebase、候选分支切换和合并 UI 属于后续增量，不能通过静默覆盖替代。

## 测试要求

- Store 测试覆盖 schema v7 迁移、元数据列表、候选分支原子创建、重复拒绝、数据库重开后可恢复、主工作区不变，以及候选快照不出现在检查点列表；
- Host 测试覆盖 artifact/hash/status 校验、atomic group 重放、候选详情恢复、exact-base 分支创建和随后正常应用；
- 前端测试覆盖详情 hydrate 的严格字段/decision/hash 校验、稍后审查、候选中心、重新打开、保存分支与应用；
- Store invariant 必须验证 proposal、branch、Commit 与 snapshot 映射完整一致。
