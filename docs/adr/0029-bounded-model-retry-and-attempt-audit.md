# ADR 0029：有界模型重试与不可变尝试审计

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：FR-06 模型失败恢复、重试 UI 与审计

## 背景

模型网关已经统一返回 `retriable`、`Retry-After`、HTTP 状态和远端 request ID，但此前 Operation Runner 不执行重试，桌面端也只能显示一次失败。直接在网关内部重试计费型 POST 会隐藏真实请求次数，并可能在已经收到模型输出后生成第二份不一致内容；复用一次性 Host capability 又会绕过其单次消费安全语义。

重试必须同时满足费用有界、用户可取消、目标不漂移、每次出站重新授权，以及失败次数可审计且不保存 Prompt、响应正文、Header 或 Secret。

## 决策

1. 模型网关仍只执行一次 HTTP 请求，不在 Provider 层自动重试。它负责归一化并脱敏错误元数据。
2. Operation Runner 是自动重试的唯一决策层。只有 `ProviderError.retriable=true`、尚未出现 Provider `start` 事件、操作未取消、尝试次数未耗尽，且 `Retry-After` 不超过策略上限时才允许重试。
3. 策略硬限制为 1—3 次总尝试、基础等待不超过 5 秒、最大等待不超过 10 秒。桌面默认使用 2 次总尝试、500ms 基础等待和 5 秒上限；等待取指数退避与 `Retry-After` 的较大值。
4. 一旦收到响应开始事件，即使尚未产生正文，也不自动重试。取消、协议错误、结构化输出错误、授权错误和其他非 `retriable` 错误同样不自动重试。当前策略不做跨 Provider fallback。
5. 每次尝试重新调用 Provider stream。桌面 Host adapter 因而生成新的 request ID，重新收集并校验当前项目/HEAD/目标/Context Packet，并获取新的单次 capability；旧 capability 绝不复用。
6. schema v8 增加不可变 `operation_attempt`。每条记录只包含序号、时间、结果、是否已开始响应、响应 ID，以及有界且脱敏的错误码、错误类别、HTTP 状态、远端 request ID、`Retry-After`、是否可重试和选定等待时间。记录不包含错误正文、Prompt、Context 正文、响应片段、Header 或凭据。
7. Operation bundle 仍使用 v1 外层协议，通过可选 `attempts` 保持旧失败/预检记录兼容；新 Provider 执行会把尝试记录与 Operation、生命周期、Context 和 Artifact 在同一事务中落库。
8. 自动尝试耗尽后，桌面端只对仍标记为可重试的失败显示“重新尝试”。该动作创建全新的 OperationRun，不复活失败 Run；它精确绑定原 Block ID、revision、hash 与 UTF-16 范围，任一基线变化都在联网前以 `TARGET_STALE` 失败。
9. 用户显式重新尝试可以在先前响应已经开始后发起，但仍是一个独立的新 Operation；先前部分输出不复用、不拼接，也不会产生 Artifact。

## 结果

- 暂时性 429、408、5xx、网络和超时错误可以在未开始响应时自动恢复，最坏请求次数与等待时间均受硬上限控制。
- 每次云端出站都重新通过一次性 capability 和当前权威基线，不会把“重试”变成授权重放。
- 用户可以区分一次 Operation 内的自动尝试和显式创建的新 Operation；失败历史保留，不被成功重试覆盖。
- 响应开始后的失败可能要求用户手动重新尝试，这是避免重复计费和双重生成的刻意取舍。

## 测试要求

- Runner 覆盖响应前成功重试、响应开始后不重试、超过等待上限不重试、等待期间取消、策略上限和尝试元数据限长；
- 桌面 Host 集成覆盖每次尝试使用新 request ID 与新 capability，并把两次尝试原子持久化；
- Store/Host 覆盖 schema v8 迁移、连续序号、成功必须为最后一次、不可变触发器、严格 DTO 与聚合审计；
- TypeScript 与 Rust 继续读取同一共享夹具；错误消息、远端 code 与 request ID 的 Secret 脱敏必须有回归测试。

## 回滚

可把桌面策略设置为 `maxAttempts=1` 并移除显式重试按钮，保留 schema v8 的只读审计数据。数据库降级仍必须恢复迁移前在线备份，不能手工降低 `user_version`。
