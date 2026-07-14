# 模型 Provider 官方兼容性核对

- 核对日期：2026-07-14
- 范围：文本优化器使用的 OpenAI-compatible Chat Completions

| Provider | 官方 base URL / 端点 | 本轮验证模型 | 关键差异 |
|---|---|---|---|
| DeepSeek | `https://api.deepseek.com/chat/completions` | `deepseek-v4-flash`、`deepseek-v4-pro` | `thinking`、`reasoning_effort`、SSE usage |
| Qwen | 地域工作区域名 + `/compatible-mode/v1/chat/completions` | `qwen-plus`、`qwen3.7-plus` | 地域域名、`enable_thinking`、`max_completion_tokens` |
| Kimi | `https://api.moonshot.cn/v1/chat/completions` | `kimi-k2.6`、`kimi-k2.7-code` | `thinking.keep`、Preserved Thinking、SSE |
| MiniMax | `https://api.minimaxi.com/v1/chat/completions` | `MiniMax-M3`、M2.7/M2.5 | `reasoning_split`、累计流片段、`max_completion_tokens` |

## 官方资料

- DeepSeek：https://api-docs.deepseek.com/
- Qwen：https://help.aliyun.com/en/model-studio/qwen-api-via-openai-chat-completions
- Kimi：https://platform.kimi.com/docs/api/chat
- MiniMax：https://platform.minimaxi.com/docs/api-reference/text-openai-api

DeepSeek 的旧 `deepseek-chat`、`deepseek-reasoner` 名称将在 2026-07-24 停用，因此没有作为默认或已知模型写入新 profile。

Qwen 中国和新加坡旧域名仍可用，但官方推荐工作区专属域名；代码在没有 workspace ID 时保留旧域名作为兼容回退，美国端点使用官方共享域名，德国和日本必须提供 workspace ID。
