# Optimizer System 项目健康审查

- 审查日期：2026-09-08
- 版本基线：v0.8.0（commit e42d547 + 57 个未提交改动文件）
- 审查方式：静态结构审读 + 全量实测验证（见下表）

## 一、实测验证结果

| 检查项 | 结果 | 备注 |
|---|---|---|
| check:schemas | ✅ | 8 个 schema，含 TS drift guard |
| check:architecture | ✅ | 38 个核心源文件依赖方向合规 |
| check:i18n | ✅ | 502 键，zh-CN/en-US 完全对齐 |
| check:version-sync | ✅ | app v0.8.0 / schema v11 / 11 个迁移一致 |
| check:motion | ✅ (PASS) | 5 处字面量缓动 warning，建议改 var(--ease-*) |
| check:types (tsc) | ✅ | 无类型错误 |
| Node 测试 | ✅ 168/168 | 含 desktop:build 全链路 |
| cargo test --workspace | ✅ 169 通过 / 5 ignored | host 单元 72、store 套件 75、集成 22 |
| clippy -D warnings / fmt --check | CI 强制 | 本地未复跑（CI kernel job 同款门禁） |

**结论：当前工作区（含未提交重构）处于全绿可交付状态。**

## 二、代码结构与模块组织（评价：优）

```
packages/protocol → kernel（纯领域，架构检查禁止依赖 UI/SQLite/Provider）
  ├─ editor-bridge / patch-engine / model-gateway / operation-runner
crates/optimizer-host（Rust 安全边界）→ optimizer-store（SQLite 权威存储）
apps/optimizer-desktop（Tauri 2 IPC）  apps/kernel-lab（零依赖实验台）
```

- 分层清晰且**有机器可执行的守护**（check-architecture.mjs 进 CI），不是纸面架构。
- 30 个 ADR 覆盖每个重大决策，自审计文档（implementation-audit.md）按设计文档逐条对照。
- 代码规模：Rust ~30.3k 行（含测试）、TS packages ~7.9k 行、前端原生 JS ~6.4k 行。
- 正在进行的拆分重构方向正确：main.js 4944→922 行；model_gateway.rs、workspace_commands.rs 拆出同名子模块目录。

## 三、核心功能实现完整性（评价：良）

MVP 主链（项目包校验 → 版本化章节树 → 自动保存/检查点 → Context 编译与预算 → 一次性 capability 授权 → 固定端点流式执行 → 严格输出校验 → 逐 hunk 审查 → 原子应用 + 全程审计）**已闭环且有测试证据**。

缺口（自审计文档口径一致，非本次新发现）：

| 项 | 状态 |
|---|---|
| FR-01 JSON 可读导出、多 Block 创建 | 部分（最小实现） |
| FR-10 费用换算、接受率聚合、诊断导出 | 部分 |
| FR-11 用户可见备份向导、损坏恢复演练 | 部分 |
| 性能预算（10 万字热启动等基准门禁） | 缺失——perf baseline 测试已存在但默认 ignored，无阈值门禁 |
| 跨平台 Secret Store | 仅 Windows（Credential Manager） |
| 正式图标 / updater 签名 / 代码签名 | v0.9.0 计划中 |

## 四、依赖配置（评价：优）

- **前端零第三方运行时依赖**（仅 typescript 7 + @types/node devDeps），静态资源全本地，CSP 仅开本地 + IPC——供应链攻击面极小。
- Rust 直连依赖约 10 个（rusqlite bundled、serde、sha2、zstd、zip、windows-sys 等），全部主流维护库；Cargo.lock 436 包均为传递依赖。
- CI 双 audit（cargo-audit + npm audit）+ release 阻塞门禁 + CycloneDX SBOM 双产物。
- 注意：`package-lock.json` 尚未提交（untracked），而 release job 依赖 npm 安装，建议纳入版本控制。

## 五、技术债务清单（按优先级）

| # | 债务 | 影响 | 建议 |
|---|---|---|---|
| 1 | **57 文件未提交**（拆分重构 +293/-8298，新模块文件全部 untracked） | 单点误操作即丢失数天工作量；重构半成品与 v0.8.0 基线混在一起 | 尽快拆成 1-2 个独立 commit 提交（Rust 拆分 / 前端拆分各一个） |
| 2 | **无 git remote**（27 个提交仅存本机） | 单机故障 = 项目归零 | 添加私有远程并推送 |
| 3 | drawers.js 仍有 2632 行；store.rs 3680 行、project_package.rs 2018 行 | 拆分未完成，改动热点集中 | 沿用既有拆分模式继续 |
| 4 | implementation-audit.md（2026-07-21）滞后 | "发布供应链缺失/i18n 未资源化"等行与 v0.8.0 实际不符，误导后续决策 | 随 v0.9.0 一并刷新 |
| 5 | .stage3_backup/（288K 拆分备份）未 gitignore | 仓库噪音 | 重构提交后删除或加入 .gitignore |
| 6 | check:motion 5 处字面量缓动 | 一致性小瑕疵 | 换 var(--ease-*) |
| 7 | 全仓库仅 1 处 TODO（canonical-json.ts 兼容别名清理） | 债务量极低 | 别名下线时删除 |

## 六、代码质量亮点

- **生产代码 unwrap/panic 仅 5 处**，且全部是带不变量说明的 `expect`（如 "started stream has response ID"）；774 处 unwrap 几乎全部位于测试代码。30k 行 Rust 达到这个水平相当少见。
- 安全边界设计成体系：一次性 120s capability、Host 授权前复算 Packet/Prompt、`never_send` 强制排除、Secret 只在 Rust 侧解析、WebView 无原始网络权限、通用端点禁代理/禁重定向/逐字 origin 确认。
- schema v1→v11 全部带迁移前在线备份、失败回滚、版本一致性 lint。

## 七、总体健康度结论

**健康度：良好偏优（约 B+/A-）。**

- **代码本身**：架构纪律、测试密度、安全设计均显著高于同类个人项目水准，本次全量实测零失败。
- **风险集中在工程外围而非代码**：① 拆分重构未提交且无远程仓库，是当前唯一可能造成不可逆损失的风险；② 性能基准有基础设施但无门禁；③ 平台覆盖（仅 Windows）与发布签名未闭环（均有明确路线）。
- 若完成"提交拆分 + 配置远程 + 性能门禁"三件事，项目可进入 A 档。
