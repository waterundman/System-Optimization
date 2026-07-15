# ADR-0012：版本化工作区读取、乐观自动保存与检查点恢复

- 状态：Accepted
- 日期：2026-07-15

## 背景

项目 Session 建立后，WebView 仍无法读取文档树或保存正文。若前端直接操作 SQLite，就会绕过稳定 ID、内容哈希、编辑日志、Commit DAG、项目 HEAD 与快照完整性；若只校验 Block revision，则另一个进程对其他 Block 的提交可能在根哈希计算和写事务之间发生，产生不代表真实项目状态的 Commit。

工程设计还要求 AI 操作前建立可恢复基线，并且恢复必须形成新版本，不能静默回写历史状态。

## 决策

在 `OpenedProject` 会话上增加三组版本化能力：

1. `workspace()` 读取项目、主分支、HEAD、项目 revision、文档树和全部活动 Block。SQLite 中的 `content_json` 必须在 Host 内解析成 JSON value；损坏内容不会作为字符串透传给 WebView。
2. `save_block()` 接收 Block ID、expected revision/hash、结构化内容与纯文本。Host 生成 edit/commit ID 和时间戳，计算内容哈希与项目根哈希；Store 在一个 `IMMEDIATE` 事务中同时校验 Block revision/hash、项目 revision 与分支 HEAD，再写 Block、EditJournal、Document revision、Commit、ChangeSet、分支 HEAD 和项目 HEAD。
3. `create_checkpoint()` 将当前 HEAD 编码为确定性、带 checksum 的物化快照；`version_history()` 只返回 Commit 与检查点元数据，不返回压缩 payload；`restore_checkpoint()` 校验快照后恢复内容，并把恢复记录为拥有当前 HEAD 父节点的新 Commit。

每次成功 Block 自动保存形成一个 `reason=autosave` Commit。相同内容返回稳定的 `NO_CHANGES`，不增加 revision 或审计记录。恢复时每个受影响 Document 只增加一次 revision，便于编辑器安全刷新。

## 并发与哈希

保存使用双层乐观并发：

- 局部基线：`expectedBlockRevision + expectedBlockHash`；
- 项目基线：Host 读取到的 `expectedProjectRevision + expectedHeadCommitId`。

项目根哈希 v1 按 Block ID 排序并绑定每个 Block 的内容哈希。当前结构编辑尚未开放，因此 v1 根只覆盖活动 Block 内容；未来引入新增、删除、移动和文档结构编辑时，必须升级根描述或证明现有编码已包含相应状态。

Block 内容哈希绑定 `kind + content + plainText + locked`，不绑定稳定 ID 与 order key，使内容相同的 Block 得到相同内容哈希。Host 是持久化哈希的最终权威，WebView 不提交 `newHash`、`newRootHash`、Commit ID 或时间戳。

## Tauri 权限

新增最小权限：

- `allow-workspace-read`：读取当前项目文档与 Block；
- `allow-workspace-write`：乐观保存单个 Block；
- `allow-version-read`：读取 Commit/检查点元数据；
- `allow-version-write`：创建和恢复检查点。

它们继续只授权给本地 `main` window。无项目时统一返回 `NO_PROJECT_OPEN`。

## 取舍与后续

每次 debounce 保存一个 Commit 简单、可审计且便于恢复，但高频输入会增加 Commit 数量。前端应采用停顿/失焦 debounce；后续可增加明确的 coalescing 策略，但不得删除已经被 AI 操作、检查点或用户命名版本引用的 Commit。

本文最初的“只支持替换既有 Block”限制已被 ADR 0016 部分取代：现在支持顶层章节创建、单文件 Markdown 导入、项目内 Markdown 导出和跨章节创建点的结构恢复。章节改名/移动/删除、Block 拆分合并与命名版本仍待实现。
