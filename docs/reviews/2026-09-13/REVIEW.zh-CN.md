# Altior 综合技术审查报告（2026-09-13）

**审查基准**：代码库工作树（基于提交 `f945ef153eb3344abd3801a3c2e4002fd25d80be` 及其后工作项）  
**2026-09-13 收口更新**：B01/B02/B04 修复完成并经独立复核与全量门禁复跑，本报告相关结论已同步纠正（详见 §一 复跑记录、§三.4、§三.9、§四、§五）。  
**执行角色**：Altior 独立审查联合小组（架构、Rust 核心、安全、存储与门禁审计）  
**适用规约**：`AGENTS.md`、`docs/AI_DEVELOPMENT.md`、已接受 ADRs（0001–0025）

---

## 一、 自动化门禁实测结果记录

首轮审查对仓库规定的所有质量门禁进行了本地实机全量分段执行；B01–B04 收口后由接管协调方（Claude Code）于 2026-09-13 复跑全量 `scripts/quality-gate.ps1`（exit 0，全绿），并单独复跑 `dto-export`。以下表格为收口后的最终实测结果：

| 门禁项 | 执行命令 | 实测结果 | 耗时/指标 |
|---|---|---|---|
| **Rust 格式检查** | `cargo fmt --all -- --check` | **PASS** | 0 违规，代码格式完全整洁 |
| **Rust 静态分析** | `cargo clippy --all-targets --all-features -- -D warnings` | **PASS** | 0 警告 / 0 错误（耗时 0.41s） |
| **Rust 工作区全测** | `cargo test --workspace` | **PASS** | 9 大 crates 全绿（493 通过 / 0 失败，较首轮 470+ 增加 Secret Store 与协议 fixture 新测试）：<br>• `altior-acp`: 49 passed<br>• `altior-core`: 93 passed（含 P14 旅程、P15 历史、P22/P23 上下文、P24 记忆产品、P25 CJK 检索相关性等）<br>• `altior-crdt`: 11 passed（10 对抗测试 + 1 bakeoff 指标）<br>• `altior-crypto`: 21 passed（两设备配对、重放窗口、会话安全）<br>• `altior-domain`: 60 passed（实体校验、ID 边界、SecretShape 探针）<br>• `altior-ipc`: 35 passed（命名管道/UDS 真实数据流、帧往返、恢复）<br>• `altior-protocol`: 72 passed（信封往返、协商、校验）<br>• `altior-relay`: 14 passed（队列深度、配额、压实、两设备流程）<br>• `altior-storage`: 115 passed（71 领域事件、13 身份快照、13 日志、10 记忆、2 性能压测） |
| **Tauri 壳静态分析** | `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml -- -D warnings` | **PASS** | 0 警告 / 0 错误 |
| **Tauri 壳独立测试** | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` | **PASS** | 7 passed（验证 UI 关闭不杀子进程、连接复用、状态断线重连） |
| **TypeScript DTO 导出** | `cargo test -p altior-protocol --features dto-export` | **PASS** | 144 passed（新增 27 项桌面命令 fixture 往返测试；自动导出 TS DTO，强制规范 LF 换行、EOF 单换行，杜绝 `bool` 关键字） |
| **桌面前端全门禁** | `npm.cmd --prefix apps/desktop run gate` | **PASS** | 包含 7 阶段全量门禁：<br>1. `tsc --noEmit`: 0 错误<br>2. `check-lint.mjs`: 0 违规<br>3. `check-format.mjs`: 全文件整洁<br>4. `vitest run`: 23 测试文件，205 测试用例全过（新增 IME/防抖/诊断面板/Grid 证据测试）<br>5. `check-contrast.mjs`: 24/24 颜色对比度通过 WCAG 2.2 4.5:1/3:1 标准<br>6. `check-visual.mjs`: 5/5 场景真实 Playwright 截图 + pixelmatch 像素回归全过（阈值 0.1，上限 50 像素，本轮全部 0 diff）<br>7. `vite build`: 生产构建成功 |

---

## 二、 三层环境验证链路定性划分

为了彻底杜绝将单元测试或 Mock 回复冒充为端到端生产可用的假象，本报告明确区分三层链路：

1. **Fixture / Mock 验证层（单元测试与模拟环境）**：
   - 包含 `MockAcpAgent`（Alpha/Beta）、`InMemoryTransport`、合成事件流、固定时间戳与静态伪随机种子。
   - **定性评估**：作为回归门禁 100% 完备且确定性极高，已在 CI/本地全量通过。
2. **Local Gates（本地操作系统与硬件闭环门禁）**：
   - 包含真实的 Windows Named Pipe (`\\.\pipe\altior-core-...`) 与 Unix Domain Socket 通信；真实的磁盘 SQLite WAL 文件读写、FTS5 倒排索引构建与崩溃自愈；真实的无头浏览器 DOM 几何与对比度渲染计算；真实的 Tauri 进程生成与生命周期脱钩。
   - **定性评估**：完全真实执行且全部通过，有力证明了本地运行时与单机个人知识库的健壮性。
3. **Live Production Path（真实第三方与生产端到端链路）**：
   - **真实第三方 ACP 模型代理进程**：依赖外部商用 CLI 程序（如 Claude Code / Codex CLI）以及有效的云端 API 凭证。在默认的 `scripts/quality-gate.ps1` 中由于未设置 `ALTIOR_ACP_SMOKE_AGENTS`，被明确汇报为 `[SKIPPED]`，**定性为未全自动验证（Opt-in）**。
   - **真实操作系统凭证管理器（OS Secret Store）**：B02 已完成 Windows Credential Manager 生产驱动接入（ADR 0026），并经独立安全复核 PASS 与真实条目往返测试（含 RAII 清理）验证；macOS Keychain / Linux Secret Service 驱动明确 deferred 且 fail-closed，**定性为 Windows 已实测接入，其余平台受控延期**。
   - **真实跨网络多设备同步**：依 ADR 0025 架构硬屏障全阻断（`sync_enabled = false`），**定性为生产未开放（Disabled by Design）**。

---

## 三、 重点领域分项核验分析

### 1. 架构边界、产品不变量与领域依赖方向
- **核验结论**：**通过 (Pass)**
- **精准证据**：
  - `crates/altior-domain/Cargo.toml`（第 8–10 行）严格只依赖 `serde` 与 `serde_json`。
  - `crates/altior-core/tests/dependency_boundaries.rs`（第 45–60 行）通过编译期测试断言 `altior-domain` 绝对不包含 `tauri`、`sqlite`、`rusqlite`、`acp`、`tokio` 等依赖字眼。
  - 核心运行时（`crates/altior-core/src/context/mod.rs`）位于 ACP 适配器（`crates/altior-acp`）之上，模型上下文装配、Token 预算估算、敏感词拦截均由 Core 独立管控，未泄漏进 ACP 协议层。
  - Desktop 客户端仅作为 IPC 客户端与 Core 进程交互，`apps/desktop/src-tauri/Cargo.toml` 未引入 `altior-storage` 或 SQLite 引擎，绝无越权操作数据库行为。
  - Lody 来源核验：全代码库零源码依赖，`THIRD_PARTY_NOTICES.md` 与根目录 `LICENSE` 齐备。

### 2. IPC 版本化、握手协商与协议安全
- **核验结论**：**通过 (Pass)**
- **精准证据**：
  - `crates/altior-protocol/src/version.rs`（第 82–140 行）定义了 `ProtocolVersionRange`，双端通过计算版本区间的交集协商协议版本；交集为空时立即抛出 `ProtocolError::NoCommonProtocolVersion`，绝无隐式静默降级。
  - `crates/altior-protocol/src/handshake.rs`（第 21–93 行）中 `DesktopHello` 与 `CoreHello` 区分了协议版本与仅供诊断的产品展示版本，能力（Capability）基于 `CapabilitySet` 双向显式声明，杜绝根据版本字符串推测能力。
  - `crates/altior-protocol/src/command.rs`（第 883–898 行）与 `event.rs`（第 380–403 行）强制每个信封携带 `protocol_version` 与 `operation_id`；未知未来事件被保存在 `EventBody::Unknown` 中安全穿透。

### 3. 离线优先自治性
- **核验结论**：**通过 (Pass)**
- **精准证据**：
  - `crates/altior-core/Cargo.toml` 与 `crates/altior-storage/Cargo.toml` 中没有引入任何 HTTP 客户端或远程网络库（如 `reqwest` 或 `hyper`）。
  - 会话创建、Prompt 分发、Turn 执行、历史漫游、记忆管理与本地 FTS5 索引检索 100% 运行在本地进程内，断网时全部核心交互不受任何阻碍。

### 4. 秘密存储与凭据隔离
- **核验结论**：**通过 (Pass)**（B02 收口后，经第二路独立复核确认）
- **精准证据**：
  - **合规部分**：SQLite 数据库（`crates/altior-storage/src/migrations.rs` SCHEMA_V5）仅持久化 `secret_refs_json`（如 `["vault:credentials:key1"]`），严禁写入明文；`altior-ipc/src/auth.rs` 中 `LaunchToken` 在 `Debug` 和 `Display` 格式化中强制输出 `<redacted>`；`crates/altior-core/src/runtime/diagnostics.rs` 自动对诊断信息执行敏感 Token 掩码。
  - **原 P1 缺陷已修复（B02 / ADR 0026）**：`crates/altior-core/src/secrets.rs` 实现 `OsSecretStore` 生产驱动（Windows Credential Management API：`CredReadW/CredWriteW/CredDeleteW`，UTF-16 宽字符、`CredFree` 全路径释放、`ERROR_NOT_FOUND` 映射 `NotFound`、删除幂等）；`AcpHarnessAdapter::new()` 默认装配该 resolver，封闭测试保留显式 `NoSecretsResolver`/`with_no_secrets()`；`SecretRef` 规范映射（`vault:credentials:`/`secret://`/`sec_`/裸名 → `Altior/credentials/<name>` 命名空间）拒绝空值/控制字符/超 256 字节；缺失或非 Windows 平台一律 fail-closed。独立复核报告（任务 01a098b0）核验 API 用法、脱敏（`ResolvedLaunchConfig` Debug 掩码、错误只含 ref 不含明文）、解析单向性与 ADR 平台矩阵诚实性，结论 PASS。

