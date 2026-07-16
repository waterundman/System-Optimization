# ADR 0024：Host 权威事实与约束库

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：FR-08 长期知识、约束与 Context 资格

## 背景

风格样本和分层摘要已经能够进入 Context，但项目仍缺少显式事实与约束。如果页面临时把设定拼进 Prompt，Host 无法判断它是否属于当前项目、是否已废弃、能否外发，也无法在目标 Block 变化后拒绝旧 Context。把所有历史段落统一向量检索还会重新召回已删除剧情。

## 决策

1. SQLite schema v6 增加项目级严格表 `knowledge_item`，类型固定为 `fact` 或 `constraint`，状态固定为 `canonical`、`archived` 或 `rejected`。
2. 约束必须声明 `hard` 或 `soft`；事实不允许 severity。条目同时保存 authority、sensitivity、内容 SHA-256 与乐观并发 revision。
3. 当前桌面入口创建的知识一律标记为 `user_confirmed`。外部导入或模型推断将来必须走不同的受控入口，WebView 不能自行提交 authority。
4. `get_knowledge_context` 必须同时绑定当前项目 HEAD、目标 Block ID、revision 与 content hash。Host 只返回当前项目的 canonical 条目；归档和拒绝条目不具备 Context 资格。
5. Host 把事实渲染为 `事实【标题】：内容`，把约束渲染为 `硬约束/软约束【标题】：内容`，计算本次候选的 SHA-256，并以 L3、`constraint` render mode 和稳定 reason code 返回。
6. WebView 对返回内容重新计算 SHA-256，并确认 source Commit 与当前 HEAD 一致后才交给 Context Compiler。远程 Provider 排除 `never_send`；固定回环 Ollama 可在发送预览确认后使用。
7. 知识库使用独立 Tauri `allow-knowledge-library` permission；页面不存在数据库直读或 authority 写入命令。

## 结果

- 风格、摘要、事实与约束保持类型隔离，废弃设定不会通过统一 RAG 意外复活。
- 用户能在发送前看到实际入选的事实/约束及被策略排除的本地条目。
- 知识条目当前使用资产 revision，不进入正文 Commit/Checkpoint 图；恢复旧正文不会静默回滚用户随后确认的事实。统一资产提交图仍是后续扩展。
- 本轮不实现自动事实抽取、向量检索或模型生成约束；它们必须先产生非 canonical 提案，再由用户确认。
