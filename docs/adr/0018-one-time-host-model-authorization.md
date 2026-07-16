# ADR 0018：一次性宿主模型请求授权

- 状态：Accepted
- 日期：2026-07-16
- 决策范围：桌面模型执行 IPC 与项目一致性

## 背景

模型 HTTP、固定供应商端点和 Secret Store 已位于 Rust 信任边界，但此前本地 `main` WebView 可以直接调用 `execute_model_stream` 并提交完整消息。正常界面会先编译并展示 Context Packet，然而一旦页面代码被篡改，原始执行命令本身无法证明请求对应用户确认的 Packet，也无法阻止确认后项目或目标 Block 已发生变化的请求继续出站。

这次迭代先收紧执行能力与时序，不在同一个变更中重写 TypeScript Context Compiler。

## 决策

1. 从 Tauri 注册清单和 `allow-model-execution` permission 中移除原始 `execute_model_stream`。WebView 不再拥有提交请求并立即联网的单步能力。
2. 增加两阶段 IPC：`authorize_model_request` 校验请求和项目绑定并签发 capability；`execute_authorized_model_stream` 只接受 capability ID 与事件 Channel。
3. 授权绑定当前 `projectId`、项目 `baseCommitId/HEAD`、Operation Intent ID、Context Packet ID/hash、目标 Block ID/revision/hash，以及 Provider locality。宿主重新读取当前工作区，校验项目、HEAD、Block 基线和 `ollama=local / cloud=remote` 映射。
4. `ModelExecutionHost` 在授权时完整执行固定端点、Provider 配置、消息和请求大小校验。通过后仅在宿主内保存原始请求，不把请求内容重新回传页面。
5. capability 使用随机 128-bit UUID，默认 120 秒失效；进程内最多保留 32 个待执行授权。同一 request ID 不能同时处于待执行或活动状态。
6. 执行时在联网前原子移除 capability，再校验当前项目与 HEAD。成功、过期、项目变化、凭据缺失或网络失败都不能重放同一个授权。
7. `cancel_model_request` 同时取消活动请求或撤销待执行授权；关闭项目先取得 Session 锁，再取消活动请求并清空全部待执行 capability。执行命令保持 Session 锁，直到请求已注册为可取消的活动请求，从而消除“消费授权后、活动注册前”的关闭竞态。
8. 授权、过期、stale 与重放错误继续走脱敏 `CommandError`，不包含 Prompt、Context、请求头或 Secret。

## 安全边界

本 ADR 阻止确认后的请求替换、重复执行、跨项目复用与旧 HEAD 请求出站，并消除公开的原始执行 IPC。它不是完整的宿主 Context policy compiler：已被攻陷的 WebView 仍可能在申请授权前伪造 Packet 内容或 hash。下一阶段必须让宿主从权威工作区和受控操作协议重新解析 Context 来源、验证 Packet hash，或直接在 Rust 中完成 Context 编译，才能对 compromised WebView 强制保证 `never_send`。

## 测试要求

- Rust Host 覆盖单次消费、重复 request ID、过期、取消、stale scope 和项目关闭清理；
- Tauri Adapter 覆盖无项目拒绝、HEAD/Block/locality 绑定、授权后项目变化和 raw command 未注册；
- 前端测试断言用户确认发生在授权之前，且授权发生在流式执行之前；
- 浏览器回归继续覆盖 Context 预览、生成、审查与应用闭环。

## 后果

- 正常 Operation 只增加一次本地 IPC 往返，不增加云端调用或 token；
- 项目在预览确认后发生保存时，本次生成会以 `MODEL_AUTHORIZATION_STALE` 安全失败，用户需基于新 HEAD 重新编译；
- 待执行授权只存在内存，应用重启后自然失效；
- raw `ModelExecutionHost::execute_stream` 仍作为 Rust 内部执行原语和测试接口存在，但不注册为 Tauri command。
