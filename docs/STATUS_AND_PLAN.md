# Altior 项目现状与规划（AI 接续指南）

> 最后更新：2026-10-16（自主会话：v0.1 工作树经用户授权落账为 7 个提交于
> `ui/toolcall-timeline`（未 push）；P3 在栈上分支 `feat/p3-sync-foundation`
> 启动——clippy 1.99 工具链漂移修复、ADR 0027 设计、加密会话加固（套件 2
> ✅）、中继加固（套件 3 ✅）、Vault 领域契约；全门禁通过（Rust+Tauri 690 /
> Vitest 298），见 CHECKPOINT.md 切片 17–22；切片 23（同步引擎 + 套件 1/4）
> 为下一切片。此前：09-26 切片 16 复查修复（7 修 1 证伪）；09-25 视觉钉定；
> 09-24 session_error UI；09-23 A18 三修复 + B03 回环桩）
> 用途：让任意 AI 编码助手在新会话中快速理解项目全貌——做到了什么、正在做什么、
> 还差什么——并能直接继续执行。

---

## 一、项目概览

Altior 是一个 **本地优先、多设备个人知识运行时**。核心定位：

- 一人拥有一个 Personal Vault，设备通过密码学配对加入——没有组织、会员、邀请、
  角色、计费或邮箱登录。
- 网络中断不阻塞本地读写、ACP 对话、记忆检索。
- 中继（Relay）是不受信任的加密邮箱，从不接收明文。
- ACP（Agent Client Protocol）是当前唯一的生产 agent harness；记忆、人格、
  技能等属于 Altior Context Runtime，位于所有 harness 之上。

## 二、技术栈

| 层 | 技术 |
|---|---|
| 核心后端 | Rust stable，edition 2024，9 个 workspace crate |
| 桌面前端 | React 19 + TypeScript + Vite 8 |
| 桌面壳 | Tauri v2（独立 crate，不在 Rust workspace 内，ADR 0008） |
| 存储 | SQLite（rusqlite bundled），日志-权威 + 可重建投影 |
| CRDT | Automerge（胜出于 ADR 0010 bake-off） |
| 密码学 | X25519 / HKDF / ChaCha20-Poly1305（spike，ADR 0011） |
| 测试 | Vitest + Playwright 视觉基线 + cargo test（全 workspace） |
| 质量门禁 | `scripts/quality-gate.ps1` 一键运行全部门禁 |

## 三、Crate 架构

```text
crates/
  altior-domain      纯领域类型/规则，无平台依赖
  altior-protocol    版本化 Desktop↔Core IPC DTO + 事件信封
  altior-ipc         Windows 命名管道 / Unix Domain Socket 传输
  altior-core        守护进程、应用协调器、运行时监管、上下文组装、OS 密钥
  altior-acp         ACP v1 子进程适配器（JSON-RPC over stdio）
  altior-storage     SQLite 持久化（v1→v8 迁移，FTS5 trigram 检索）
  altior-crdt        Automerge SyncDocumentEngine（spike）
  altior-crypto      两设备加密信封（spike）
  altior-relay       内容无关加密中继队列（spike）

apps/
  desktop/           React + Vite 渲染器
  desktop/src-tauri/  Tauri v2 桥接壳（独立于 Rust workspace）
```

**依赖方向**：外层依赖内层，`altior-domain` 不依赖任何基础设施。

## 四、路线图阶段总览

```text
版本 0.1 — 本地连续性    P0 + P1 + P2 （+ A/B 审查修复）
版本 0.2 — Personal Vault    P3 加密多设备同步
版本 0.3 — 个人运行时    P4 项目工作台 + 扩展
稍后 —                    P5 发布加固 + 额外 harness
```

## 五、已完成（✅）

### P0 — 合约与技术验证

| 子阶段 | 内容 | 状态 |
|---|---|---|
| P0.1 | 仓库结构、协议合约、TS DTO 生成 | ✅ ADR 0004–0005 |
| P0.2 | Core 监管与本地 IPC | ✅ ADR 0006 |
| P0.3 | ACP v1 子进程适配器 spike | ✅ ADR 0007 |
| P0.4 | Desktop 五区域工作台 shell、虚拟化时间线、视觉基线 | ✅ ADR 0008 |
| P0.5 | SQLite、CRDT bake-off、密码学信封、中继队列 spike | ✅ ADR 0009–0012 |

### P1 — ACP 桌面连续性

