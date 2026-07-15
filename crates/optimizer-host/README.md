# optimizer-host

Rust 宿主安全边界与跨语言命令适配层。

当前实现：

- `ProjectRoot` 校验项目相对路径，拒绝绝对路径、`..` 与目录逃逸；
- `OperationCommandHost::persist_operation_bundle_json` 严格接收 v1 Operation bundle，并在一个 Store 事务中持久化；
- `OperationCommandHost::append_review_event_json` 使用 expected revision/status 写入审查事件；
- `OperationCommandHost::get_operation_audit` 聚合运行、ContextPacket、生命周期、Artifact 与 Patch review 历史；
- 所有输入 DTO 使用 `deny_unknown_fields`，协议版本不匹配或嵌套未知字段会在写库前失败；
- ContextPacket、Artifact 与 review 的开放 JSON payload 会递归拒绝 credential、API key、Authorization 和原始 Header 字段；
- TypeScript 与 Rust 读取同一份 `packages/protocol/fixtures/operation-persistence-bundle.v1.json` 夹具，避免边界字段漂移。

下一阶段在这些无框架 API 外注册 Tauri commands/capabilities，并加入 OS Keychain。Provider 网络域名策略与 WASM 插件资源限制仍属于后续宿主能力。

任何文件命令都必须先经过 `ProjectRoot`，不得接受未经校验的绝对路径或 `..`。宿主命令不得接收 API Key 或原始 HTTP Header，只接收协议允许的审计数据。
