# AI 执行检查点（2026-09-13 审查周期）

本文件用于记录 2026-09-13 审查周期及后续 B01–B06 任务包的执行进度，不能替代长期契约或已接受 ADR。

---

## 初始状态

- **审查基准**：当前工作树状态（基于提交 `f945ef153eb3344abd3801a3c2e4002fd25d80be` 及其后工作项）。
- **本轮已完成核心工作**：
  1. 完成全代码库 Rust 核心、同步、安全、记忆与质量门禁的独立深度审查，全部门禁实测通过。
  2. 修复 `docs/reviews/2026-09-05/CHECKPOINT.md` 中关于 A01–A20 全闭环的虚假表述，明确 A18 仍为 BLOCKED，并将 `UnsupportedNewerSchema` 纠正为代码实际 `StorageError::SchemaTooNew`。
  3. 在 `ADR 0018` 增加非破坏性 update note，合规指向 `ADR 0022` 的 1024/2048 预算规范，保护历史决策不变。
  4. 更新 `docs/WORK_PACKAGES.md` 对 P2.1–P2.3 与 ADR 0019–0025 的完成度说明，诚实声明真实 ACP、OS Secret Store 与生产同步的未完成状态。
  5. 确立 `docs/reviews/2026-09-13/` 审查集（README、REVIEW.zh-CN、TASKS、CHECKPOINT）。
- **桌面审计专项状态**：
  - 桌面专项审计精要已于本轮补发并完成 B04 修复与复验（详见 TASKS.md 执行状态记录与 REVIEW.zh-CN.md §三.9）。

---

## 任务执行状态看板

| ID | 标题 | 优先级 | 状态 | 依赖/交接提示 |
|---|---|---|---|---|
| **B01** | 修复门禁脚本权限兼容性与清理无用测试桩 | P2 | ✅ DONE-VERIFIED | `quality-gate.ps1` fail-fast + `cmd.exe` npm 调用；`relevance_eval.rs` 已删除；附带 streamDeduplicator O(1) 与 pixelmatch 真实像素门禁 |
| **B02** | 接入操作系统原生凭证管理器（OS Secret Store） | P1 | ✅ DONE-VERIFIED | ADR 0026 + `secrets.rs` Windows 驱动；独立安全复核 PASS（任务 01a098b0）；macOS/Linux deferred fail-closed |
| **B03** | 提供真实 ACP 本地确定性回环 Smoke 门禁 | P1/P2 | ✅ DONE-VERIFIED | 2026-09-23 完成：`loopback_smoke.rs` 真实子进程旅程接入 Gate 4 必跑；外部 ACP 保持 opt-in（见切片 9） |
| **B04** | 桌面 UI、Tauri IPC 专项审计问题修复 | 待定 | ✅ DONE-VERIFIED | 六项修复落地（SVG/IME/diagnostics/Grid/i18n/Agents 诚实化）；Vitest 205、contrast 24/24、pixel 5/5 全 0 diff；接管方代码复核实属 |
| **B05** | 落实 ADR 0025 多设备同步安全前置基线 | P0/P1 | BLOCKED (受控延期) | `sync_enabled = false` 屏障维持；待 P3 安全闭环 |
| **B06** | 发布工程：Windows 签名、安装器与升级验证 | P2 | BLOCKED (受控延期) | 待外部代码签名证书（P5） |

---

## 外部环境阻塞登记

1. **A18 真实第三方 ACP 代理外部凭证**：
   - 依赖外部开发者或 CI 运行环境注入有效的商用 API Key 及对应的 CLI 可执行程序（Claude Code / Codex CLI）。
2. **Windows Authenticode 代码签名证书**：
   - B06 安装包签名依赖公司/个人有效的硬件代码签名数字证书。
3. **自托管多设备同步中继网络基础设施**：
   - B05 中继端到端部署依赖真实的公网或测试网 WebSocket 节点。

---

## 每个工作切片追加模板

```text
Execution time / environment:
Task ID / subslice:
Baseline / current commit:
Pre-existing unrelated changes:
User-visible outcome:
In scope / out of scope:
Contracts and decisions adopted:
Changed files:
Failure / cancellation / offline / restart:
Security / synchronization impact:
Migration / downgrade:
Regression proof before and after:
Exact verification commands and results:
Screenshots actually inspected:
Not run / blockers / residual risks:
Status: [DONE-VERIFIED | BLOCKED | IN-PROGRESS]
Next task and first concrete action:
```

---

## 本轮执行切片记录（2026-09-13 收口）

### 切片 1 — B01 门禁失真修复与去重性能（DONE-VERIFIED）
- Baseline: `f945ef1` 工作树；执行: OpenCode（任务 01a09870/01a0989e）。
- 变更: `scripts/quality-gate.ps1`（$LASTEXITCODE fail-fast、cmd.exe npm 调用）；`apps/desktop/src/stores/streamDeduplicator.ts`（摊销 O(1)）；`apps/desktop/scripts/check-visual.mjs`（pixelmatch 真实像素门禁）；删除空壳 `relevance_eval.rs`。
- 验证: 失败注入非零退出；百万事件结构化性能契约；正向 0 diff / 扰动 2174 像素失败；cargo test -p altior-storage 通过。
- 风险/未验证: 无新增；视觉基线仅可显式 `npm run baselines` 更新。

### 切片 2 — B02 OS Secret Store（DONE-VERIFIED）
- Baseline: 同上；执行: OpenCode（任务 01a0987d）；独立复核: OpenCode 第二路（任务 01a098b0）+ 接管方 Claude Code 通读。
- 变更: `docs/decisions/0026-os-secret-store-credential-resolution.md`；`crates/altior-core/src/secrets.rs`；`crates/altior-core/src/runtime/adapters/acp.rs`（默认 OsSecretStore、`with_no_secrets()` 测试口）；`crates/altior-core/Cargo.toml`（windows-sys）；`docs/SECURITY.md`。
- 验证: Windows Credential Manager 真实随机条目写入/读取/SecretResolver 解析/删除 + RAII 清理全闭环；fail-closed（缺失/非 Windows）；`cargo test -p altior-core secrets` 5 通过；clippy -D warnings 0 警告；复核结论 PASS。
- 安全边界: 明文仅流向子进程环境；SQLite/日志/诊断/IPC/serde 均无明文；Desktop 永不读取回明文。
- 未验证: macOS Keychain / Linux Secret Service（deferred fail-closed，需对应平台主机）。

