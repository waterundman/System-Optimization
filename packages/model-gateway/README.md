# @optimizer/model-gateway

无第三方运行时依赖的 OpenAI-compatible 模型网关。当前内置 DeepSeek、Qwen、Kimi、MiniMax 官方 API profile、固定回环地址的 Ollama profile，以及只能从 Host 可信端点记录构造的通用 profile。

## 已实现

- 非流式 Chat Completions；
- SSE 流式文本、reasoning、tool-call delta 与 usage；
- 外部取消、超时和流消费者提前退出；
- 鉴权、配额、限流、无效请求、服务端、网络和协议错误归一化；
- API Key 在公开消息、远端错误码和 request ID 中精确脱敏，并限制请求体与错误审计字段大小；
- DeepSeek/Kimi/Qwen/MiniMax/Ollama thinking 参数映射；
- Qwen 中国、新加坡、美国、德国、日本地域端点；
- 通用端点的保守 capability 映射：未声明的 JSON、流式 usage、reasoning 与 tool calls 不会发送；
- Provider Router 动态切换。

每个 Provider Profile 同时声明 `locality`。四个官方云端 profile 与通用 profile 固定为 `remote`；Ollama 固定为 `local`，供 Context Compiler 和 Operation Runner 执行 `never_send` 隐私策略。

网关不直接读取环境变量。宿主解析 secret 后注入，避免领域包访问进程环境：

```ts
const deepseek = new OpenAICompatibleModelGateway({
  profile: deepSeekProfile,
  apiKey: secretStore.get("DEEPSEEK_API_KEY"),
});

const router = new ModelProviderRouter([deepseek]);
const result = await router.complete("deepseek", {
  messages: [{ role: "user", content: "润色这段文字" }],
  maxOutputTokens: 1000,
});
```

持久化配置只保存 `ModelProviderConfiguration.credentialRef`（例如
`secret://providers/deepseek/default`），由宿主 Secret Store 在创建网关时解析；协议校验器会拒绝
`apiKey` 等未知字段，防止凭据误入 SQLite、事件日志或导出文件。

Qwen 推荐使用工作区专属域名：

```ts
const profile = createQwenProfile({
  region: "china",
  workspaceId: "your_workspace_id",
});
```

通用 profile 不接收自由输入的 URL。桌面 Host 先登记并确认 HTTPS 端点，再把只读记录交给适配层：

```ts
const profile = createOpenAICompatibleProfile(trustedEndpoint, "acme-writer-v1");
```

这里的 `trustedEndpoint` 必须来自 Host 注册表；TypeScript 工厂只执行形状和保守 dialect 映射，不替代 Host 的 DNS/path/origin 信任校验。真正出站时 Rust Host 会按 endpoint ID 再次解析目标和专属凭据。

## 内置预设

| Provider | 默认模型 | API Key 建议名称 |
|---|---|---|
| DeepSeek | `deepseek-v4-flash` | `DEEPSEEK_API_KEY` |
| Qwen | `qwen-plus` | `DASHSCOPE_API_KEY` |
| Kimi | `kimi-k2.6` | `MOONSHOT_API_KEY` |
| MiniMax | `MiniMax-M3` | `MINIMAX_API_KEY` |
| OpenAI-compatible | 用户选择 | Host 生成的端点专属槽位 |
| Ollama | `qwen3:8b` | 无；固定 `127.0.0.1:11434` |

模型名称只是经过验证的预设，不是白名单；调用方可以传入同一端点当前支持的其他模型。

## 重试策略

网关不会自动重试 Chat Completions POST。失败会返回 `retriable`、`retryAfterMs` 和经过限长/脱敏的错误元数据，由 Operation Runner 在考虑费用、用户取消和响应是否已经开始后决定是否重试。当前 Runner 仅允许响应开始前的有界重试；每次尝试仍是一次独立 Provider 调用。

MiniMax 的 `json_object` 能力尚未在当前官方 OpenAI SDK 页面确认，因此 profile 默认拒绝该参数。可继续用明确的文本输出协议，待官方能力确认后再开放。

通用 profile 永远不假设 reasoning 或 tool-call 能力；`json_object`、流式 usage 和输出 token 字段只能来自用户建立端点信任时的显式声明。`/models` 探测只检查可达性和列表格式，不能自动证明这些语义能力。

Ollama profile 只接受 `http://127.0.0.1:11434/v1`。这个 HTTP 例外按 Provider ID、locality、host、port 和 path 同时校验，不能用来访问其他本地或远程地址。
