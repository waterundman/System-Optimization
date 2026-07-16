# Optimizer Text 工程基线完成度审计

- 基线：`Optimizer_Kernel_文本优化器工程设计文档_v1.0.docx`
- 审计日期：2026-07-16
- 审计目标：以设计文档中的 MVP MUST/SHOULD 和第 25 章验收清单为准，不以已有实现反向缩小范围
- 状态定义：`完成` 表示当前代码和测试有直接证据；`部分` 表示主链已存在但验收项不完整；`缺失` 表示没有可运行实现

## 功能需求

| ID | 状态 | 当前证据 | 缺口 / 完成条件 |
|---|---|---|---|
| FR-01 项目/文档/章节/场景/Block 创建编辑重排导出 | 部分 | `.optimizer` 项目包、版本化章节树、Block 自动保存、Markdown 导入导出 | 缺 JSON 可读导出；Block 类型/多 Block 创建仍是最小实现 |
| FR-02 自动保存、操作前提交、AI 接受提交、版本恢复 | 完成 | `workspace_commands`、快照/Commit DAG、操作前 `flushAll`、`ai_accept` 原子事务 | 继续补充真实崩溃故障注入，但主验收闭环已存在 |
| FR-03 五种内置操作 | 完成 | 桌面续写/润色/压缩/扩写/批评，Operation Profile、严格产物解析 | 多 Provider contract 与浏览器完整流程已覆盖 |
| FR-04 选区菜单、右键、快捷键、命令面板 | 部分 | 编辑器工具栏与选区/光标目标 | 缺右键菜单、键盘快捷键、命令面板；自动触发目前未启用 |
| FR-05 Context 编译、预算、来源展示、敏感预检 | 部分 | Kernel Context Compiler、发送确认、`never_send`、Host summary source | 局部正文与风格仍由 WebView 组装；完整 Packet hash/资格策略尚未全部下沉 Host |
| FR-06 流式、取消、超时、有限重试、结构化校验 | 部分 | Rust 固定端点流式传输、取消/超时、严格 JSON 输出 | 缺显式有限重试策略、幂等重试 UI 与重试审计 |
| FR-07 差异、逐项/整段接受拒绝、保留候选、冲突 | 部分 | 中文分层 diff、逐 hunk 决策、原子应用、冲突拒绝、不可变候选审计 | 缺“全部接受/全部拒绝”的批量交互和候选分支入口 |
| FR-08 事实、约束、风格、摘要及 canonical 状态 | 完成 | schema v6 事实/约束库、authority/sensitivity/severity、canonical/archived/rejected、目标绑定 L3 Context；风格样本与分层摘要 | 自动事实抽取属于后续增强，不是 MVP 必需项 |
| FR-09 OpenAI-compatible 与 Ollama | 部分 | DeepSeek、Qwen、Kimi、MiniMax 固定官方端点；Ollama 固定回环与模型发现 | 缺通用 OpenAI-compatible 手工配置/能力探测；当前安全模型故意不接受任意 URL |
| FR-10 日志、token/费用、模型、来源、反馈 | 部分 | Operation/Context/usage/lifecycle/Artifact/Review 本地审计 | 缺费用换算、接受率/二次编辑率聚合、用户反馈与诊断导出 |
| FR-11 备份、迁移备份、完整性与恢复向导 | 部分 | 迁移前在线备份、快照 checksum、Store invariant | 缺用户可见项目备份和损坏恢复向导、恢复演练入口 |
| FR-12 停顿/段落补全 | 缺失（MAY） | 默认不启用，符合非目标 | P1 实验项，不阻塞 MVP MUST |

## 非功能与工程验收

| 领域 | 状态 | 证据 / 缺口 |
|---|---|---|
| 事务与静默覆盖保护 | 完成 | Block/Commit/Review/Operation 原子事务、乐观并发、快照恢复与浏览器回归 |
| Kernel 隔离 | 完成 | 架构检查阻止 Kernel 依赖 UI/Tauri/SQLite/Provider SDK |
| 密钥隔离 | 完成（Windows） | Credential Manager、opaque ref、WebView 无明文读取、日志脱敏测试；其他桌面平台尚未实现 |
| 离线编辑/版本/导出/本地摘要 | 完成 | 所有本地能力无 npm 运行时依赖；摘要默认零网络 |
| 可访问性 | 部分 | 语义按钮/aria-label/键盘可聚焦；缺完整快捷键、焦点管理和屏幕阅读器验收 |
| 国际化 | 部分 | Unicode/语言标签/中文 diff；界面字符串尚未资源化 |
| 性能预算 | 缺失证据 | 无 10 万字热启动、Context 编译、补丁落盘基准与阈值门禁 |
| 插件最小权限与跨宿主复用 | 缺失 | 有 Ports/Adapters 协议，但无 Obsidian contract adapter、WASM 插件 runtime/manifest/权限 |
| 发布与供应链 | 缺失 | 无正式安装包签名、SBOM、依赖审计门禁、分阶段更新和真实迁移矩阵 |
| Beta 指标 | 缺失 | 无 20—50 用户数据；产品阈值不能由单机自动测试证明 |

## 当前执行顺序

1. `P0 / FR-05`：继续把正文/结构/风格 Context 资格与 Packet 复算下沉 Host。
2. `P0 / FR-04 + FR-07`：右键、快捷键、命令面板，以及全部接受/拒绝交互。
3. `P0 / FR-06`：有限重试、幂等审计和重试 UI。
4. `P0 / FR-09`：在固定端点安全原则下设计通用 OpenAI-compatible 配置与白名单策略。
5. `P1`：JSON 导出、项目备份/恢复向导、性能基准、可访问性和国际化。
6. `M5`：插件/Obsidian contract、安装签名、SBOM、依赖审计与更新。

每一轮实现后必须更新本文件状态与直接证据；只有所有 MVP MUST 和第 25 章工程验收均有可复核证据时，才可以声明完整目标完成。Beta 用户指标需要真实外部数据，不能用测试替代。
