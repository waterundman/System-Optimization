# ADR-0004：快照恢复不改写历史，编辑器修改必须经过事务桥接

- 状态：Accepted
- 日期：2026-07-14

## 决策

项目快照使用确定性的规范 JSON，并以 zstd level 3 压缩。数据库记录 `optimizer-json+zstd` codec、版本和压缩载荷 SHA-256；读取时先验证载荷大小与校验和，再限制解压后的最大尺寸并执行严格 schema 校验。

恢复快照不会移动到旧 Commit 或覆盖历史。系统比较当前分支状态与快照，针对变化 Block 写入 edit journal 和 ChangeSet，创建一个以当前 head 为父节点的新 Commit，再原子更新 branch head。

编辑器集成通过独立的 `@optimizer/editor-bridge` 完成。桥接层接收稳定 Block 快照、UTF-16 选区和带 expected revision/hash 的事务，输出可审计的变更集合；它不直接依赖 Tiptap 或 UI 框架。

## 不变量

- 相同规范状态必须编码出相同快照 payload 和 checksum。
- codec/version、数据库描述符、payload 描述符与 Commit root hash 必须一致。
- 解压必须有上限，校验失败或未知字段不能继续恢复。
- 恢复只能产生新 Commit，不修改、删除或复用历史 Commit。
- MVP 恢复要求文档和 Block 结构一致；结构变化显式失败。
- MVP 选区必须位于同一 Block，且偏移是合法 UTF-16 边界。
- 编辑事务必须校验 document revision、Block revision/hash、锁定状态和 order key 唯一性。
- 单个规范事务不能重复修改同一 Block。
- Block 内容哈希只覆盖内容语义，不覆盖稳定 ID 或排序位置。

## 取舍

恢复为新 Commit 会增加历史节点，但保留完整因果关系，撤销与审计更可靠。MVP 暂不支持跨 Block 选区和结构恢复，以避免在没有稳定位置映射和结构操作日志时制造静默数据错配。
