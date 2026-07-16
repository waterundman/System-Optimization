# ADR 0026：确认后的 Context Packet 能力

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：FR-05 最终 Packet、模型 Prompt 与出站能力的 Host 强制绑定

## 背景

ADR 0025 已把 L0—L4 候选统一迁到 Host，但确定性预算、敏感策略和最终 Packet hash 仍在 WebView 的 Kernel 中产生。ADR 0018 的一次性模型 capability 只保存 Packet ID/hash，无法证明页面提交的完整模型 Prompt 确实只包含该 Packet，也无法阻止被篡改页面重签一个伪造的评分、token 或排除结果。

正常页面仍需要复用跨宿主 Kernel Context Compiler，并在联网前向用户展示最终发送内容。因此本阶段不复制用户交互，而是在发放计费能力前，由 Host 对用户确认后的最终载荷执行独立、失败关闭的复算。

## 决策

1. `authorize_model_request` 必须同时提交 Context Packet 的完整 JSON、Packet ID/hash 绑定、目标 UTF-16 `from/to` 和将要执行的完整模型请求。缺少其中任一项都不能获得 capability。
2. Host 使用 `deny_unknown_fields` 的严格 DTO 解析 Packet，移除 `packetHash` 后按 Kernel 的递归键排序 stable JSON 规则重新计算 SHA-256，并要求计算结果同时匹配 Packet 与授权绑定。
3. 在持有当前项目会话锁时，Host 基于当前 HEAD、目标 Block revision/hash 和 UTF-16 选区重新收集全部 L0—L4 候选。Packet 必须对每个候选恰好给出一个入选或排除结果，且必须包含唯一 mandatory L0。
4. Host 逐项复核 source ref/hash、内容、状态、authority、sensitivity、render mode、reason codes 和 mandatory 标志；同时独立复算 Unicode tokenizer 估算、六位评分、Operation Profile 层级预算、借用规则和最终选取结果。
5. `archived/rejected/deleted`、未固定 draft 和远程 Provider 的 `never_send` 只能分别产生受控排除原因。页面不能把策略排除改写成预算排除，也不能通过伪造 token 或评分重新引入内容。
6. 模型请求只能包含一个固定 system message 和一个严格的 user JSON envelope，不允许 tools/tool choice。Host 验证操作类型、强度、目标、硬约束、输出预算和输出 contract，并要求 Prompt 中 Context items 与确认 Packet 的入选项逐字段、逐顺序一致。
7. 通过验证后，完整不可变 Packet JSON 与复算 hash 一并进入最多 120 秒、只可消费一次的 `ModelAuthorizationScope`。其 `Debug` 实现只显示 ID、hash 和字节数，不输出 Packet 内容。
8. 项目保存、关闭、取消、过期或首次消费都会使 capability 失效；执行 IPC 仍只接收 capability ID，不能在执行阶段替换 Prompt 或 Context。

## 安全性质

- 被篡改 WebView 可以拒绝发起操作，但不能让未知来源、过期来源或远程 `never_send` 内容获得出站能力。
- Packet hash 不再只是页面自证；Host 对载荷、权威候选集合和实际模型消息做三方一致性校验。
- Context 正文只存在于确认载荷、待执行请求和最终本地审计所需的数据路径；capability 调试输出及公开错误不回显正文或 secret。

## 代价与限制

- 授权前会再次读取并评分候选，同时在 Host 内短暂保存一份 Packet；上限为 8 MiB、2,048 个条目和 32 个待执行 capability。
- Kernel 与 Host 各有一份确定性编译规则。两边版本常量、评分和预算算法变更必须同步，并由跨层合同测试阻止漂移。
- 当前受控操作协议只接受五种内置 Operation。第三方 Operation 必须先扩展可信协议，不能通过自由 Prompt 绕过。

## 测试要求

- 覆盖 stable JSON/hash、token、评分、预算与候选全覆盖复算；
- 覆盖绑定 Packet 替换、重签后的评分篡改、Prompt Context 替换；
- 覆盖远程 `never_send` 排除原因篡改、旧 HEAD/Block/UTF-16 选区；
- 覆盖 capability 单次消费、过期、取消、项目关闭和调试输出脱敏；
- 浏览器测试继续证明用户确认先于授权，授权先于模型流执行，提交载荷包含所展示的完整 Packet。