| 子阶段 | 内容 | 状态 |
|---|---|---|
| P1.1 | 领域实体 + SQLite 持久化（v1→v3 迁移） | ✅ ADR 0013 |
| P1.2 | ACP 运行时：子进程启动/流式/取消/检查点 | ✅ ADR 0014 |
| P1.3 | Desktop MVP：线程/转弯/权限/搜索/诊断 | ✅ ADR 0015 |
| P1.4 | 8 步验收旅程（真实守护进程 + 命名管道 + SQLite + mock ACP） | ✅ ADR 0016 |

### P2 — 身份与记忆

| 子阶段 | 内容 | 状态 |
|---|---|---|
| P2.1 | 记忆生命周期、FTS5 检索、秘密内容过滤 | ✅ ADR 0017 |
| P2.2 | 身份文档、确定性上下文组装、可解释注入 | ✅ ADR 0018 |
| P2.3 | ID 边界、epoch 去重、CJK trigram、投影 digest 优化 | ✅ ADR 0019–0024 |

### A 审查运动 (2026-09-05)

A01–A17, A19, A20 全部 **DONE-VERIFIED**。涵盖：历史保真度、epoch
去重、ACP 诚实配置、五区域网格、IME 安全、WCAG 色彩令牌、zh-CN/en 双语、
SafeMarkdown、记忆产品接线、质量门禁等。

A18（真实第三方 ACP + 签名打包）原为 BLOCKED，2026-09-20 部分解锁：
- Tauri WebView 桥接修复（`tauriTransport.ts`）
- `bool_or_object` 反序列化修复（`messages.rs`）
- opencode 1.18.29 真实端到端通过

### B 审查运动 (2026-09-13)

| ID | 内容 | 状态 |
|---|---|---|
| B01 | 门禁脚本修复 + 去重性能 + 像素门禁 | ✅ DONE-VERIFIED |
| B02 | OS Secret Store（ADR 0026，Windows Credential Manager） | ✅ DONE-VERIFIED |
| B04 | 桌面 UI 六项修复（SVG/IME/Grid/i18n/Agents 诚实化） | ✅ DONE-VERIFIED |

**收口门禁 (2026-09-13)**：493 Rust 测试、205 Vitest、对比度 24/24、
视觉 5/5、`quality-gate.ps1` exit 0。

### 最近合并 (2026-09-22, 17 PRs)

- zh-CN 本地化扫描（身份类型、能力令牌、fixture 行）
- UI 打磨（权限卡片焦点、composer chip、检查器空密度、搜索空态 CTA、
  暗色 chrome 对比度）
- `request_snapshot` 与真实 Core 线程快照对齐
- `5ba5fec` — fix(core): map ACP ToolCall to KnownEvent::ToolCall

## 六、进行中（🔧）

### 当前分支：`ui/toolcall-timeline`

截至 2026-09-25 约 1512 行插入 / 809 行删除（内容级 diff，含 CRLF stat 噪声
以外的真实变更 47 文件），加上未跟踪的：

| 文件 | 内容 |
|---|---|
| `docs/UI_REDESIGN_BRIEF.md` | UI 重设计简报 |
| `docs/UI_IA_MAP.md` | 信息架构地图 |
| `apps/desktop/baselines/memory.png` | 新增记忆面板基线 |

**主要工作**：Desktop UI 重设计——记忆面板从卡片瀑布改为密集表格、
时间线/Composer 去聊天气泡化、权限卡片 UX 增强、半径/令牌裁剪、
i18n 新增条目、视觉基线重建。

2026-09-26 追加（同一未提交分支）：批次 4 深度复查修复 7 项——多凭证绑定
更新不再截断 secret_refs、恢复路径改为先历史后快照（ADR 0020）、检查器
标签箭头键移动焦点、时间线行内按键守卫、记忆模式失败横幅、IPC 状态药丸
i18n、sessionError 仅被 open_thread 置/清；Vitest 291→298。详见 CHECKPOINT
切片 16。

> ⚠️ 本工作**尚未提交**——按执行规则，提交需要用户明确授权。
> main 上存在一个已暂存的 stash：`parked-old-core-wip-do-not-mix-with-ui`。

## 七、待做 / 计划中（📋）

### 近期（v0.1 收尾）