### 切片 3 — B04 桌面 UI 修复（DONE-VERIFIED）
- Baseline: 同上；执行: OpenCode（任务 01a098b1）；独立复核: Claude Code 接管方逐项代码核验。
- 变更: `shell.tsx`（SVG RailIcon/防抖+IME/RuntimeDiagnosticsView/Agents deferred 说明）、`App.tsx`/`App.module.css`（调试区移除、Grid 恢复）、`TimelineRowView.tsx`/`ContextPanel.tsx`/`MemoryPane.tsx`/`i18n/*`（本地化补齐）、`p18/p19` 证据测试、五张基线显式重建。
- 验证: typecheck/lint/format PASS；Vitest 23 文件 205 用例 PASS；contrast 24/24；test:visual 5/5 全 0 diff；vite build 成功。
- 残留: Refresh 按钮与 App 空态文案未走 i18n（P2，登记后续）。

### 切片 4 — 收口门禁复跑（DONE-VERIFIED）
- 执行: Claude Code（任务 01a098b6），2026-09-13。
- 命令与结果: `cargo fmt --all -- --check` PASS；`cargo clippy --all-targets --all-features -- -D warnings` PASS；`cargo test --workspace` 493 通过/0 失败；Tauri clippy PASS、`cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` 7 通过；`cargo test -p altior-protocol --features dto-export` 144 通过；`npm.cmd --prefix apps/desktop run gate` 全 7 阶段 PASS（205/24/5 全 0 diff/build）；`powershell -File scripts/quality-gate.ps1` exit 0「ALL PASSED CLEANLY」（Gate 4 真实 ACP 诚实 `[SKIPPED]`）。
- 诚实边界: 以上不构成真实第三方 ACP、签名安装包或生产同步的验收；mock/fixture、无头截图与构建成功不作为上述三项的验收证据。

### 下一步建议
1. B03：实现本地确定性回环 ACP 子进程桩，消除 Gate 4 `[SKIPPED]` 盲区。
2. P2 小项：诊断面板 Refresh 与 App 空态文案接入 i18n。
3. B05/B06：维持屏障，待外部依赖（同步安全闭环、签名证书）。

---

## 2026-09-23 自主会话执行切片（分支 `ui/toolcall-timeline`）

> 执行环境：Windows，cargo 1.98.0 / Node v24，无人值守自主会话。
> 基线：`5ba5fec`（分支 ui/toolcall-timeline）。工作树含大量**预先存在的**
> 未提交 UI 重设计变更（约 28 文件，另见 STATUS_AND_PLAN 第六节），本会话
> 按规则**不提交**、不混入；stash `parked-old-core-wip-do-not-mix-with-ui` 未触碰。
> 附带修复：HEAD 上 `cargo clippy -D warnings` 已不通过（clippy 1.98 新 lint），
> 以及 `cargo fmt --check` 漂移（来自提交 5ba5fec），均按门禁要求先行修复。

### 切片 5 — A18 残留：configure_agent 更新已有 profile 报存储错误（DONE-VERIFIED）
- Baseline: `5ba5fec` 工作树。
- 根因: daemon `handle_configure_agent` 每次都用 `created_at: now` 构造 profile/binding；
  storage upsert 以 `WHERE created_at = excluded.created_at` 保护不可变字段，
  更新路径必然 0 行受影响 → `InvalidEntityData: cannot modify immutable created_at`。
- 修复: 显式 `agent_profile_id` 命中已有 profile 时保留其 `created_at`
  （`updated_at = created_at.max(now)`）；binding 同理按 binding_id 读取保留。
  显式 ID 语义保持为真 upsert（不存在则创建），与 p14 旅程一致。
- Changed: `crates/altior-core/src/application/daemon.rs`；
  新增 `crates/altior-core/tests/p34_configure_agent_update.rs`（2 用例）。
- 回归证明: 修复前两用例均以 `CONFIGURE_AGENT_FAILED: cannot modify immutable created_at`
  失败（实测），修复后 PASS；`InMemoryListener` 确定性回环，无 sleeps。

### 切片 6 — A18 残留：open_thread 吞掉 create_session 错误（DONE-VERIFIED）
- 根因: `CoreApplication::open_thread` 中 `let _ = create_session(...)` 丢弃失败原因，
  线程打开后 session 为空且无任何解释。
- 修复: 新增 `ThreadOpenResult.session_error: Option<String>`（512 字符有界），
  捕获 create_session 失败原因；离线可读不变量保持——open 本身不因 harness
  失败而失败。协议面 `ThreadSnapshotDto` 增加 `session_error`（additive 可选字段，
  serde default + skip_serializing_if none，兼容旧端），`request_snapshot` 的
  gap-recovery 投影恒为 None（语义：未尝试 session）。
- Changed: `application/mod.rs`、`application/mod_types.rs`、`runtime/adapters/acp.rs`（1c 同文件）、
  `crates/altior-protocol/src/dto.rs`、`application/daemon.rs`（open_thread 投影 + request_snapshot None）、
  `snapshot.rs`（测试构造）、TS 导出 `apps/desktop/src/ipc/dto/ThreadSnapshotDto.ts`（重新生成）。
- UI 说明: 本切片不接前端渲染——桌面 UI 正处于未提交的重设计中，渲染
  `session_error` 归 UI 重设计分支（已登记 STATUS_AND_PLAN 后续项）。
- 回归证明: 新用例 `open_thread_reports_session_failure_instead_of_swallowing_it`——
  不存在的程序路径 → open 成功 + `session_error` 非空 + supervisor 未就绪；
  修复前（临时还原 `let _ =`）该用例 FAIL（实测），修复后 PASS。

### 切片 7 — A18 残留：probe_binding 无诊断输出（DONE-VERIFIED）
- 根因: `AcpHarnessAdapter::probe_binding` 两处 `Err(_err)` 把 spawn/握手失败
  丢弃为 `diagnostics: None`，wire 层（daemon `test_harness_binding`）本就转发
  diagnostics 字段，缺口仅在适配器。
