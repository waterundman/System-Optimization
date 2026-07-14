# optimizer-store

本地项目的 SQLite 权威存储层，当前实现：

- bundled SQLite 3.51.3 运行时硬校验；
- WAL、`synchronous=FULL`、外键与 busy timeout；
- 严格表、版本化 migration、FTS5 派生索引；
- Project/Document/Block 初始种子；
- Block 编辑、edit journal、Commit、parent、branch head 与 ChangeSet 的单事务写入；
- 乐观并发冲突，禁止 last-write-wins；
- 与 Commit root hash 绑定的不可变物化快照；
- SQLite Online Backup 与完整性检查。

存储层不负责生成 ID、内容哈希或压缩快照；这些由上层端口提供。这样可以保持模型供应商和宿主无关。

