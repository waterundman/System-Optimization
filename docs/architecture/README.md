# 架构文档

本目录中的 DOCX 是 v1.0 工程设计基线。实现若偏离其中的 MUST 决策，应新增 ADR，说明背景、选择、代价、迁移和回滚方案。

当前代码对应路线图：

- M0：协议、中文 diff/WAL/编辑器等风险验证。
- M1：Optimizer Kernel Foundation。
- M2（进行中）：多模型网关、Operation 执行闭环、模型产物与审查审计存储、Tauri IPC、Secret Store、`.optimizer` 原子项目包与单项目桌面 Session。

下一段实现文档树读取、Block 自动保存和版本提交/恢复命令。manifest 已将项目、主分支与 SQLite 权威状态绑定，Operation 命令只能访问当前已校验会话。