### 5. 同步安全：防重放、撤销、轮换与资源上限
- **核验结论**：**通过（生产硬屏障受控封锁）**
- **精准证据**：
  - **P0 风险现状**：`crates/altior-crypto/src/session.rs`（第 60–95 行）在静态密钥两设备会话重启时，`send_counter` 直接从 0 开始，导致同一密钥下的 ChaCha20-Poly1305 Nonce 出现复用隐患（ADR 0025 F31）；代码中尚未实现设备撤销证书（Revocation Certificates）与数据密钥轮换机制。
  - **硬屏障有效性**：根据 ADR 0025 与 `docs/SECURITY.md`（第 44–65 行），多设备同步在生产发布中被严格禁用（`sync_enabled = false`）；`crates/altior-core/Cargo.toml` 移除了对 `altior-crypto`、`altior-relay` 与 `altior-crdt` 的依赖，在二进制构建级别杜绝了风险渗入。
  - **资源上限**：`crates/altior-domain/src/entity.rs` 对记忆内容（32 KiB）、摘录（4 KiB）、启动参数（64 KiB）等施加了严格上限；`crates/altior-core/src/runtime/adapters/acp.rs`（第 34 行）设置了 `CHANNEL_BOUND = 1024` 的有界通道。

### 6. 墓碑与压实机制
- **核验结论**：**本地通过；分布式压实留待 P3**
- **精准证据**：
  - `crates/altior-storage/src/memory.rs`（第 596–649 行）：`forget_memory` 向不可变追加日志 `domain_journal` 提交 `MemoryForgotten` 事件，并把投影表状态改为 `forgotten`。
  - `crates/altior-storage/src/migrations.rs`（第 405–411 行）：`memory_fts_update` 触发器在状态为 `forgotten` 时立即将其移出 FTS 索引，永远不再被检索或注入上下文。
  - `crates/altior-storage/src/migrations.rs`（第 133–143 行）：SQLite 触发器直接禁止对 `journal` 与 `domain_journal` 进行 `UPDATE` 或 `DELETE`，杜绝本地历史擦除。
  - 跨设备向量时钟墓碑防复活与压实算法（ADR 0025 Threat 10）属于 P3 规划项，目前在单机架构中不激活。