- 修复: 失败路径填 `BoundedDiagnosticsSummary::from_raw`（既有的脱敏+限长通道），
  内容含被探测的 program 路径与底层错误。
- Changed: `crates/altior-core/src/runtime/adapters/acp.rs`；
  p34 新增 `probe_binding_failure_carries_actionable_diagnostics`（断言 ok=false、
  diagnostics 含 program 名、返回 probed_binding_id）。
- 回归证明: 修复前（临时还原 None）该用例 FAIL（实测），修复后 PASS。

### 切片 8 — 批次 1 门禁 + 既有 clippy/fmt/陈旧迁移测试修复（DONE-VERIFIED）
- 门禁结果:
  - `cargo fmt --all` ✓（顺带修复 5ba5fec 遗留漂移：event_pump.rs、adapters/acp.rs、
    sync_gate.rs EOF、command.rs 双空行——均为机械性格式）。
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` ✓
    （修复 HEAD 上已存在的 3 处：`entity_ids.rs` 缺 `# Panics` 文档、
    `sync_gate.rs` 缺 `# Errors` 文档、`assert!(!SYNC_ENABLED)` 常量断言 lint
    ——改为编译期 `const _: () = assert!(...)`，A25 屏障语义不降级）。
  - `cargo test --workspace` **508 通过 / 0 失败**。
  - Tauri 桥接 crate `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` 14 通过。
- 附带修复（陈旧测试）: `altior-storage/tests/identity.rs` 的
  `migration_v6_to_v7_...` 断言 schema==7，但当前 latest 已是 8（v8 落地时未同步），
  属预先存在的失败。改名为 `migration_v6_to_latest_preserves_journal_and_memory`，
  断言 `latest_schema_version()`；`altior-storage` 窄导出该函数（`pub use migrations::latest_schema_version`），
  避免下次迁移再次踩陈旧断言。
- Status: 批次 1（A18 三残留）全部 DONE-VERIFIED。
- Next: B03 本地确定性 ACP 回环桩。

### 切片 9 — B03 本地确定性 ACP 回环 Smoke（DONE-VERIFIED）
- Baseline: 同上；分支 `ui/toolcall-timeline` 工作树。
- 设计: 复用既有真实子进程桩 `crates/altior-acp/src/bin/mock_acp_agent.rs`（真实
  OS 子进程 + stdio JSON-RPC v1，非 in-process mock 冒充）。将 opt-in 外部 smoke
  的驱动器原样提取为共享 harness `tests/common/mod.rs`（同一旅程，仅被测程序可换）：
  initialize 握手 → `negotiation::negotiate` 能力协商 → `session/new` → prompt
  （`PromptDelivery` not_sent→written→Confirmed）→ agent 请求经
  `AgentLifecycle` 状态机（permission 排队/取消即答 cancelled，其余 method-not-found）
  → `mapping::normalize_trace` 归一化断言 → kill & reap。
- 新增: `crates/altior-acp/tests/loopback_smoke.rs` 两个用例——
  ① `agent_a_full` 场景标准 smoke 旅程（resume=true、delta≥1、delivery=Confirmed）；
  ② `agent_b_minimal` 场景取消旅程（首个流活动后经
  `on_cancel_requested` 发 `session/cancel`，stub 阻塞等待后以
  `stopReason: cancelled` 结算，delivery=Confirmed）。
- 确定性: stub 脚本化响应；watchdog 由 `sleep` 改为 channel `recv_timeout`
  ——通过路径立即退出（全量 0.10s），仅挂死时触发 kill；无网络、无时序依赖。
- 边界: ①不改变 opt-in 外部 smoke 语义（`smoke.rs` 行为保持，仅复用 harness）；
  ②不声称覆盖真实第三方 agent 行为/真实模型输出——Gate 4 的外部 opt-in 分支保留；
  ③permission 流在 AcpRuntime 层已由 `process_adapter.rs` 覆盖，loopback 不重复。
- Changed: `crates/altior-acp/tests/common/mod.rs`（新增）、
  `crates/altior-acp/tests/loopback_smoke.rs`（新增）、
  `crates/altior-acp/tests/smoke.rs`（改为复用 common，行为不变）、
  `scripts/quality-gate.ps1`（Gate 4 改名 "ACP Loopback + Real Opt-in"：
  loopback_smoke 必跑；外部 smoke 保持 opt-in，SKIPPED 文案更新为指向 loopback 覆盖）。
- 验证: `cargo test -p altior-acp` 全绿（loopback 2 通过 0.10s）；clippy -D warnings 0 警告；
  fmt check 通过。Gate 4 不再出现无条件 `[SKIPPED]` 盲区。
- Status: DONE-VERIFIED。
- Next: 批次 3 小项（Refresh 按钮 / App 空态 i18n）。

### 切片 10 — 批次 3：Refresh 按钮与 App 空态 i18n（DONE-VERIFIED，无需新工作）
- 核验结果: 两项均已在 `ui/toolcall-timeline` 未提交的 UI 重设计分支中完成——
  `shell.tsx` 的 Refresh 按钮走 `t.common.refresh`；App 四态空态文案走
  `workbenchEmpty.connecting / coreDisconnected / noConversations / selectConversation`；
  en.ts / zh-CN.ts 字典条目齐全（含 zh: 「刷新」等）。
- 验证: `p27ShellAriaOnboardingMemoryI18nEvidence.test.tsx` 5 用例 PASS（vitest 从
  apps/desktop 运行）。
- Status: DONE-VERIFIED（由既有分支工作覆盖，本会话无新增变更）。
- Next: 批次 4——全量门禁。

