# ADR-0011：原子项目包与单项目桌面会话

- 状态：Accepted
- 日期：2026-07-15

## 背景

此前 Tauri Adapter 在构造时直接持有一个已经打开的 `OperationCommandHost`，因此无法表达“尚未打开项目”、创建新项目、关闭项目或切换项目。SQLite 文件也缺少稳定的外层项目包协议，桌面端若自行拼接数据库路径和初始记录，会绕过 Host 的路径边界、初始化事务和完整性检查。

自动保存和版本提交还需要一个可稳定引用的主分支。仅返回项目 HEAD 不足以执行后续编辑，因为写入命令必须同时绑定分支、预期 revision 与预期 hash。

## 决策

采用目录型 `.optimizer` 项目包，并由 `optimizer-host` 独占创建和打开逻辑。v1 包结构为：

```text
MyNovel.optimizer/
├── manifest.json
├── project.sqlite3
├── assets/
├── backups/
└── exports/
```

`manifest.json` 是严格、版本化且有 64 KiB 上限的绑定清单，包含 `projectId`、`mainBranchId`、标题、语言、数据库固定文件名与创建时间。未知字段、未知 schema、缺失文件、数据库不变量失败、项目 ID 不匹配，或主分支不属于该项目/不指向项目 HEAD 时，整个项目包都拒绝打开。

创建过程先在目标父目录下建立同卷隐藏 staging 目录，完成子目录、SQLite 初始化、完整性校验和临时 manifest 发布后，再用目录 rename 原子发布为最终 `.optimizer` 路径。失败时只清理由 Host 自己生成、且经父目录与命名双重校验的 staging 目录。已存在的最终目录永不覆盖。

`DesktopState` 改为 `Mutex<Option<OpenedProject>>`：

- 启动时没有打开的项目；
- `create_project`、`open_project`、`close_project` 和 `get_project_session` 是显式会话命令；
- 同一时刻只允许一个项目，打开第二个项目前必须关闭当前项目；
- 所有 Operation 写入与审计读取都从当前会话获取 Store；没有项目时返回稳定的 `NO_PROJECT_OPEN`；
- Secret Store 的生命周期独立于项目会话，关闭项目不会删除 provider credential。

四个项目会话命令使用单独的 `allow-project-session` permission，并继续只授权给本地打包的 `main` window。阻塞文件与 SQLite 工作仍在 `spawn_blocking` 中执行。

## 安全与失败语义

- 父目录必须已存在且可 canonicalize；包名必须是单一安全路径组件并以 `.optimizer` 结尾；拒绝遍历、控制字符、Windows 保留名与非法字符。
- manifest 只允许固定数据库文件名，不接受任意相对或绝对数据库路径。
- WebView 收到稳定的公开错误码和脱敏消息，不获得内部 SQLite 文本、凭据或任意文件读写能力。
- 打开包时先执行 SQLite 迁移与 invariant 检查，再向会话发布 `OpenedProject`；失败不会留下半打开状态。

## 取舍与后续

目录 rename 的原子性依赖 staging 与最终目录位于同一父目录，因此 v1 不支持跨卷临时目录。manifest 中标题和语言目前是创建时元数据，运行时响应以 SQLite 权威记录为准；后续修改项目元数据时需要同步更新 manifest 或明确把这两个字段降级为展示缓存。

当前只建立会话与包边界，尚未提供系统文件选择器、最近项目列表、文档树查询和自动保存命令。下一阶段将在同一 Host 会话上增加文档/Block 读取、乐观并发编辑和提交前快照，不允许前端直接打开 SQLite。
