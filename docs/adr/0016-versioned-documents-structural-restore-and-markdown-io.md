# ADR 0016：版本化章节、结构恢复与 Markdown I/O

- 状态：Accepted
- 日期：2026-07-15
- 决策范围：M2 文档管理、检查点和开放格式
- 取代：ADR 0004/0012 中“暂不支持文档创建与结构恢复”的阶段性限制

## 背景

桌面 MVP 只有初始化时创建的单个章节，无法形成真实长篇工作流。直接增加 Document CRUD 会使早期检查点的结构与当前结构不同；旧恢复逻辑会拒绝这种快照，破坏“恢复到任意检查点”的产品承诺。同时，项目需要一种显式、可验证的开放格式进出路径。

## 决策

1. `create_document` 每次创建一个顶层 `chapter` 和首个 `paragraph` Block。Document、Block、`document_create` Commit、父提交、两条 ChangeSet、分支头和项目 revision 在一个 SQLite immediate transaction 中完成。
2. Block 内容哈希和项目 root hash 由 Rust 宿主根据规范化内容计算；页面不能指定 Commit ID、root hash、order key 或 revision。
3. 快照恢复支持结构集合变化：当前多出的 Document/Block 被软归档，快照中存在但当前已归档的实体被复活。结构 ChangeSet 与恢复 Commit 在同一事务中写入，FTS 由 `deleted_at` trigger 同步。
4. Block revision 仍只由 EditJournal 推进。结构归档/复活不伪造内容编辑，也不增加 Block revision；Document revision 记录结构变化。
5. 相同稳定 ID 的 parent、kind、title、order key 或 locked 元数据发生无法解释的变化时，恢复失败并回滚，不猜测重排或改名意图。
6. Markdown 导入使用 WebView 原生文件选择器。只有用户显式选择的 `.md/.markdown/.txt` 文件会被页面读取，最大 2 MiB；内容通过 `create_document` 作为一个版本化章节保存，不授予 WebView 任意宿主文件读取能力。
7. Markdown 导出由 Rust 将当前权威 Document/Block 顺序编译为一个文件，先写临时文件再原子 rename 到项目包内的 `exports/`。页面不能指定任意目标路径。
8. 导出拥有独立 `allow-project-export` capability；导入复用已受约束的 `create_document` 命令。

## 结果

- 用户可以创建多章节项目，并可跨越章节创建时间点前后恢复检查点。
- 创建、导入和恢复均保留 Commit DAG 与 ChangeSet 审计，不产生数据库/Markdown 双写权威源。
- 原生文件选择器避免了“页面提交任意绝对路径，宿主代为读取”的 confused-deputy 风险；导出始终限制在当前项目包内。
- Rust 集成测试覆盖创建事务、陈旧 head 回滚、结构归档/复活、FTS/版本不变量和 Markdown 原子导出；浏览器回归覆盖创建、选择文件导入和导出反馈。

## 代价与后续

- 当前导入把一个文件保存为一个章节并保留原始 Markdown 文本，尚未按标题拆分为多个结构化 Block。后续解析器必须先生成可预览 Import Plan，再一次性提交，避免半批导入。
- 本文最初的“改名、移动、删除待实现”限制已由 ADR 0019 取代：顶层章节现支持版本化改名、相邻移动、软归档和恢复；嵌套与批量导入仍需要后续统一结构操作协议。
- 导出文件使用唯一名称保留历史，不覆盖已有导出。后续可在宿主受控的保存对话框中增加“另存为”。

## 回滚

可以移除创建/导入/导出入口和对应 capability；已有 Document/Block、快照与导出文件仍保持有效。结构恢复逻辑是旧内容恢复的兼容超集，无需降级数据库 schema。