### 7. 记忆系统、置信度、来源与安全过滤
- **核验结论**：**通过 (Pass)**
- **精准证据**：
  - `crates/altior-domain/src/entity.rs`（第 2130–2195 行）：`MemoryRecord` 严格包含 `scope`、`kind`、`confidence`（0..=100）、`provenance`、`source`、`state`、`expires_at` 与 `superseded_by`。
  - `crates/altior-domain/src/secret_shape.rs`（第 10–64 行）：静态正则表达式过滤 9 大类凭据模式（AWS / sk- / PEM / GitHub / Slack / JWT / 键值赋值 / Hex / Base64）。
  - `crates/altior-storage/src/memory.rs`（第 250 行）：写入与检索双重 fail-closed 校验，任何敏感信息均在入库前被彻底拦截。
  - 检索可解释性：`MemoryHit` 携带 `why_matched: MemoryMatchExplanation`，说明匹配项与分值计算依据。

### 8. 取消安全、幂等性与 Prompt 重复投递控制
- **核验结论**：**通过 (Pass)**
- **精准证据**：
  - `crates/altior-domain/src/lib.rs`（第 128–139 行）：严密定义 `DeliveryState`（Absent, Confirmed, Rejected, Indeterminate）。
  - `crates/altior-core/src/application/mod.rs`（第 1053–1075 行）：
    ```rust
    if is_terminal
        || delivery == DeliveryState::Confirmed
        || delivery == DeliveryState::Indeterminate
    {
        return Err(CoreAppError::AutomaticResendForbidden { ... });
    }
    ```
    对于可能已达或执行状态未决的轮次，绝对禁止自动重发。
  - `crates/altior-core/src/operations.rs`（第 63–70 行）：`OperationRegistry` 记录已处理的 `OperationId`，重复命令只返回 `Admission::Duplicate`，不重复执行。
  - `crates/altior-core/src/ownership.rs`：桌面 UI 重载、窗口关闭绝不杀灭 Core 正在运行的后台轮次。

