# optimizer-host

Rust 宿主层的最小安全基线。当前只实现项目根目录边界，后续在此加入：

- Tauri command 与 capability 校验；
- SQLite 连接、迁移、备份和 integrity check；
- OS Keychain；
- Provider 网络请求与域名策略；
- WASM 插件资源限制。

任何文件命令都必须先经过 `ProjectRoot`，不得接受未经校验的绝对路径或 `..`。

