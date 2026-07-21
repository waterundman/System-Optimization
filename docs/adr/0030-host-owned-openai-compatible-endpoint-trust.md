# ADR 0030：宿主管理的 OpenAI-compatible 端点信任

- 状态：Accepted
- 日期：2026-07-21
- 决策范围：FR-09 通用 OpenAI-compatible Provider、端点信任、凭据绑定与能力声明

## 背景

固定官方端点适合 DeepSeek、Qwen、Kimi、MiniMax，但不能覆盖企业代理、私有部署和其他兼容 Chat Completions 的服务。允许 WebView 在每次模型请求中提交任意 URL 会扩大 SSRF、凭据转发、重定向和审计漂移风险；只把 URL 存入普通 Provider 配置也无法证明本次运行使用的是用户曾明确确认的目标。

OpenAI-compatible 只描述协议家族，不保证每个服务都支持 `json_object`、流式 usage、reasoning、tool calls 或相同的输出 token 字段。一次 `/models` 请求也不能诚实证明这些语义能力。

## 决策

1. 通用 Provider ID 为 `openai_compatible`。WebView 不获得通用网络命令，也不能在模型授权或执行请求中提交 URL、Header、API Key 或任意 credential reference。
2. Host 维护独立的 `trusted-model-endpoints.json` 注册表。注册输入拆分为 label、hostname、base path、逐字确认的 HTTPS origin 和显式能力声明；Host 生成不可猜测的 `endpoint-{uuid}`、规范化 base URL 与专属 credential reference。
3. hostname 必须是至少两段的公共 ASCII DNS 名称。IP literal、单标签名称及 localhost/local/internal/home/lan/localdomain/test/example/invalid/onion/arpa/corp 后缀被拒绝。端口固定为 HTTPS 443；base path 禁止凭据、转义、query、fragment、反斜杠、空白、`.` 和 `..` 段。
4. 用户必须逐字确认 `https://{canonical-hostname}`。确认值不匹配时不登记端点。重复 location、超过 32 个条目、未知字段、非规范派生字段和被篡改注册表均失败关闭。
5. 每个端点绑定唯一的 `secret://providers/openai-compatible/{endpointId}`。Secret 命令只接受四个内置云端固定槽位或当前注册表中存在的精确端点槽位；删除信任时先删除该槽位凭据，再删除注册表记录。注册表不保存凭据。
6. `ModelProviderConfiguration.openaiCompatible` 只携带 endpoint ID 与端点能力快照。Host 在授权和真正执行前都按 ID 重取注册表，并要求 credential reference、能力声明和派生路径与注册表完全一致；页面不能通过篡改配置把已授权请求转向另一个地址。
7. 通用端点只使用固定的 `/chat/completions` 和 `/models` 后缀，禁用系统代理和 HTTP 重定向；TLS 证书与 hostname 仍由系统传输层验证。所有其他模型请求也统一禁止重定向，避免 Authorization 被转发到第二目标。
8. 默认能力是保守的：`jsonObject=false`、`streamUsage=false`、`maxOutputTokenField=max_tokens`，reasoning 与 tool calls 始终关闭。用户可以在建立信任时显式声明前述三个兼容差异；未声明能力对应字段必须完全省略。
9. `/models` 探测是显式操作，使用该端点自己的凭据、3 秒超时与 1 MiB 响应上限，不发送项目、Context 或 Prompt。它只证明当时的可达性和模型列表响应可解析，不自动声称已验证 JSON、usage、reasoning 或其他语义能力。
10. schema v9 在 `operation_run` 增加 `provider_configuration_id` 与 `provider_endpoint_id`。通用 Provider 的成功或失败运行都必须保存二者；其他 Provider 不得伪造 endpoint ID。端点 ID 因而进入不可变 Operation 审计，但 URL 和凭据不会复制进项目数据库。

## 结果

- 用户可以登记多个 HTTPS OpenAI-compatible 服务，分别保存凭据、选择模型并显式声明兼容差异。
- 项目内容只能发送到 Host 注册并由用户确认过的目标；一次性 capability、当前 Context 复算和 endpoint ID 解析形成连续信任链。
- 注册表是设备级配置，Operation 只保存稳定来源 ID。删除端点不会修改历史审计，但该端点不能再授权新请求。
- 自定义端点故障不会自动回退内置云端或另一个端点；重试仍遵循 ADR 0029 的响应前硬上限。
- `/models` 不是完整能力协商。错误的人工能力声明可能导致 Provider 返回协议/参数错误，但不能改变目的地、取得其他端点凭据或绕过 Context 确认。
- DNS 解析和系统根证书仍属于操作系统信任边界；本阶段不支持自定义 CA、任意端口、HTTP、企业系统代理或私网 IP literal。

## 测试要求

- 注册表覆盖规范化、持久化、备份恢复、重复条目、篡改、数量/大小上限、保留后缀、IP literal、路径逃逸和确认不匹配；
- Secret IPC 覆盖任意引用拒绝、端点专属槽位写入/查询/删除，以及删除信任时删除凭据；
- Host 网关覆盖 endpoint ID 解析、配置快照篡改失败、专属 secret、固定 path、禁用代理、保守 dialect、响应上限和模型列表排序；
- TypeScript Profile、协议 Schema、桌面配置和 Operation Store 覆盖同一字段集合；
- schema v9 迁移保留旧运行，并拒绝没有配置/端点来源的通用运行或带伪造 endpoint ID 的内置运行；
- Tauri command manifest、capability 和 permission 必须保持完全一致，WebView CSP 继续禁止网络访问。

## 回滚

可从界面移除所有通用端点并隐藏 `openai_compatible` Provider，保留 schema v9 的只读审计字段和注册表文件。数据库不得手工降级；如必须回退二进制，应从迁移前在线备份恢复项目。内置官方 Provider 与固定回环 Ollama 不依赖通用端点注册表。