### 9. 桌面 UI、Tauri/Core IPC 与真实 ACP 连续性
- **核验结论**：**通过（B04 修复后复验）**；真实外部 ACP 连续性仍为未自动验证（见 §五）。
- **桌面审计发现与 B04 修复（均已落地并经接管方独立代码复核）**：
  1. Activity Rail 原 `label.slice(0,2)` 文字截断图标 → 替换为 6 个语义化 inline SVG（`aria-hidden`、`currentColor` 主题感知、CSS 可缩放），按钮保留本地化 `aria-label`/`title`，Projects 角标更正为 P4。
  2. 线程搜索接入 280ms 可测试防抖（`debounceMs` prop，测试可置 0）与 `compositionstart/end` IME 保护：合成态绝不发 IPC，合成结束才以最终值派发（`p19AccessibilityImeEvidence` 以 fake timers 锁定该行为）。
  3. Inspector/Settings 新增 `RuntimeDiagnosticsView`：loading/empty/error/loaded 四态稳定渲染，仅展示 instance_id、状态、活跃线程/轮次计数与摘要等元数据，不携带任何环境变量值或秘密；并附脱敏说明。
  4. 移除 App 底部 P0.1 `<details>` 协议调试区及破坏 Grid 的行，恢复 ADR 0008 三行网格；协议证据保留为 `display:none` 测试钩子容器。
  5. TimelineRowView、ContextPanel、MemoryPane、shell 列举的硬编码中英文全部接入 `useI18n`（zh-CN/en 词条齐备），清除 Provisional 冗文；经全文件 Han 字符扫描确认四文件零残留。
  6. Agents 管理不伪造 list/probe/disable/delete 按钮：后端无对应契约，以本地化 deferred 说明诚实标注。
