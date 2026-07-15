# @optimizer/model-gateway

无第三方运行时依赖的 OpenAI-compatible 模型网关。当前内置 DeepSeek、Qwen、Kimi、MiniMax 官方 API profile，以及固定回环地址的 Ollama profile。

## 已实现

- 非流式 Chat Completions；
- SSE 流式文本、reasoning、tool-call delta 与 usage；
- 外部取消、超时和流消费者提前退出；
- 鉴权、配额、限流、无效请求、服务端、网络和协议错误归一化；
- API Key 精确脱敏和请求体字节上限；
- DeepSeek/Kimi/Qwen/MiniMax/Ollama thinking 参数映射；
- Qwen 中国、新加坡、美国、德国、日本地域端点；
- Provider Router 动态切换。

每个 Provider Profile 同时声明 `locality`。四个云端 profile 固定为 `remote`；Ollama 固定为 `local`，供 Context Compiler 和 Operation Runner 执行 `never_send` 隐私策略。

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

## 内置预设

| Provider | 默认模型 | API Key 建议名称 |
|---|---|---|
| DeepSeek | `deepseek-v4-flash` | `DEEPSEEK_API_KEY` |
| Qwen | `qwen-plus` | `DASHSCOPE_API_KEY` |
| Kimi | `kimi-k2.6` | `MOONSHOT_API_KEY` |
| MiniMax | `MiniMax-M3` | `MINIMAX_API_KEY` |
| Ollama | `qwen3:8b` | 无；固定 `127.0.0.1:11434` |

模型名称只是经过验证的预设，不是白名单；调用方可以传入同一端点当前支持的其他模型。

## 重试策略

网关不会自动重试 Chat Completions POST。失败会返回 `retriable` 和 `retryAfterMs`，由 Operation 调度层在考虑费用、用户取消和是否已产生输出后决定是否重试。

MiniMax 的 `json_object` 能力尚未在当前官方 OpenAI SDK 页面确认，因此 profile 默认拒绝该参数。可继续用明确的文本输出协议，待官方能力确认后再开放。

Ollama profile 只接受 `http://127.0.0.1:11434/v1`。这个 HTTP 例外按 Provider ID、locality、host、port 和 path 同时校验，不能用来访问其他本地或远程地址。