### 切片 11 — 批次 4：统一质量门禁收口（DONE-VERIFIED）
- 命令与结果（2026-09-23，`powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1`，
  本机执行策略需 Bypass 前缀）：
  - Stage 1 Rust: `cargo fmt --all -- --check` PASS；`cargo clippy --all-targets
    --all-features -- -D warnings` PASS；`cargo test --workspace` **508 通过 / 0 失败**
    （含本会话新增 p34 4 用例 + loopback 2 用例）。
  - Stage 2 Tauri: clippy PASS；14 测试通过。
  - Stage 3 Desktop: typecheck/lint/format PASS；Vitest **37 文件 / 289 用例** PASS；
    对比度 **24/24**；视觉基线 **6/6 全 0 diff**（light/dark/narrow/error/approval/memory）；
    vite build 成功（JS 378.69 kB / gzip 110.79 kB）。
  - Stage 4 ACP: `loopback_smoke` **2 通过（0.11s，真实子进程）** 必跑通过；
    外部真实 ACP smoke 保持 opt-in（未设 ALTIOR_ACP_SMOKE_AGENTS，诚实 SKIPPED）。
  - 最终横幅: ALL ALTIOR QUALITY GATES PASSED CLEANLY，exit 0。
- 未提交状态: 按执行规则，本会话全部变更（A18 三修复 + B03 回环 + 门禁脚本 +
  检查点/规划文档）保留在工作树，未创建任何 commit。分支
  `ui/toolcall-timeline` 上的 UI 重设计变更原样保留；stash 未触碰。
- Status: DONE-VERIFIED。
- Next: 用户回来后建议审阅工作树并决定提交切分（见最终报告）。

---

## 2026-09-24 续跑切片（同一分支，继续无人值守）

### 切片 12 — 重设计变更审查 + session_error UI 落地（DONE-VERIFIED）
- 审查结论（批次 4 第一项）: 逐文件复查了 `ui/toolcall-timeline` 未提交的
  UI 重设计 diff（MemoryPane 表格化含 roving tabindex/选择清理/错误横幅、
  shell/App 内联样式→令牌类、Timeline 去气泡化、ContextPanel/SafeMarkdown/
  localizeEnums/tokens、视觉门禁新增 memory 视图）。未发现功能性 bug；
  i18n en/zh 键位脚本核对 **336/336 完全对齐**。
- 残留风险（登记，不属本切片修复范围）: 记忆时间列 `toLocaleString` 的
  时区/区域依赖使视觉基线绑定捕获机器的 locale（既有问题的延伸——旧代码
  已用 `toLocaleTimeString`）。跨时区机器重跑像素门禁会 diff。后续可在
  Playwright context 固定 `timezoneId` 并重建基线。
- 核心层吞错扫描（批次 4 第二项）: 复查 crates 内全部 `let _ =`——剩余均为
  合理 teardown/best-effort（transport kill/reap、connection close、RAII
  secret 删除、已传播 `?` 后仅弃 Ok 值），无新修复点。
- **顺带修复（HEAD 上已提交的编码损坏）**: `en.ts` composer.cancelFailed 的
  `stop ? the` → `stop — the`（em-dash 被此前会话编码事故吞成 `?`）；
  测试仅断言 `/Could not confirm stop/i` 前缀，不受影响。
- **session_error UI 落地**（登记的后续项，数据面已由切片 6 下发）:
  - `ThreadSummaryView.sessionError?: string | null`；`applyThreadSnapshot`
    每次以最新快照刷新该字段（会话恢复后自然清除）。
  - App 在 Composer 上方渲染 `role="status"` 通知
    （`data-testid="session-error-notice"`），线程仍可读历史。
  - i18n 三件套新增 `composer.sessionUnavailable(reason)`（en/zh）。
  - 测试: store 级（快照携带/不携带 session_error 两态）+ App 级
    （通知渲染含原因），Vitest 291 通过。
- 门禁: npm gate 全绿（typecheck/lint/format PASS、291 用例、对比度 24/24、
  视觉 6/6 全 0 diff、build 成功）；统一门禁复跑见切片 13。
- Status: DONE-VERIFIED。
- Next: 待用户审阅工作树并决定提交切分。

### 切片 13 — 统一门禁最终复跑（DONE-VERIFIED）
- 命令: `powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1`（2026-09-24）。
- 结果: exit 0，**ALL ALTIOR QUALITY GATES PASSED CLEANLY**——
  fmt/clippy PASS；`cargo test --workspace` 全绿；Tauri 全绿；npm gate 全绿
  （Vitest 291、对比度 24/24、视觉 6/6 全 0 diff、build 成功）；
  loopback smoke 2 用例通过；外部真实 ACP 保持诚实 opt-in SKIPPED。
- Status: DONE-VERIFIED。
- 本会话全部工作保持未提交，等待用户审阅。

---

## 2026-09-24 第二轮续跑切片

### 切片 14 — 门禁加固：TS DTO 漂移检测（DONE-VERIFIED）
- 动机: `cargo test --workspace` 不带 `--all-features`，`dto-export` 导出测试
  不在默认门禁内——Rust DTO 定义变更而 TS 导出未重新生成时，门禁会放行
  （前端对过期类型编译）。这是 A01 手工验证步骤（导出后核对 `git status
  dto/` 为空）的门禁化。
- 实现: `scripts/quality-gate.ps1` Stage 1 新增——运行
  `cargo test -p altior-protocol --features dto-export`，前后对比 dto 目录
  全部 .ts 的内容（忽略 CRLF），有漂移即 fail 并提示重新生成+提交；通过后
  自动 `git checkout --` 还原纯行尾噪声文件（保留真实内容变更）。
- 验证:
  - 正方向: 当前树（含未提交但正确的 ThreadSnapshotDto.ts）→ "TS DTO exports
    are in sync with Rust definitions"，真实变更保留。
  - 负方向: 临时篡改 `TurnDto.ts` 字段名 → "DRIFT-DETECTED: TurnDto.ts" exit 1，
    还原后恢复。
  - 全量门禁复跑 exit 0（ALL ALTIOR QUALITY GATES PASSED CLEANLY），脚本
    PowerShell 语法解析 PASS。
- 已知外观现象（无需处理）: dto 目录 ~69 个文件在 `git status` 显示 M，但
  `git diff`（内容级）只有真实变更——这是 autocrlf 重整化的 stat 噪声，
  提交时自动消失。
- Status: DONE-VERIFIED。
- Next: 待用户审阅工作树并决定提交切分。

---

## 2026-09-25 第三轮续跑切片

