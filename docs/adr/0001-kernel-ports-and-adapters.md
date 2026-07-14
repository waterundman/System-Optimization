# ADR-0001：Optimizer Kernel 使用纯领域核心与 Ports/Adapters

- 状态：Accepted
- 日期：2026-07-14

## 背景

同一套优化能力需要运行在独立桌面端、Obsidian 以及未来其他优化器产品中。若领域逻辑直接依赖 Tauri、SQLite、编辑器对象或模型 SDK，将难以复用、测试和迁移。

## 决策

领域类型放在 `@optimizer/protocol`；操作状态机、上下文编译、策略与版本规则放在 `@optimizer/kernel`。Kernel 只依赖协议和纯函数，通过 Ports 请求文档、知识、检索、模型、密钥、时钟、哈希与遥测能力。

Tauri/Rust、SQLite、Obsidian、OpenAI-compatible 与 Ollama 均作为 Adapter。

## 约束

- Kernel 不得导入 React、Tauri、SQLite、Obsidian 或厂商 SDK。
- 所有 IPC、插件和 Provider 边界必须使用版本化协议并进行运行时校验。
- 主机权限不得泄漏为 Kernel 的隐式全局对象。

## 代价

需要维护端口、适配器与协议版本；早期代码量高于直接在 UI 中调用模型。但这换来了跨宿主复用、确定性测试和更清晰的安全边界。