| 优先级 | 任务 | 说明 | 阻塞 |
|---|---|---|---|
| ~~P1~~ ✅ | **B03** — 本地确定性 ACP 回环桩 | 2026-09-23 完成：`loopback_smoke.rs` 用真实子进程桩跑完整 smoke 旅程（含取消旅程），接入 Gate 4 必跑；外部真实 ACP 保持 opt-in。 | 已完成 |
| ~~P1~~ ✅ | **A18 残留** — `configure_agent` 更新报错 | 2026-09-23 修复：daemon 保留已存在 profile/binding 的 `created_at`（p34 测试）。 | 已完成 |
| ~~P1~~ ✅ | **A18 残留** — `open_thread` 吞掉 `create_session` 错误 | 2026-09-23 修复：`ThreadOpenResult`/`ThreadSnapshotDto` 新增有界 `session_error`，读路径不阻塞（p34 测试）。 | 已完成 |
| ~~P1~~ ✅ | **A18 残留** — `probe_binding` 无诊断输出 | 2026-09-23 修复：失败路径经 `BoundedDiagnosticsSummary` 返回含 program 的原因（p34 测试）。 | 已完成 |
| ~~P2~~ ✅ | 诊断面板 Refresh 按钮 i18n | 2026-09-23 切片 10 核验：已由重设计分支完成（`t.common.refresh`）。 | 已完成 |
| ~~P2~~ ✅ | App 空态文案 i18n | 2026-09-23 切片 10 核验：四态空态均走 `workbenchEmpty.*`；09-26 切片 16 另补 IPC 状态药丸 i18n。 | 已完成 |
| **P2** | ~~桌面 UI 渲染 `session_error`~~ ✅ | 2026-09-24 完成：Composer 上方 `role="status"` 通知（store 视图字段 + i18n en/zh + store/App 测试）。 | 已完成 |
| **P2** | UI 重设计后续阶段 | Agents 配置页打磨、命令面板（列为范围外）。 | UI 重设计分支先合并 |
| **P2** | 真实第三方 ACP smoke | 需 Claude Code / Gemini CLI 的有效 API Key + CLI 可执行程序。 | 外部凭证 |

### 中期 — P3 Personal Vault 同步

> **进展（2026-10-17，分支 `feat/p3-sync-foundation`，ADR 0027 为设计依据）**：
> **ADR 0025 四个验收套件全部实现并通过**：套件 1（三设备收敛）✅、
> 套件 2（nonce 唯一性）✅、套件 3（中继零知识审计）✅、套件 4（墓碑不复活）✅。
> 同步引擎（签名日志 + pull-before-push + 墓碑抑制 + ack 回流压缩门控 + 重启安全持久化）、**blob 平面（ADR 0028）**、**会话链快照恢复（crypto 层）**、
> 存储 v9（计数器租约等四表 + SqliteCounterLease + 墓碑 GC 门）已落地；全门禁绿（Rust+Tauri **699** / Vitest 298）。
> `SYNC_ENABLED` 屏障保持关闭——翻它还需独立密码学审查（ADR 0025），且永远
> 不是 UI 开关。P3 剩余：设备配对 UI、同步诊断面板、Desktop 会话接线、blob 平面。

| 内容 | 阻塞 |
|---|---|
| ~~设备身份、配对指纹、恢复、撤销、密钥轮换~~ 契约层 ✅（`vault.rs`）；SQLite 持久化在切片 23 | ADR 0025 的 11 项威胁缓解（会话/中继/契约层已完成 8/11） |
| 加密日志 + CRDT 文档 + opt-in blob 同步平面 | 引擎在切片 23；blob 平面延后 |
| ~~不受信任中继追赶、回放保护、配额~~ ✅ 中继层完成 | |
| ~~压缩保留墓碑和撤销前沿~~ ✅ 契约层完成；引擎接线在切片 23 | |
| 设备 UI 和同步诊断 | 引擎落地后 |

**验收**：三设备在离线创建/修正/遗忘冲突后收敛；中继检查无明文；
已撤销设备无法发布被接受的事件；模拟 30 天离线设备不能复活已遗忘记忆。

当前状态：`sync_enabled = false` 硬屏障维持（ADR 0025），有编译期测试断言。

### 远期 — P4 / P5 / Later

| 阶段 | 内容 |
|---|---|
| **P4** | 项目工作台：文件树、Git 状态/diff、PTY、MCP/技能注册表、调度器、路径隔离 |
| **P5** | 发布加固：Windows 签名/安装器/升级器、崩溃恢复/浸泡/资源/损坏测试、许可证审计 |
| **Later** | Terminal/Codex app-server harness、有界多 agent 协调、Altior 原生 harness、语音、移动伴侣 |

P5 安装包签名阻塞于**外部代码签名证书**（B06）。

## 八、外部阻塞登记

