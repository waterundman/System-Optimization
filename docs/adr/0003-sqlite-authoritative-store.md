# ADR-0003：SQLite 项目库作为桌面端权威源

- 状态：Accepted
- 日期：2026-07-14

## 决策

桌面端以 SQLite 项目库作为文档结构、Block、编辑日志、Commit DAG、ChangeSet 和物化快照的权威源。Markdown/JSON/DOCX 是可验证导出，不与数据库进行无仲裁双写。

使用 `rusqlite 0.39.0` 的 bundled SQLite 3.51.3，并在运行时拒绝低于 3.51.3 的版本。数据库启用 WAL、`synchronous=FULL`、外键、busy timeout、严格表和 FTS5。

## 不变量

- Block 更新必须携带 expected revision/hash。
- 编辑、journal、Commit、parent、ChangeSet 与 branch head 在同一事务提交。
- Commit、parent、journal、ChangeSet 和 snapshot 不可原地更新或删除。
- Snapshot 必须与 Commit root hash 一致。
- 备份使用 SQLite Online Backup API，不能只复制运行中的 `.sqlite3` 文件。

## 取舍

稳定 ID、事务和恢复能力优先于“数据库文件可直接手工编辑”。开放性通过 Markdown/JSON 导出和可迁移 schema 保证。

