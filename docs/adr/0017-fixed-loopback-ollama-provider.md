# ADR 0017：固定回环 Ollama Provider

- 状态：Accepted
- 日期：2026-07-15

## 背景

Optimizer Kernel 已用 `ProviderLocality` 区分远程与本地执行，并规定 `never_send` 内容只能进入本地 Provider 的 Context Packet。此前可运行宿主只有四个云端 Provider，所有请求都假设 HTTPS、Bearer 凭据与系统代理，因此本地隐私路径尚未闭环。

Ollama 官方 OpenAI 兼容接口提供 `/v1/chat/completions`、SSE、JSON mode、tool calls、usage 与 reasoning 控制；默认服务监听 `localhost:11434`。官方 SDK 示例中的 API key 只是客户端必填占位，服务会忽略它，直接 HTTP 请求无需鉴权。

## 决策

1. 新增稳定 Provider ID `ollama`，Profile 的 `locality` 固定为 `local`，默认模型为 `qwen3:8b`。
2. 宿主只允许 `127.0.0.1:11434/v1/chat/completions`。WebView、项目文档和 Provider 设置均不能修改 scheme、host、port 或 path；不支持远程 Ollama 地址。
3. Ollama 不读取、写入或发送 Secret Store 凭据。配置继续携带固定 `secret://providers/ollama/default` 作为 v1 协议的无效占位，以避免本次扩展破坏已发布结构；宿主按 Provider ID 明确跳过解析。
4. Windows 传输仅在该本地路径使用明文 HTTP，并强制 `WINHTTP_ACCESS_TYPE_NO_PROXY`；四个云端 Provider 继续使用 TLS 与原有代理策略。Authorization header 只在实际存在宿主凭据时生成。
5. 请求沿用 OpenAI-compatible SSE 解析器，输出上限写入 `max_tokens`，JSON 产物使用 `response_format`。reasoning 的 disabled/enabled/effort 映射到官方 `reasoning_effort`，adaptive 且未指定 effort 时不注入供应商参数。
6. SQLite schema v4 扩展 `operation_run.provider_id` 约束。迁移在事务中重建父表、复制全部列、恢复 immutable/transition triggers，并在提交前执行 `foreign_key_check`；回归测试携带 lifecycle event、artifact 和 review 子记录完成 v3→v4 升级。
7. Context Compiler 以 Profile 的 `local` 属性为唯一依据执行数据策略，因此 canonical `never_send` 样本可以进入用户确认过的 Ollama Context Packet，云端行为不变。
8. 用户可显式探测已安装模型。Rust Host 使用同一固定回环、无代理边界 GET `/v1/models`，响应限制为 1 MiB/10,000 项，并校验、去重、排序 model ID 后才返回 WebView；探测不会触发模型生成或云端 fallback。

## 后果

- 用户无需云端账号即可完成 Context 编译、流式生成、Patch 审查、原子应用和审计持久化。
- 本地服务未运行、模型未拉取或模型名错误时，统一模型错误会安全返回界面，不会回退到云端。
- 固定回环边界牺牲了局域网/远程 Ollama 的灵活性，但消除了 SSRF、代理外发和文档注入目标地址的风险。
- 模型探测只列出当前 Ollama 已公开的模型；自动拉取、删除和更新模型仍由 Ollama 自身负责。

## 依据

- Ollama API introduction: https://docs.ollama.com/api/introduction
- Ollama OpenAI compatibility: https://docs.ollama.com/api/openai-compatibility
- Ollama streaming: https://docs.ollama.com/api/streaming