### 切片 15 — 视觉门禁时区/区域确定性：Playwright context 钉定 locale+timezoneId（DONE-VERIFIED）
- Task: 切片 12 登记的残留风险收口——MemoryPane 时间列与检查器记忆日期用
  `toLocaleString`（默认 locale + 机器时区），视觉基线绑定了捕获机器的
  区域设置，跨时区机器重跑像素门禁必然 diff。
- 根因实测: 探针（临时脚本，已删）对同一时间戳 `1700000000000`——
  UTC/en-US 渲染 `11/14, 10:13 PM`，Asia/Shanghai/en-US 渲染
  `11/15, 06:13 AM`（日期都不同）；且本机 Chromium 默认
  `navigator.language = zh-CN`，应用 `localeSource="system"` 依此选 UI 语言，
  故钉 locale 同时是钉 UI 语言。
- 决策: 产品代码渲染本地时间是对的，确定性修在测试夹具层——两个视觉脚本
  的 `browser.newContext` 统一 spread `VISUAL_CONTEXT_DEFAULTS =
  { locale: "zh-CN", timezoneId: "UTC" }`。钉 zh-CN 而非 en-US：既有基线
  一贯是 zh 机器捕获（换 en 会使全部 6 视图全版 diff，实测确认），且保留
  CJK 排版覆盖。
- Changed: `apps/desktop/scripts/check-visual.mjs`（导出常量+spread）、
  `apps/desktop/scripts/baselines.mjs`（同值常量+spread，注释互指）；
  `apps/desktop/baselines/*.png` 重捕获。基线渲染差异审查（临时 pixelmatch
  脚本+diff 图，已删）: light/dark/narrow/error/approval **0 diff**；memory
  仅 159 px，全部落在表格「更新时间」列与检查器「更新时间」字段两个时间
  字符串（`11/15 06:13`→`11/14 22:13`、`2023/11/15 06:13:20`→
  `2023/11/14 22:13:20`），符合预期且仅此而已。
- 门禁: `npm run gate` 全绿（视觉 6/6 全 0 diff、build 成功）；
  `powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1`
  exit 0（ALL ALTIOR QUALITY GATES PASSED CLEANLY；loopback 2 用例 0.13s）。
- Rust 计数勘误: `cargo test --workspace` 本日实测 **510 通过 / 0 失败**
  （逐行汇总全部 `test result`）。较切片 8/11 记录的 508 有 +2，为上次会话
  计数口径差异（本会话零 Rust 变更；loopback 实为 2 用例已复核），以实测为准。
- Status: DONE-VERIFIED。切片 12 残留风险就此关闭。
- Next: 待用户审阅工作树并决定提交切分（变更仍在积累，建议尽早）。

---

## 2026-09-26 第四轮续跑切片

### 切片 16 — 批次 4 深度复查：7 项修复落地 + 1 项发现证伪（DONE-VERIFIED）
- 执行时间/环境: 2026-09-26 晚，Windows 本机（zh-CN 机器），`ui/toolcall-timeline`
  未提交工作树（基线 = 切片 15 全绿状态 + 用户当日新增的未跟踪 task-pack 工具
  `docs/reviews/2026-09-26/prompts/`、`scripts/task-pack/`、
  `docs/AUTONOMOUS_SESSION_PROMPT.md`，未触碰）。
- 任务: 批次 4 开放项「如发现其他 bug 或改进点，按项目规范修复」。独立审查
  agent 对未提交 diff 做第二遍功能性复查（聚焦测试覆盖不到的 React 正确性、
  store 竞态、Rust 数据路径、i18n 漏网），返回 8 项发现；逐一到代码核实后
  修复 7 项、证伪 1 项（详见下）。
- 修复清单（全部当场核实为真）:
  1. **[高] `setAgentMemoryMode` 截断多凭证绑定**（applicationStore.ts:1519）:
     更新绑定发送 `env_keys` 全量但 `secret_refs` 只发第一个，Core
     `upsert_harness_binding` 整体覆盖后，第二个及之后的映射丢失凭据。改为
     优先发送 `agent.secretRefs` 全量（缺省回退单数 `secretRef`）。
  2. **[中] 恢复路径违反 ADR 0020 顺序**（applicationStore.ts
     `triggerSnapshotRecovery`）: 先 `request_snapshot` 后 `get_history`，
     快照的 pending permission 行会插到恢复出的更早历史行之上。改为与
     `openThread` 一致的先历史后快照（快照 pending 行按 getRow 去重 no-op）。
  3. **[中] Inspector 标签箭头键不动焦点**（shell.tsx 检查器 tablist）:
     有 roving `tabIndex` 但 `onKeyDown` 只 `setActiveTab`，焦点留在变 -1
     的按钮上，下一次 Tab 直接逃出 tablist。补 `document.getElementById(...)
     ?.focus()`（与 Timeline 做法一致）。
  4. **[低] TimelineRowView y/d/n 快捷键无 target 守卫**: 焦点在行内
     Deny 按钮上按 "y" 会冒泡成批准。补
     `event.target !== event.currentTarget` 早退（与 MemoryPane 一致）。
  5. **[低] 记忆模式切换失败静默**（MemoryPane.tsx）: select onChange 直调
     `onSetAgentMemoryMode` 未捕获，rejection 无提示。改经现有
     `runMemoryOperation` 包装，失败进共享操作错误横幅；select 因 store
     仅在成功后更新而自动回位。
  6. **[低] 头部 IPC 状态药丸硬编码英文**（App.tsx）: "IPC connecting…" 与
     `IPC (${status})` 绕过字典。新增 typed keys `statusBar.ipcConnecting`
     / `ipcState(state)`（en/zh 同步），新增
     `localizeTransportStatus`（localizeEnums.ts，复用既有
     `statusBar.core*` 词表 + 新键 `coreClosed`）；"IPC v{n}" 为协议行话
     保持原样（zh 字典既有先例保留 IPC 不译）。
  7. **[低中] `sessionError` 被未尝试会话的路径误清除**:
     `summaryToView` 不带 sessionError，reconnect/recovery 的列表重建会抹掉
     横幅；`request_snapshot` 的快照恒为 `session_error: None`（daemon 注释
     即"未尝试会话"），apply 后同样清横幅。修复两处语义:
     summaryToView 从缓存视图携带既有错误（条件展开、不引入 `null` 键避免
     破坏既有 toEqual）；`applyThreadSnapshot` 增加
     `options.sessionAttempted`，仅 `openThread` 传 true——只有真正尝试过
     会话的快照才有资格置/清该字段。横幅现在只在成功的 open_thread 后清除。
