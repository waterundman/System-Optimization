# @optimizer/editor-bridge

编辑器运行时与 Optimizer Kernel 之间的纯 TypeScript 边界。该包不依赖 Tiptap、React、Tauri 或 Node 系统 API。

当前职责：

- 将同一稳定 Block 内的 UTF-16 选区转换为 `OperationTarget`；
- 拒绝切开代理对的无效文本偏移；
- 编译 replace/insert/delete/move 的乐观并发编辑事务；
- 保护锁定 Block、文档 revision、Block revision/hash 与唯一 order key；
- 从序列化的 Tiptap 顶层节点读取 Optimizer 稳定属性。

真实编辑器只能在事务重新校验成功后应用 `CompiledEditorTransaction`，不能直接把模型输出写入正文。
