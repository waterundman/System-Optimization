# 架构文档

本目录中的 DOCX 是 v1.0 工程设计基线。实现若偏离其中的 MUST 决策，应新增 ADR，说明背景、选择、代价、迁移和回滚方案。

当前代码对应路线图：

- M0：协议、中文 diff/WAL/编辑器等风险验证。
- M1：Optimizer Kernel Foundation。
- M2（进行中）：多模型网关、Operation 执行闭环、模型产物与审查审计存储、Tauri IPC、Secret Store、`.optimizer` 原子项目包、版本化工作区与可运行桌面壳。

M2 已具备项目入口、文档树、Block 自动保存、冲突草稿保护、版本历史、检查点恢复、Rust 原生四云端 Provider 与固定回环 Ollama 流式执行、一次性宿主模型 capability、桌面 AI 操作与原子 Patch 审查应用。当前正文输入采用零依赖 `contenteditable` 适配器；Tiptap 集成延后到依赖源可用时，不改变版本协议。下一段把 Context 来源与隐私资格校验继续下沉宿主，并加入结构化摘要失效队列。