- 证伪记录（重要，防后续会话再"修"）: 审查 agent 报告的「App 全局搜索回显
  effect 会把新输入回退并杀掉 pending 派发」（App.tsx:88-94、shell.tsx
  ThreadsPane 同型）经推演**不可达**: `setThreadFilter` 在 dispatch 当刻同步
  更新 store，React 在下一个用户输入任务之前完成 commit + effect，"echo 落在
  更新按键之后"的窗口实际不存在；审查员给出的时序示例中 echo 所对应的
  dispatch 已被去抖清除，根本不会发出。原 effect 的无条件重同步是"外部写入
  优先"的既定语义。本会话曾实现 lastDispatchedRef 修复后主动回退——该修复
  反而引入"外部写入值恰等于上次自派值时跳过重同步"的新边角。
- Changed（本切片新增改动）: `apps/desktop/src/stores/applicationStore.ts`、
  `apps/desktop/src/components/shell.tsx`、`apps/desktop/src/app/App.tsx`、
  `apps/desktop/src/components/MemoryPane.tsx`、
  `apps/desktop/src/features/timeline/TimelineRowView.tsx`、
  `apps/desktop/src/i18n/{types,en,zh-CN}.ts`、
  `apps/desktop/src/i18n/localizeEnums.ts`；
  测试: `applicationStore.test.ts`（+3: sessionError 生命周期含
  reconnect/stream.gap 两路径与干净 open 清除、恢复顺序 get_history 先于
  request_snapshot、多凭证 secret_refs 保真）、`TimelineRedesign.test.tsx`
  （+1 行内控件按键守卫）、`p27…Evidence.test.tsx`（+1 检查器标签焦点移动）、
  `p24…Evidence.test.tsx`（+1 记忆模式失败横幅 + select 回位）、
  `p26…Evidence.test.tsx`（+1 药丸字典组合 localizeTransportStatus 断言）。
  全部保持未提交。
- 失败/取消/离线/重启: 修复 2 直接关乎 Core 重启/stream.gap 恢复路径的行序;
  修复 7 关乎重连后错误解释的存续; 其余不改变失败语义仅补提示与焦点。
- 安全/同步影响: 无密码学/同步面变化; 修复 1 属凭据引用完整性（少发引用使
  启动缺凭据），修复 5 的错误横幅继续不透传传输层细节（测试断言 canary 不出现）。
- 迁移/降级: 无持久格式变化。
- 回归证明: 修复前 291 Vitest 全绿（基线复核）；修复后
  `npm run typecheck` 干净、37 文件/298 用例全绿（7 新增全过）、对比度
  24/24、视觉基线 6/6 全 0 diff（修复不影响基线渲染状态）、vite build 成功。
- 精确验证命令与结果（2026-09-26）:
  `powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1` →
  exit 0，**ALL ALTIOR QUALITY GATES PASSED CLEANLY**（fmt/clippy PASS；
  `cargo test --workspace` 全绿——本会话零 Rust 改动，计数与切片 15 实测
  510 基数一致；Tauri 全绿；npm gate 全绿 298 用例；loopback smoke 2 用例
  0.12s；外部真实 ACP 保持诚实 opt-in SKIPPED）。
  窄测试: `npx vitest run`（5 个触碰文件 75 用例）全过。
- 实际查验截图: 视觉基线 6 视图 pixel diff 0（门禁输出）；本切片无新 UI
  状态需要重捕基线（错误横幅/焦点环不在基线视图中触发）。
- 未运行/阻塞/残留: 修复 3/4 的键盘行为仅由 jsdom 测试覆盖（jsdom 无真实
  焦点环渲染），建议用户审阅时在真机上走一遍 Tab/箭头键; 检查器 tablist 与
  StatusBar 各有一份状态词 switch（未抽公共 helpers 以避免改变 StatusBar
  既有 "?" 占位渲染，已在此登记）。
- Status: DONE-VERIFIED。
- Next: 待用户审阅工作树并决定提交切分；「七、待做」中剩余项均为外部阻塞
  （真实第三方 ACP 凭证）或依赖分支合并决策（UI 重设计后续阶段）。

---

## 2026-10-16 第五轮切片（授权提交 + P3 启动，分支 feat/p3-sync-foundation）

### 切片 17 — v0.1 工作树落账：7 个 Conventional Commits（DONE-VERIFIED）
- 用户明确授权提交切分。切片 16 及之前 4 个会话的全部未提交变更（约 150 文件）
  按层切分提交至 `ui/toolcall-timeline`（未 push）：
  `d3cab2e` feat(protocol) session_error 契约 → `63c34fe` fix(core) A18 三修复
  → `724fbc8` feat(acp) 回环桩 → `52e11e5` chore(gate) 门禁脚本 →
  `b9e349b` feat(desktop) UI 重设计大块 → `9606d62` docs → `69e58aa`
  chore(task-pack) 用户当日工具。CRLF 噪声文件在提交时自动坍缩为空 diff，
  只有真实内容入库。提交前秘密扫描（tight 模式）干净。工作树清零，stash 未动。
- Status: DONE-VERIFIED。

### 切片 18 — 工具链漂移修复：clippy 1.99 + 门禁脚本 UTF-8（DONE-VERIFIED）
- **clippy 1.99**（`fd66247`）: rustc 1.99.0 将 `clippy::assert_is_empty` 收入
  pedantic（三个 crate 的 Cargo.toml 自选 pedantic=warn，被门禁 `-D warnings`
  升为错误）。此前门禁"绿"依赖 clippy 缓存（切片 16 零 Rust 改动），本次
  commit 触碰文件使缓存失效而暴露——登记为工具链漂移教训：跨会话的"绿"
  必须在缓存失效路径下复验。修复 29 处（21 正形式 `assert_eq!(x, [])`、
  8 负形式 `assert_ne!`），机械变换，断言语义不变。
