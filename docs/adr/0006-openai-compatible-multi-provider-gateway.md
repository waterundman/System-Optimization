# ADR-0006：共享 OpenAI-compatible 多模型网关

- 状态：Accepted
- 日期：2026-07-14

## 决策

DeepSeek、Qwen、Kimi、MiniMax 通过一个无厂商 SDK 依赖的 Chat Completions HTTP 内核接入。厂商差异保存在不可变 `ProviderProfile` 与 request dialect 中，上层只使用 `ModelProvider`、`ModelRequest`、`ModelCompletion` 和 `ModelStreamEvent`。

每个 profile 固定官方 base URL、建议环境变量、默认模型、已知模型、输出 token 字段、流片段模式、能力声明、文档地址和验证日期。Qwen 通过工厂函数生成地域/工作区域名，workspace ID 只能包含安全字符。

## 厂商差异

- DeepSeek：`thinking.type`、`reasoning_effort`、`max_tokens`。
- Qwen：`enable_thinking`、`preserve_thinking`、`max_completion_tokens`。
- Kimi：`thinking.type/keep`、`max_tokens`；`kimi-k2.7-code` 禁止关闭 thinking。
- MiniMax：`thinking.type`、`reasoning_split=true`、`max_completion_tokens`，并归一化累计式 reasoning/content。

## 安全边界

- 核心包不读取 `process.env`，API Key 由宿主 secret store 注入。
- API Key 只进入 Authorization 头，不进入请求 JSON、事件或遥测。
- 远端错误消息最多保留有限长度，并精确替换已知 secret。
- base URL 必须是无凭据、query 和 fragment 的 HTTPS URL。
- 请求体和 SSE 单事件具有硬上限。
- 流消费者退出时取消 reader；超时和用户取消分别分类。

## 错误与重试

HTTP/网络错误归一为 authentication、permission、quota、rate_limit、invalid_request、server、network、timeout、cancelled、protocol 和 configuration。

网关不自动重试计费型 POST。429/408/5xx 标记 `retriable`，并解析 `Retry-After`；是否重试由更高层 Operation 调度器结合输出进度和用户意图决定。

## 取舍

直接使用 Fetch/SSE 需要维护响应校验，但可以避免同时引入四套 SDK，统一取消、隐私、错误和测试语义。当前测试使用模拟 HTTP，不发送真实凭据或产生 API 费用；带密钥的供应商冒烟测试应由独立 opt-in 集成测试执行。
