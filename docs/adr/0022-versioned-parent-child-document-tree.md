# ADR 0022：版本化父子章节树

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：子章节创建、同级排序、缩进/移出、子树归档与摘要传播
- 扩展：ADR 0019 的顶层相邻移动限制

## 背景

Document 从首版 schema 起已有 `parent_id`，项目根哈希 v2 和检查点也能绑定/恢复父子关系，但桌面命令仍只创建顶层章节并按全局列表移动。这会导致“数据模型是树、产品只能平铺”的断层，也会让从检查点恢复出的嵌套结构缺少安全编辑入口。

树操作不能由 WebView 直接提交任意 parent/order。缩进、移出和同级交换都可能产生环、指向归档父节点、兄弟顺序冲突或越过并发 HEAD。归档父节点若只归档单行，则活动子节点会悬挂在归档父节点下。

## 决策

1. `CreateDocumentSpec` 增加可选 `parent_id`。Host 只接受当前项目的活动父节点，并为该兄弟组生成追加顺序键；Store 再次验证父节点和同父顺序键唯一性，包括 SQLite 对 NULL 唯一约束不能覆盖的顶层节点。
2. 上移/下移只在同一父节点的兄弟组中生效。Host 按 `order_key + id` 得到权威顺序、交换相邻 ID，并只重写该兄弟组的顺序键。
3. 新增 `change_document_depth`：
   - 缩进把当前节点设为上一兄弟节点的最后一个子节点；首个兄弟不可缩进。
   - 移出把当前节点放到父节点之后，父级改为祖父节点；顶层节点不可移出。
   - 移动节点会携带其整棵活动子树，子孙 parent 不改写。
4. WebView 只提交稳定 Document ID、expected revision 和 `indent/outdent` 意图。父节点、目标位置、受影响兄弟、顺序键、Commit ID、时间和根哈希全部由 Host 计算。
5. `ApplyDocumentBatch` 新增 `document_reparent/reparent` 原因与操作。所有受影响 Document revision、ChangeSet、Commit、分支/项目 HEAD 和根哈希在一个 `IMMEDIATE` 事务中推进；最终树必须父节点存在且活动、无环、同父顺序键唯一。
6. 归档活动父节点会在同一批次中归档其全部活动后代，并要求项目至少留下一个活动节点。恢复保持显式逐层语义：必须先恢复父节点，再恢复子节点，避免无证据地复活过去单独归档的后代。
7. 摘要失效除每个实际变更 Document 外，还包含变更前父节点和变更后父节点。这样子节点换父、归档或恢复不会留下陈旧的父级聚合摘要；Project 摘要仍随每个结构 Commit 失效。
8. 桌面侧栏从权威 `parentId + orderKey` 构造先序树。上移/下移按钮按兄弟边界禁用；选中节点显示缩进、移出和新建子章节操作。树渲染包含循环/孤儿防御，但 Host/Store invariant 才是安全来源。

## 归档与恢复

- 子树归档只软删除 Document，保留其 Block、父子 ID、顺序键、Commit 历史和摘要记录。
- 所有归档 Document/Block 从活动工作区、FTS、导出、Context 和摘要失效队列排除。
- 恢复父节点后，后代仍显示在归档区；逐个恢复时重新登记 Document 和活动 Block 摘要失效。
- 检查点恢复可一次恢复历史树状态，仍创建新 Commit，并保持 Document revision 单调增加。

## 测试要求

- Host 覆盖根节点缩进为子节点、同父移动、移出到父后、根哈希/Commit reason 和父级摘要失效。
- Store 覆盖顶层 NULL parent 的重复顺序键拒绝、reparent reason/operation、环和归档父引用拒绝。
- 子树归档测试必须证明后代一并离开活动工作区、先恢复子节点失败、按父到子恢复成功。
- Tauri 覆盖新命令 schema、permission、revision 冲突；浏览器回归覆盖树缩进标记、移出和原有 AI 全流程。

## 后果

- 章节结构现在从存储、版本、快照、摘要到 UI 都使用同一棵权威树，不再把 `parent_id` 当作未实现占位字段。
- 当前顺序更新仍会递增目标兄弟组全部 Document revision；MVP 规模可接受。超大树需要稀疏排序键和局部重平衡，但不得改变乐观并发语义。
- 当前交互提供结构按钮，不包含拖拽。未来拖拽必须编译为相同的 Host 意图/批处理，不能直接在页面中改 parent/order 后提交结果。
