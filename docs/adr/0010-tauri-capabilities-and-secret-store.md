# ADR-0010：Tauri 最小权限 IPC 与宿主 Secret Store

- 状态：Accepted
- 日期：2026-07-15

## 背景

Operation Host 已具有严格 DTO 和事务性 Store，但尚未连接桌面 WebView。直接将全部 Rust API 注册为 Tauri command 会扩大前端被攻陷后的影响范围；把 Keychain 读取结果返回 JavaScript，又会让 API Key 进入 DevTools、前端内存、异常上报或日志。

## 决策

新增 `apps/optimizer-desktop/src-tauri` 作为单独 Adapter crate。它不复制 Host/Store 规则，只负责：

- 将六个明确命名的异步 IPC command 注册到 `tauri::Builder`；
- 将阻塞 SQLite/Keychain 工作移入 `spawn_blocking`；
- 把内部错误映射为稳定的 `code + message`，SQLite 与 invariant 细节不返回 WebView；
- 用 AppManifest、permission TOML 和 capability JSON 将 command 限制到本地打包的 `main` window；
- 配置非空 CSP，且不配置任何 remote capability。

权限按用途分为 Operation write、Operation audit read 和 provider secret manage。构建脚本使用同一份 `REGISTERED_COMMANDS` 清单生成 Tauri command manifest；测试要求每个注册 command 都出现在显式 permission 中。

Tauri 2 的 permission/capability 选择遵循官方安全模型：permission 决定命令权限，capability 将权限授予特定 window/webview，Runtime Authority 在 IPC 到达 command 前执行检查。参考 [Tauri Permissions](https://v2.tauri.app/security/permissions/) 与 [Tauri Capabilities](https://v2.tauri.app/security/capabilities/)。

## Secret Store

`optimizer-host` 定义与平台无关的 `SecretStore`：

- 持久化配置只保存 `secret://` 引用；
- `SecretValue` 不实现 Clone，Debug 始终显示 `[REDACTED]`，Drop 时清零内存；
- WebView 只能 store、has、delete，不能 resolve/read 明文；
- 当前 Windows 后端使用 Credential Manager Generic Credential，并使用独立 `OptimizerSystem` namespace；
- 自动化测试使用清零的内存实现，不读写用户真实凭据库。

Windows Generic Credential blob 上限为 2,560 字节，因此所有后端统一先执行这一保守限制，保持跨平台行为确定。

## 安全边界

```text
bundled main WebView
  │ capability: explicit local-only commands
  ▼
Tauri async command
  │ structured DTO/error; spawn_blocking
  ▼
DesktopState
  ├── OperationCommandHost → SQLite
  └── SecretStore → Windows Credential Manager
                     ▲
                     └── plaintext never returns to WebView
```

模型网关当前仍是 TypeScript 实现，因此本阶段只管理 Secret，不提供前端读取命令。后续应将模型 HTTP 执行移入 Rust 信任边界，或提供一次性宿主请求命令，由宿主内部解析 `credentialRef`；不得为了复用现有 TypeScript Gateway 而新增 `resolve_provider_secret` IPC。

## 取舍与后续

当前 crate 是可编译、可挂载的 Tauri Adapter，尚未建立项目打开/创建 Session 和正式编辑器入口。构建脚本在 `OUT_DIR` 生成开发占位图标，发布前必须替换为正式品牌资源。macOS Keychain 与 Linux Secret Service 后端将在对应平台 CI 可用时实现，并复用同一端口与契约测试。