| 阻塞项 | 影响 | 需要 |
|---|---|---|
| 第三方 ACP 代理凭证 | A18 完整闭环、真实 smoke | Claude Code / Gemini CLI 的有效 API Key + 可执行程序 |
| Windows Authenticode 证书 | B06 安装包签名 | 有效的硬件代码签名数字证书 |
| 自托管中继基础设施 | B05/P3 同步 | 公网或测试网 WebSocket 节点 |
| macOS / Linux 主机 | OS Secret Store 跨平台 | 对应平台测试环境 |

## 九、已有 ADR 一览

项目已记录 **26 个 ADR**（`docs/decisions/0001–0026`），全部 Accepted：

| 范围 | ADR |
|---|---|
| 基础 | 0001 文档先行、0002 聚合运行时、0003 ACP 优先 |
| 合约 | 0004 稳定 ID/信封、0005 Rust 驱动 TS DTO、0006 本地 IPC、0007 ACP 适配器范围 |
| Spike | 0008 Desktop shell、0009 SQLite 日志、0010 CRDT bake-off、0011 密码学信封、0012 中继队列 |
| P1 | 0013 领域实体、0014 ACP 运行时、0015 Desktop MVP、0016 验收旅程 |
| P2 | 0017 记忆生命周期、0018 身份文档/上下文、0019 ID 分配、0020 有界历史、0021 epoch 去重、0022 上下文边界、0023 FTS5 trigram、0024 投影 digest |
| 安全 | 0025 同步生产硬门禁、0026 OS Secret Store |

## 十、测试覆盖

| 层 | 规模 | 说明 |
|---|---|---|
| Rust workspace | 510 测试 | 每个 crate 都有 `tests/`，含 p 系列验收测试 |
| Vitest (Desktop) | 291 测试 | 37 个测试文件，p04–p38 证据命名 |
| Playwright 视觉基线 | 6 张 | light/dark/approval/error/memory/narrow |
| 对比度审计 | 24/24 | WCAG 2.2 合规 |
| Tauri 后端 | 15+ 测试 | 附加/生成、认证、进程生存、路径遍历防护 |

## 十一、开发纪律提醒

新会话的 AI 必须遵守的关键规则（摘自 `docs/AI_DEVELOPMENT.md`、
`AGENTS.md`、以及项目 memory）：

1. **永不自动提交**——提交需要用户明确授权。
2. **BLOCKED 不伪造 PASS**——无法验证的切片标记 BLOCKED 而非假通过。
3. **继续执行无需询问**——在没有外部依赖阻塞的情况下，直接进入下一个
   依赖已满足的任务。
4. **保持 CHECKPOINT.md 更新**——每个完成的切片都要追加执行记录。
5. **门禁命令**：`cargo fmt`、`cargo clippy --all-targets --all-features -- -D warnings`、
   `cargo test --workspace`、`npm --prefix apps/desktop run gate`。
6. **Windows 注意事项**：heredoc 会被反斜杠/CRLF 破坏（用 Write 工具代替）；
   Vitest 必须从 `apps/desktop` 运行。

## 十二、文件导航

| 你想了解… | 读这个文件 |
|---|---|
| 产品愿景与约束 | `docs/PRODUCT.md` |
| 技术架构 | `docs/ARCHITECTURE.md` |
| 安全模型 | `docs/SECURITY.md` |
| 同步设计 | `docs/SYNC.md` |
| 记忆系统 | `docs/MEMORY.md` |
| Agent Harness 设计 | `docs/HARNESSES.md` |
| UI 架构 | `docs/UI_ARCHITECTURE.md` |
| 验收标准 | `docs/ACCEPTANCE.md` |
| 实施计划（详细进度） | `docs/IMPLEMENTATION_PLAN.md` |
| 工作包 | `docs/WORK_PACKAGES.md` |
| 路线图 | `docs/ROADMAP.md` |
| 架构决策记录 | `docs/decisions/0001–0026` |
| A 审查记录 | `docs/reviews/2026-09-05/CHECKPOINT.md` |
| B 审查记录 | `docs/reviews/2026-09-13/CHECKPOINT.md` |
| AI 开发规范 | `docs/AI_DEVELOPMENT.md` |
| UI 重设计简报 | `docs/UI_REDESIGN_BRIEF.md`（未提交） |
| 信息架构地图 | `docs/UI_IA_MAP.md`（未提交） |
| 贡献规则 | `AGENTS.md`（仓库根目录） |

---

*本文件是给 AI 接续者的快速上手指南。权威的实施细节和验收证据在上述各文档中。
修改本文件时请同步更新「最后更新」日期。*
