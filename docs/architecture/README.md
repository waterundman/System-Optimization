# 架构文档

本目录中的 DOCX 是 v1.0 工程设计基线。实现若偏离其中的 MUST 决策，应新增 ADR，说明背景、选择、代价、迁移和回滚方案。

当前代码对应路线图：

- M0：协议、中文 diff/WAL/编辑器等风险验证。
- M1：Optimizer Kernel Foundation。
- M2（进行中）：多模型网关、Operation 执行闭环、模型产物与审查审计存储、Tauri IPC、Secret Store、`.optimizer` 原子项目包、版本化工作区与可运行桌面壳。

M2 已具备宿主最近项目入口、版本化父子章节创建/改名/同级排序/缩进/移出/子树软归档/逐层恢复、Block 自动保存、冲突草稿保护、版本历史、检查点恢复、Rust 原生四云端 Provider 与固定回环 Ollama 流式执行、一次性宿主模型 capability、桌面 AI 操作与原子 Patch 审查应用。项目根哈希 v2 同时绑定活动章节 parent/order 元数据和 Block 内容。SQLite schema v5 已加入项目/章节/Block 三层摘要记录与事务性合并失效队列，正文和结构提交不会留下未登记的陈旧摘要；父级聚合摘要会随子树结构变化失效。Rust 内置摘要 worker 已能在桌面空闲期以零外发生成器分批消费队列；只有无失效项且绑定当前 HEAD/目标 Block 的文档、祖先与项目摘要会作为 L2/L3 Context 候选返回。schema v6 进一步加入 Host 权威事实/约束库，明确 canonical/archived/rejected、authority、敏感级别与硬/软强度。统一 Operation Context 命令从同一 Host 基线生成目标、局部正文、结构、摘要、知识与风格候选，页面交给 Kernel 执行预算和外发策略；用户确认后，Host 会基于当前 HEAD 重新收集候选并独立复算最终 Packet 的 stable hash、token、评分、预算与策略，再验证实际模型 Prompt 与 Packet 完全一致。完整 Packet 作为不可变载荷进入短期单次 capability，远程 `never_send` 与任何 Packet/Prompt 替换都会失败关闭。桌面五种操作现由同一命令注册表驱动工具栏、右键、`Alt+1…5` 和可搜索命令面板；Patch 审查支持逐项及全部接受/拒绝，批量动作仍逐 revision 写入审计。最近项目注册表位于宿主用户配置区，按 `projectId` 打开并重新校验项目包，不作为项目真实性来源。当前正文输入采用零依赖 `contenteditable` 适配器；Tiptap 集成延后到依赖源可用时，不改变版本协议。下一阶段优先完成持久候选列表/分支入口和有限重试审计，并为摘要生成器增加显式授权的模型插件。