- **门禁脚本 UTF-8**（`be87bac`）: `Get-DtoContentMap` 未指定编码，Windows
  PowerShell 5.1 按 ANSI(GBK) 解码无 BOM 文件，多字节字符吞掉相邻 CR 使
  CRLF 归一化失效，干净 checkout 上全量 dto 误报漂移。补 `-Encoding UTF8`。
- 分支: `feat/p3-sync-foundation`（自 ui/toolcall-timeline 栈上切出，避免
  storage 冲突；两分支按序合并）。落账后统一门禁复跑 exit 0。
- Status: DONE-VERIFIED。

### 切片 19 — ADR 0027：P3 同步协议设计（DONE-VERIFIED，`aff68a5`）
- 逐条映射 ADR 0025 的 11 项威胁 → 机制 → crate → 验收测试；三大层：
  会话加固（epoch + 计数器租约 + turn ratchet + 零共享密钥检查）、设备生命周期
  （登记/撤销证书/密钥轮换）、同步引擎（pull-before-push + ack 前沿 + 墓碑
  不可回收）。信封 v2 新版本不重写 v1；sync_gate 屏障保持关闭。
- Status: DONE-VERIFIED。

### 切片 20 — P3.2 加密会话加固 + ADR 0025 套件 2（DONE-VERIFIED，`f2e36d7`）
- `SessionV2`（session_v2.rs，信封 v2）: 128-bit `SessionEpoch`（HKDF info +
  AAD 绑定）；`CounterLease` 租约端口（计数器块 [start,end) 耗尽即停，安装
  回退块/零起点块 typed 拒绝——F31 修复）；turn ratchet（每轮换批新 X25519
  ephemeral 链入 HKDF-Extract，静态密钥泄露不回溯历史轮）；epoch_seq 单调
  回滚拒绝；全零共享密钥拒绝（威胁 2）。
- **两阶段提交**（测试驱出的真修复）: 接收端 turn 推进原实现在 AEAD 之前用
  未认证的 turn 头改链状态；改为"候选链 → AEAD → 认证通过才提交"，伪造/损坏
  turn 头不再毒化接收端（ADR 0011 "never act on unauthenticated counters"）。
- 计数器从 1 起（租约零起点拒绝），与 ReplayWindow 的 0-不接受规则对齐。
- **套件 2** `test_session_reconstruction_nonce_uniqueness`: 1000 次进程杀死
  模拟重建 × 100 信封 = 100,000 信封，计数器与线上 nonce 双集合零重复
  （9.02s）。测试文件另有 6 个行为测试（roundtrip/双 turn ratchet/单比特
  翻转全表/epoch 不匹配/turn 跳过/租约耗尽与回退/低序点/turn 卫生）。
- 门禁: `cargo test -p altior-crypto` 全绿（12 lib + 6 v2 + 1 套件 2），
  clippy -D warnings 干净。
- Status: DONE-VERIFIED。

### 切片 21 — P3.3 中继加固 + ADR 0025 套件 3（DONE-VERIFIED，`0687b08`）
- `RelayPolicy` 增 `max_push_id_bytes`（4 KiB 元数据帽）与 `max_bucket_bytes`
  （50 MiB 桶配额，背压不驱逐）+ `production()` 预设（14 天 tick 映射注记）。
  拒绝整体生效、桶不动。dedupe 有界性按 ADR 0012 既有结构（仅 retained 条目）
  维持并登记。`retained_frames()` 审计钩子（生产不调用）。
- **套件 3** `test_relay_zero_knowledge_audit`: 三轮 v2 会话（含中英记忆、
  secret 引用、恢复短语 6 个 canary）过中继，逐帧 + 全量字节两层面断言零
  明文。`policy_bounds` 测试覆盖四大限额与拒绝后桶不变性。
- 门禁: altior-relay 16 测试全绿，clippy 干净。
- Status: DONE-VERIFIED。

### 切片 22 — P3.4 Vault 领域契约（DONE-VERIFIED，`1c69a9e` + fmt `383686e`）
- `altior-domain/src/vault.rs`: `VaultDeviceRecord`（配对自配对拒绝、单向
  Active→Revoked、证书绑定记录设备与时间序、EpochSeq checked 递增）、
  `RevocationCertificate`/`RecoverySignature`（不透明有界字节，验证归 crypto）、
  `MemoryTombstone`+`TombstoneDigest`（全零保留值拒绝）、`AckFrontier`
  （compaction_floor = 已记录设备游标最小值；无记录 → None 钉死压缩；回滚
  声明不降前沿）。纯规则无依赖，入库 7 个内联测试。
- 计数器租约的 SQLite 实现与存储迁移**顺延至切片 23**（P3.5 一起做）。
- **统一门禁**（2026-10-16，`powershell -ExecutionPolicy Bypass -File
  scripts/quality-gate.ps1`）: exit 0，ALL ALTIOR QUALITY GATES PASSED
  CLEANLY——Rust+Tauri 690 测试、Vitest 298、对比度 24/24、视觉 6/6 全
  0 diff、DTO 漂移 in sync、loopback 2 用例。所有 P3 提交保持不 push。
- Status: DONE-VERIFIED。

### 切片 23 — P3.5 同步引擎落地 + ADR 0025 套件 1/4 全绿（DONE-VERIFIED）
- **crypto 事件签名**（并入 f9d1a12）: `DeviceKeys::sign` +
  `DeviceKeys::verify_signature`（verify_strict，配对签名的一般化）。
- **storage v9**（`f9d1a12`）: 迁移 v9 四表（sync_counter_lease /
  vault_device / sync_ack_cursor / memory_tombstone，全部在 sync gate 后）+
  `SqliteCounterLease`（事务化租约，F31 持久半场）+ 3 测试（重开后不回绕、
  500 信封跨重建零重复、设备表往返）。版本钉定测试 8→9（journal/domain/
  memory/p14 四文件）。
- **同步引擎**（`93d264f`）: `altior-core::sync`——Ed25519 签名事件
  （event_id 去重）、文档 grow-only 条目集、记忆 propose/forget + 永久墓碑、
  pull-before-push flush。测试驱出两个真修复: ① poll 在 has_more=false 时
  先 break 后处理，丢掉最后一页还推进游标（已修：处理完再断）；② 空 outbox
  的 flush 也推进 ratchet turn，接收端正确地按跳号拒绝（已修：turn=批次，
  空批不换轮）。引擎内简化（grow-only vs Automerge、outbox 不修剪、内存态）
  均在模块文档具名登记。
