# optimizer-store

本地项目的 SQLite 权威存储层，当前实现：

- bundled SQLite 3.51.3 运行时硬校验；
- WAL、`synchronous=FULL`、外键与 busy timeout；
- 严格表、版本化 migration、FTS5 派生索引；
- Project/Document/Block 初始种子；
- Block 编辑、edit journal、Commit、parent、branch head 与 ChangeSet 的单事务写入；
- 乐观并发冲突，禁止 last-write-wins；
- 与 Commit root hash 绑定的不可变物化快照；
- 确定性 `optimizer-json+zstd` 编码、SHA-256 校验和压缩/解压上限；
- 快照恢复生成新 Commit、edit journal 与 ChangeSet，不改写已有历史；
- SQLite Online Backup 与完整性检查。
- schema v2 的 ContextPacket、OperationRun、模型用量、生命周期和模型产物审计存储；
- Operation bundle 单事务落库，任何晚期 artifact 冲突都会回滚此前写入；
- Patch review head、不可变 review event 与 expected-revision 乐观并发；
- 审查接受、拒绝、冲突和 rebase 与 Operation 生命周期原子同步；
- 文件数据库从旧 schema 升级前自动创建并 `quick_check` 的在线备份，路径可由 `StoreDiagnostics.migration_backup` 获取。
- schema v5 的项目/章节/Block 分层摘要记录，以及与正文、树结构 Commit 同事务更新的合并失效队列；子节点换父、归档或恢复会同时使相关父级聚合摘要失效；
- 摘要完成必须匹配该作用域当前排队的 source Commit，生成期间再次失效会以乐观冲突拒绝陈旧写回；
- ready summary 读取以“不存在同作用域失效项”为必要条件；旧记录可留作派生缓存历史，但不能被 Host 作为当前 Context 返回；
- schema v6 的项目事实/约束资产，显式区分 `canonical/archived/rejected`、authority、敏感级别与硬/软约束，并以 revision 拒绝并发状态覆盖；

恢复支持跨父子章节创建、改名、同级排序、缩进/移出、子树软归档和复活；恢复仍创建新的 Commit，不改写旧节点。归档章节不参与活动 FTS、工作区、导出或摘要队列，恢复时其活动 Block 会重新失效以避免漏掉未完成摘要。

存储层不负责生成 ID 或正文内容哈希；调用方必须显式提供。快照编码由存储层统一完成，以固定格式、版本和安全上限。

Operation payload 使用严格 JSON 列和独立绑定 hash 保存。ContextPacket 与 PatchProposal/Findings 不会因为审查状态变化而被改写；可变 head 只保存当前 revision/status，完整历史以不可变事件为准。