- **复验证据**：Vitest 23 文件 / 205 用例全过（含新增 p18 Grid/p19 IME 证据测试）、WCAG 24/24、pixelmatch 5/5 场景 0 diff（基线为 UI 变更后显式 `npm run baselines` 重建）、vite build 成功。
- **残留小项（P2）**：诊断面板 Refresh 按钮文案、App 空态文案未接入 i18n（超出本轮列举范围，已登记后续处理）。

---

## 四、 确认问题与风险矩阵（P0 / P1 / P2）

| 编号 | 等级 | 问题名称与位置 | 精确文件与行号 | 影响分析与现状缓解 |
|---|---|---|---|---|
| **P0-1** | P0 (受控) | 同步会话重启 Nonce 复用漏洞与缺失撤销/轮换 | `crates/altior-crypto/src/session.rs:60-95` | **影响**：会话重建计数器重置破坏 ChaCha20-Poly1305 机密性；缺失设备撤销。<br>**缓解**：ADR 0025 确立生产绝对屏障（`sync_enabled = false`），Core 剔除了对该 crate 的依赖，物理阻断流入主干。 |
| **P1-1** | ~~P1~~ 已解决 | OS Secret Store 真实驱动未实现（仅保留桩接口） | 原 `crates/altior-acp/src/config.rs:400-411`、`crates/altior-core/src/runtime/adapters/acp.rs:101` | **已修复（B02/ADR 0026）**：`crates/altior-core/src/secrets.rs` Windows Credential Manager 生产驱动接入，独立复核 PASS（任务 01a098b0）；macOS/Linux deferred fail-closed。 |
| **P1-2** | P1（B03 未闭环，受控） | 真实第三方 ACP 代理门禁在默认流水线中跳过 | `scripts/quality-gate.ps1` Gate 4、`crates/altior-acp/tests/smoke.rs` | **现状**：门禁仍依赖 Mock 代理，真实外部代理 smoke 保持显式 opt-in（未设置 `ALTIOR_ACP_SMOKE_AGENTS` 时诚实 `[SKIPPED]`）；B03 本地确定性回环桩本轮未实施，维持 pending。 |
| **P1-3** | P1 | 跨设备分布式同步关键机制未在代码层面落地 | `crates/altior-crypto/`<br>`crates/altior-core/src/application/` | **影响**：P3 同步功能开放前，必须补齐 Double Ratchet、持久计数块与先拉后推握手。 |
| **P2-1** | ~~P2~~ 已解决 | 存储测试目录遗留空文件 | 原 `crates/altior-storage/tests/relevance_eval.rs:1` | **已修复（B01）**：空壳文件已删除，真实检索测试在 `altior-core` `p25_memory_retrieval_relevance`。 |
| **P2-2** | ~~P2~~ 已解决 | 质量门禁脚本中 npm 调用触发权限策略拦截 | `scripts/quality-gate.ps1` | **已修复（B01）**：npm 经 `cmd.exe /c` 调用（解析为 `npm.cmd`），并新增每步 `$LASTEXITCODE` fail-fast 断言，失败注入验证非零退出。 |
| **P2-3** | ~~P2~~ 已解决 | 桌面 UI 专项审计结论处于占位状态 | （桌面专项审计） | **已解决（B04）**：审计精要已补发，六项修复落地并复验（见 §三.9），最终前端门禁全绿。 |

---

## 五、 明确的“未验证 / 未实现”项

1. **真实 OS Secret Store（Windows 已实测；macOS/Linux 未验证）**：
   - Windows Credential Manager 已接入生产 resolver 并通过真实条目往返/清理与独立安全复核；macOS Keychain 与 Linux Secret Service 驱动未实现（cfg 门控 fail-closed），需对应平台开发主机验证。
2. **真实第三方 ACP 代理连续性（未自动验证）**：
   - 本地流水线仅验证了 `MockAcpAgent`；外部真实商用模型代理需额外配置环境变量与有效凭据，日常门禁处于 `[SKIPPED]` 状态；B03 本地确定性回环桩未实施。
3. **多设备网络同步（未实现 / 故意禁用）**：
   - 当前发布版本中多设备同步处于绝对禁用状态，不可当作生产已具备的功能。
4. **发布包代码签名与安装分发（未执行）**：
   - Windows 安装包签名、自动更新链路（P5）留待后续版本。
