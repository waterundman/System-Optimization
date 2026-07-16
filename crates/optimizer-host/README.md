# optimizer-host

Rust 宿主安全边界与跨语言命令适配层。

当前实现：

- `ProjectRoot` 校验项目相对路径，拒绝绝对路径、`..` 与目录逃逸；
- `OpenedProject::create/open` 原子创建并严格校验 `.optimizer` 项目包；
- `RecentProjectRegistry` 在宿主用户配置区严格解析、去重并崩溃安全地替换最近项目元数据；
- `OperationCommandHost::persist_operation_bundle_json` 严格接收 v1 Operation bundle，并在一个 Store 事务中持久化；
- `OperationCommandHost::append_review_event_json` 使用 expected revision/status 写入审查事件；
- `OperationCommandHost::get_operation_audit` 聚合运行、ContextPacket、生命周期、Artifact 与 Patch review 历史；
- 所有输入 DTO 使用 `deny_unknown_fields`，协议版本不匹配或嵌套未知字段会在写库前失败；
- 开放 JSON payload 会递归拒绝 credential、API key、Authorization 和原始 Header 字段；
- TypeScript 与 Rust 读取同一份 Operation 持久化夹具，避免边界字段漂移；
- `SecretStore` 只接受 `secret://` 引用；`SecretValue` Debug 固定脱敏并在释放时清零；
- Windows 使用 Credential Manager Generic Credential，Secret 不写入 SQLite、配置文件或日志。
- `ModelExecutionHost` 通过固定官方 HTTPS 端点支持 DeepSeek、Qwen、Kimi、MiniMax，统一流事件、用量、取消、超时和安全错误；
- Windows 原生传输使用 WinHTTP，活动 request handle 可由取消/超时关闭，Authorization 临时缓冲发送后清零；
- `apply_reviewed_proposal` 重验 Proposal hash、UTF-16 anchor、目标基线、hunk 与审查决策，并原子写入正文和 Operation/Review 终态。

## `.optimizer` 项目包

`OpenedProject::create/open` 是桌面端项目生命周期的唯一入口。项目包使用固定的 `manifest.json + project.sqlite3 + assets/backups/exports` 结构；创建时在目标父目录的隐藏 staging 目录完成初始化与校验，再原子 rename 发布。打开时严格校验 manifest 大小/schema、数据库 invariant、项目 ID 和主分支/HEAD 绑定。

`ProjectInfo` 对上层暴露 `projectId`、`mainBranchId`、HEAD、revision 与数据库 schema，为下一阶段的文档加载、乐观并发自动保存和版本提交提供稳定基线。

## 最近项目

最近项目注册表不存入任一项目包，也不作为项目真实性来源。它采用固定 schema、256 KiB 上限、最多 12 项、同目录临时文件、旧文件备份与发布失败回滚。记录只包含项目 ID、标题、语言、绝对目录和最后打开时间；列表可标记缺失路径。快速打开从注册表解析目录后仍调用 `OpenedProject::open`，并再次比对登记项目 ID。WebView 不能修改目录映射；移除记录不删除项目文件。

## 工作区命令

`OpenedProject` 现在提供树形结构化工作区读取、顶层/子章节创建、同级移动、缩进/移出、子树归档、单 Block 乐观保存、检查点创建、版本历史和检查点恢复。Host 生成持久化 ID/时间戳与内容/根哈希；Store 同时校验 Block revision/hash、Document revision 和项目 revision/HEAD，并在一个事务中更新正文、结构、编辑日志、Commit DAG 与分支/项目 HEAD。恢复始终形成新 Commit。

Tauri Adapter 已在 `apps/optimizer-desktop` 注册最小权限 commands/capabilities。Windows Provider 网络执行已经完全位于宿主；macOS Keychain/Linux Secret Service 与对应原生 HTTP 后端、宿主 Operation capability 和 WASM 插件资源限制仍属于后续能力。

任何文件命令都必须先经过 `ProjectRoot`，不得接受未经校验的绝对路径或 `..`。宿主命令不得接收 API Key 或原始 HTTP Header，只接收协议允许的审计数据。