- **套件 1** `test_three_device_offline_convergence`（p36）: 三设备配对，
  C 离线期间 A/B 并发编辑 + 遗忘 M1 + 新增 M2，C 回归 pull-before-push 后
  贡献 M3——三方 digest 全等，墓碑/双编辑/M2/M3 全员一致，零抑制。
- **套件 4** `test_stale_device_tombstone_non_resurrection`（p37）: C 带旧
  状态重连并重提议 M1——pull 先应用墓碑，peer 各抑制 1 次，新鲜 M4 正常
  通过；三方 digest 全等。
- **边界与门禁**: 依赖边界钉定测试按 ADR 0027 更新（core +crypto+relay，
  storage +crypto，dc89004）。dto-export 写入遇 Windows ERROR 1224（索引器
  映射瞬态）加有界重试（8b64d52，~500ms 上限，其余错误照旧诚实失败）。
  统一门禁 **exit 0，ALL ALTIOR QUALITY GATES PASSED CLEANLY**——Rust+Tauri
  **695** 测试、Vitest 298、对比度 24/24、视觉 6/6 全 0 diff、DTO in sync、
  loopback 2 用例。ADR 0025 四套件状态: **1 ✅ 2 ✅ 3 ✅ 4 ✅**（独立密码学
  审查与 SYNC_ENABLED 翻转仍按 ADR 0025 保留给用户）。
- Status: DONE-VERIFIED。
- Next: P3 剩余为产品接线（设备配对 UI、同步诊断面板、Desktop 会话）与
  blob 平面；均等用户决策后另起切片。


### 切片 24 — 威胁 10 闭环：ack 回流 + 墓碑压缩门控（DONE-VERIFIED，2026-10-17）
- **引擎 half**（同提交）: `EventBody::Ack`——每个 flush 广播本节点的
  per-origin 已应用前沿（普通签名日志事件，去重/加密/换轮一路复用）；节点
  折叠为 per-peer 台账，`gc_frontiers()` = 对每个 origin 取**全体直连 peer**
  的最小值，无 ack 台账的 peer 按 0 计（成员变化自动重钉门控，与领域
  `AckFrontier` 语义一致）。墓碑标记升级为 `TombstoneMark`
  （digest + forget 事件的 origin/origin_seq）。
- **storage half**: `vault_device` CRUD（domain 新增 `restore()` 构造器，
  撤销状态可跨持久化重建）；墓碑行携带 origin/origin_seq（v9 未发布，
  同分支原地扩展）；`observe_ack_cursor` 只进不退（回滚声明不是 ack）；
  `compact_tombstones(floors)` 只删严格低于该 origin ack 地板的行——
  测试证明：地板之下保留、覆盖地板回收、无地板钉死。
- **p38** `ack_frontiers_gate_tombstone_compaction`: 遗忘分发前 A 的门控
  低于 forget seq（关）；两轮 settle 后三节点门控全部 ≥ forget seq（开）；
  新配对未 ack 的 peer 把门控钉回 0。三方无复活、digest 一致。
- 门禁: exit 0，ALL ALTIOR QUALITY GATES PASSED CLEANLY——Rust+Tauri **699**
  测试（+4）、Vitest 298、对比度 24/24、视觉 6/6 全 0 diff。存储 121 测试。
- Status: DONE-VERIFIED。
- Next: 引擎态持久化接线（applied 日志/outbox 走 v9 表）与 blob 平面为
  下两个工程切片；产品层（配对 UI/诊断面板）等用户决策。


### 切片 25 — 引擎态持久化接线：重启安全的 SyncNode（DONE-VERIFIED，2026-10-17）
- **SyncStatePort 端口**（sync/persist.rs）: applied 日志/fetch 游标/墓碑三类
  持久化 + `MemorySyncStatePort`（进程内默认，行为与原引擎等价）+
  `StoreSyncStatePort`（走 storage 类型化 API，引擎零 SQL）。`SyncNode::durable`
  构造时载入日志并经正常 apply 路径重放，重建 memories/documents/tombstones/
  peer_acks 全部投影；origin 序列从重放日志推导（永不回绕——重启后 event id
  与中继历史撞车由结构上排除）。
- **v9 扩表**（未发布同分支原地扩展）: sync_journal（applied_order 单调）+
  sync_fetch_cursor；storage 增类型无关 API（append/list journal、fetch 游标
  只进不退）。
- **测试驱出的三个真修复**:
  ① poll 最后一页先 break 后处理（丢页+游标虚进）——切片 23 已修，本切片
  回归覆盖；
  ② flush 的 ack 事件走 apply 不走 admit——从未进日志，重启后 origin seq
  回退与中继撞车（p39 捕获，已修：ack 入日志）；
  ③ 重配对后新旧配对信封同桶、turn 号重叠但密钥链不同——AEAD 必败。修复
  落在 ADR 0027 §4.4 本来的设计上：pair() 携带 epoch_seq（重配对递增），
  poll 对**更旧 epoch** 的桶内残留信封跳过（内容在重配对前已应用），更新
  epoch 或认证失败一律 fail closed。三套件同步改造（pair 携带 1）。
- **p39** `durable_node_survives_restart_without_loss`: 重启后重放恢复记忆/
  文档/去重基；跨重启继续收敛，digest 一致，无复活无重复。
- 门禁: exit 0，ALL ALTIOR QUALITY GATES PASSED CLEANLY——Rust+Tauri **700**
  测试、Vitest 298、对比度 24/24、视觉 6/6 全 0 diff。core 127 测试、
  storage 121 测试。
- 已登记未做: 会话密钥材料（chain_ikm）的持久化属 OS Secret Store 职责
  （SQLite 存密钥违反 AGENTS.md 不变量），当前重启后经 pair() 重建会话；
  turn 链状态的 SecretStore 持久化留待配对流程切片。
- Status: DONE-VERIFIED。
- Next: blob 平面（内容寻址加密块，需先补 ADR 0027 附录或新 ADR）；产品层
  等用户决策。
