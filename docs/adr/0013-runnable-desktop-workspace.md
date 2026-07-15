# ADR-0013：零依赖可运行桌面工作区与编辑器适配边界

- 状态：Accepted
- 日期：2026-07-15

## 背景

Host 已提供项目会话、工作区、乐观保存、版本历史和检查点命令，但仓库还没有能实际启动和使用这些能力的桌面入口。当前构建环境无法可靠访问 npm registry，若把 Tiptap 作为启动前置条件，会让已经完成的版本化正文闭环继续停留在不可运行状态。

桌面端还必须确保 WebView 不获得数据库、文件系统或 Secret 明文权限，并正确处理 debounce 期间的连续输入、并发提交和恢复操作。Windows WebView2 默认数据目录在受限开发环境中也可能不可写，导致代码通过编译但进程在窗口初始化时退出。

## 决策

交付一个没有第三方 npm 运行时依赖的 Tauri 2 前端：

1. 欢迎页调用 `get_project_session`，并提供项目创建与打开表单；Host 是路径、manifest 和 SQLite 校验的最终权威。
2. 工作区呈现文档树、Block 列表、字符统计、保存状态、检查点和版本抽屉。正文输入暂用 `contenteditable="plaintext-only"`。
3. `frontend-state.js` 隔离排序、选中、Tiptap-compatible JSON 生成、保存请求和权威响应合并。UI 不提交 Commit ID、时间戳、项目 revision 或新 hash。
4. 每个 Block 使用 900ms debounce，并保证同一 Block 的写入串行。保存期间产生的新输入会在当前请求结束后继续保存。
5. Host 返回 `CONFLICT` 时，前端保留本地纯文本草稿、重新加载权威工作区，并让用户明确选择重试或放弃。未处理冲突阻止检查点和恢复；关闭项目允许显式确认。
6. 静态构建只复制已审查的本地 HTML/CSS/JS，并拒绝任何 `http://` 或 `https://` 资源引用。CSP 只允许本地资源、data/asset 图片和 Tauri IPC。
7. main window 配置为 `create=false`，由 Rust 使用 `WebviewWindowBuilder::from_config` 显式创建并指定数据目录。正式构建使用系统应用数据目录；调试构建使用 `target` 内目录；自动化可用绝对路径 `OPTIMIZER_WEBVIEW_DATA_DIR` 覆盖。

## Tiptap 边界

本决策不是放弃 Tiptap。当前 Block 内容仍生成 Tiptap-compatible JSON，协议、Store 和 editor-bridge 不依赖 DOM 编辑实现。依赖源可用后，将 `blockEditor` 输入面替换为 Tiptap，并把状态转换继续留在适配层；Host IPC、乐观并发、快照和审计模型不变。

在 Tiptap 接入前，当前输入面只承诺 paragraph、heading、quote 和 dialogue 的纯文本编辑，不承诺富文本 marks、复杂节点、协同光标或逐 hunk 行内装饰。

## 验证

- Node 测试覆盖文档/Block 稳定排序、版本化保存请求、权威响应合并、错误归一化与字符统计；
- Tauri 测试继续覆盖权限清单、项目 Session、工作区、保存、检查点、恢复和 Secret 不回传；
- `desktop:build` 验证四个入口文件非空且不引用远程资源；
- Rust binary `cargo check` 和 Clippy 覆盖真实 Wry 入口；
- 本地静态视觉冒烟验证中文、表单、布局和无控制台错误。

## 代价与后续

原生 `contenteditable` 无法提供成熟富文本编辑器的 selection mapping、schema enforcement 和 decoration 能力，因此不能直接承担 Patch 审查 UI。下一阶段优先把模型网络执行迁入 Rust 信任边界并接通桌面 Operation；Tiptap 依赖可用后再替换输入适配器，然后实现逐 hunk 行内审查。

调试 WebView 数据目录随 `target` 清理，不承诺保存浏览器本地状态；项目正文和版本数据始终位于 `.optimizer` 包，不依赖 WebView cache。
