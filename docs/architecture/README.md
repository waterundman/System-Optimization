# 架构文档

本目录中的 DOCX 是 v1.0 工程设计基线。实现若偏离其中的 MUST 决策，应新增 ADR，说明背景、选择、代价、迁移和回滚方案。

当前代码对应路线图：

- M0：协议、中文 diff/WAL/编辑器等风险验证。
- M1：Optimizer Kernel Foundation。
- M2（进行中）：多模型网关、Operation 执行闭环、模型产物与审查审计存储、Tauri IPC、Secret Store、`.optimizer` 原子项目包与单项目桌面 Session。

M2 已具备文档树读取、Block 自动保存、版本历史和检查点恢复命令。下一段进入可运行桌面入口与 Tiptap 编辑器壳，并把模型网络执行迁入 Rust 信任边界。
