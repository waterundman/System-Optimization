# 架构文档

本目录中的 DOCX 是 v1.0 工程设计基线。实现若偏离其中的 MUST 决策，应新增 ADR，说明背景、选择、代价、迁移和回滚方案。

当前代码对应路线图：

- M0：协议、中文 diff、WAL、编辑器等风险验证。
- M1：Optimizer Kernel Foundation。
- M2（进行中）：多模型网关、Operation 执行闭环、模型产物与审查审计、Tauri IPC、Secret Store、`.optimizer` 原子项目包、版本化工作区与可运行桌面壳。

M2 已具备宿主最近项目入口、版本化父子章节生命周期、Block 自动保存、冲突草稿保护、版本历史和检查点恢复。项目根哈希 v2 同时绑定活动章节 parent/order 元数据与 Block 内容；结构或正文变化均通过 Commit DAG、乐观并发和快照恢复，不做静默覆盖。

Context 主链已下沉到 Host 权威基线：schema v5 提供项目/章节/Block 分层摘要与事务性失效队列，schema v6 提供 canonical/archived/rejected 事实与约束，项目风格样本作为 L4 来源。统一 Operation Context 命令生成目标、局部正文、结构、摘要、知识和风格候选；Kernel 负责预算和外发策略，用户确认后 Host 基于当前 HEAD 重新收集来源，复算 Packet stable hash、token、评分、预算、`never_send` 排除和受控 Prompt。完整 Packet 进入短期单次 capability，任何 Packet/Prompt/HEAD/目标篡改或重放都会失败关闭。

模型执行由 Rust 原生支持 DeepSeek、Qwen、Kimi、MiniMax 固定官方端点和固定回环 Ollama。桌面五种操作共享 Context Compiler、Operation Runner 与 Patch Engine；Patch/Findings 经严格 JSON 校验后才进入审查。FR-06 的自动恢复采用响应开始前有界重试：桌面默认最多两次，每次生成新 request ID 并重新取得 Host capability；schema v8 将每次尝试作为有界、脱敏、不可变审计与 Operation 原子落库。耗尽后显式重试创建新 Operation，并精确绑定原 Block revision/hash/UTF-16 范围。

Patch 审查支持逐项和全部接受/拒绝，批量动作仍逐 revision 写入不可变事件。schema v7 的候选中心可从 Proposal/Review event 跨会话恢复，并把 ready 候选保存为独立 Commit 与物化快照而不移动主 HEAD；候选快照不进入用户检查点列表，分支创建采用 exact-base 冲突保护。

当前正文输入采用零依赖 `contenteditable` 适配器；Tiptap 集成延后到依赖源可用时，不改变版本协议。下一阶段优先推进 FR-09 的安全通用 OpenAI-compatible 配置、目标白名单和能力探测，其后补齐费用/反馈可观测性、备份恢复向导、性能与可访问性门禁，以及插件宿主。
