# optimizer-desktop (src-tauri)

> 对应版本 v0.2.0

## 定位

Tauri 桌面后端（Rust）：将 `optimizer-host` crate 的能力封装为 Tauri `#[tauri::command]` 异步命令，向前端 `apps/optimizer-desktop` 暴露 IPC 接口，是桌面应用唯一的宿主适配层。

## 包结构

- `src/main.rs`：二进制入口，加载 Tauri 配置并启动应用。
- `src/lib.rs`：核心实现。包含 `DesktopState`（持有会话、最近项目注册表、可信端点注册表、密钥存储、模型执行主机）、`CommandError` / `CommandResult`、各命令的 `#[tauri::command] async fn` 包装，以及 `attach` 函数（向 Tauri builder 注册 `manage(state)` 与 `invoke_handler`）。
- `src/command_manifest.rs`：命令清单单一真理源（SSOT），导出 `REGISTERED_COMMANDS: &[&str]`。
- `capabilities/main-local.json`：Tauri capability 配置，限定命令仅允许在本地 `main` 窗口调用。
- `permissions/optimizer.toml`：命令权限授权文件，必须覆盖 `REGISTERED_COMMANDS` 中的每一个命令名。
- `tauri.conf.json`：Tauri 应用配置（CSP、窗口、capability 绑定）。
- `build.rs`：Tauri 构建脚本。

## 命令清单 SSOT

`src/command_manifest.rs` 中的 `REGISTERED_COMMANDS` 常量是命令清单的单一真理源。截至 v0.2.0 共注册 48 个命令，覆盖以下职责域：

- 项目会话：`create_project` / `open_project` / `close_project` / `get_project_session` / `get_project_workspace`。
- 最近项目：`list_recent_projects` / `open_recent_project` / `remove_recent_project`。
- 文档生命周期：`create_document` / `rename_document` / `reorder_document` / `change_document_depth` / `set_document_archived` / `list_archived_documents` / `export_markdown`。
- 摘要维护：`list_summary_invalidations` / `refresh_summaries` / `get_summary_context`。
- 风格样本：`list_style_samples` / `create_style_sample` / `set_style_sample_status`。
- 知识库：`list_knowledge_items` / `create_knowledge_item` / `set_knowledge_item_status` / `get_knowledge_context`。
- 操作上下文与块写入：`get_operation_context` / `save_block`。
- 审阅候选：`list_review_candidates` / `load_review_candidate` / `create_review_candidate_branch` / `apply_reviewed_proposal`。
- 版本与检查点：`get_version_history` / `create_checkpoint` / `restore_checkpoint`。
- 操作持久化与审计：`persist_operation_bundle` / `append_review_event` / `get_operation_audit`。
- 可信端点：`list_trusted_model_endpoints` / `register_trusted_model_endpoint` / `remove_trusted_model_endpoint`。
- 密钥管理：`store_provider_secret` / `has_provider_secret` / `delete_provider_secret`。
- 模型执行：`authorize_model_request` / `execute_authorized_model_stream` / `cancel_model_request` / `list_ollama_models` / `list_openai_compatible_models`。

`lib.rs::attach` 中的 `tauri::generate_handler![...]` 列表与 `REGISTERED_COMMANDS` 严格对应；`lib.rs` 内的 `tauri_security_configuration_is_local_and_covers_every_registered_command` 测试会断言 `permissions/optimizer.toml` 覆盖了 `REGISTERED_COMMANDS` 中的每一个命令名，且不包含被禁用的危险命令（如 `execute_model_stream`、`resolve_provider_secret`）。

## 新增命令流程

新增一个 Tauri 命令需按以下顺序操作，缺一不可：

1. 在 `src/command_manifest.rs` 的 `REGISTERED_COMMANDS` 数组中追加命令名（snake_case）。
2. 在 `src/lib.rs` 中实现对应的 `#[tauri::command] async fn` 函数。函数签名通常接受强类型请求结构体（实现 `Deserialize` 并带 `validate()` 校验 `schemaVersion`）与 `State<'_, DesktopState>`，通过 `spawn_host_task` 将同步逻辑投递到 `tauri::async_runtime::spawn_blocking`。
3. 在 `src/lib.rs` 的 `attach` 函数内的 `tauri::generate_handler![...]` 列表中追加该函数名。
4. 在 `permissions/optimizer.toml` 中为新命令授权（按既有命令的权限分组归位）。
5. 前端在 `apps/optimizer-desktop` 的 `main.js` 中通过 `invokeHost("command_name", payload)` 调用，payload 携带 `schemaVersion: 1` 与命令特定字段。

完成 1–4 步后，运行 `cargo test`，`tauri_security_configuration_is_local_and_covers_every_registered_command` 会自动验证权限覆盖完整性。

## 依赖关系

- `optimizer-host`（路径依赖 `../../../crates/optimizer-host`）：提供 `OpenedProject` / `ProjectWorkspace` / `ModelExecutionHost` / `SecretStore` / `TrustedModelEndpointRegistry` / `RecentProjectRegistry` 等核心能力；间接拉入 `optimizer-store` crate（含 `SNAPSHOT_SCHEMA_VERSION` 等存储常量）。
- `tauri` 2.11.2（feature `wry`）：IPC、窗口、状态管理。
- `serde` / `serde_json`：请求 / 响应序列化。
- `tauri-build` 2.6.2（build-dependency）：构建脚本支持。
- `sha2` 0.10.9（dev-dependency）：仅用于测试中的哈希构造。

## 安全约束

- 全部命令仅允许在本地 `main` 窗口执行（`capabilities/main-local.json` 中 `windows: ["main"]`，无 `remote` 字段）。
- 密钥类命令（`store_provider_secret` / `has_provider_secret` / `delete_provider_secret`）通过 `SecretStore` 抽象访问，明文密钥永远不会经 IPC 返回；`CommandError` 序列化结果会过滤 `apiKey` / `Authorization` 等敏感字段。
- 模型执行需先经 `authorize_model_request` 完成上下文包绑定与哈希校验，再通过 `execute_authorized_model_stream` 流式消费；明文密钥不进入 prompt。

## 开发命令

```pwsh
# 在 apps/optimizer-desktop/src-tauri 目录下
cargo build                # 编译桌面后端
cargo test                 # 运行 src/lib.rs 内嵌的全部单元测试
cargo test tauri_security_configuration_is_local_and_covers_every_registered_command
                           # 单独验证权限覆盖完整性
```

构建产物为 `optimizer-desktop` 二进制（`src/main.rs`），由 Tauri 在打包阶段嵌入桌面应用。
