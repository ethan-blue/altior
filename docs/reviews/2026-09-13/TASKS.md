# 2026-09-13 审查后续任务规划包 / Actionable Task Pack (B01–B06)

本任务包是针对 2026-09-13 独立审查发现的问题所制定的工程修复与演进计划。各工作项遵循严格的依赖拓扑与验收标准。

> **2026-09-13 收口状态**：B01、B02、B04 已完成并通过独立复核与全量门禁复跑（`scripts/quality-gate.ps1` exit 0 全绿）；B03 未实施（真实外部 ACP 保持 opt-in 跳过，本地回环桩待做）；B05、B06 维持受控延期（外部依赖未解除）。各任务状态详见文末「执行状态记录」。

---

## 任务依赖拓扑

```text
[B01 门禁脚本与代码卫生] ──> [B02 操作系统密钥环接入] ──> [B03 真实 ACP 本地回环验证]
                                     │
[B04 桌面审计待定项修复 (Pending)] ───┘
                                     │
                                     ▼
                      [B05 P3 多设备同步安全前置基线]
                                     │
                                     ▼
                      [B06 P5 发布打包与代码签名]
```

---

## 任务规约明细

### B01 — 修复门禁脚本权限兼容性与清理无用测试桩（P2）

- **优先级**：P2
- **依赖**：无
- **目标**：解决 Windows PowerShell 环境下执行 `quality-gate.ps1` 时因 `npm.ps1` 策略报错的问题，并移除存储层空测试文件。
- **涉及文件**：
  - `scripts/quality-gate.ps1`
  - `crates/altior-storage/tests/relevance_eval.rs`
- **In Scope**：
  - 将 `scripts/quality-gate.ps1` 中的 `npm` 改为 `npm.cmd`（或判断平台后调用 `.cmd`），保证在默认受限执行策略下平稳运行。
  - 删除空文件 `crates/altior-storage/tests/relevance_eval.rs`（或补充基础的 FTS5 召回性能微基准）。
- **Out of Scope**：
  - 修改前端构建配置或影响任何测试逻辑。
- **验收标准**：
  - 在未放开脚本执行策略的原生 PowerShell 窗口中运行 `.\scripts\quality-gate.ps1`，脚本不抛权限异常，成功执行整套质量门禁。
  - `cargo test -p altior-storage` 无空目标报告。

---

### B02 — 接入操作系统原生凭证管理器（OS Secret Store）（P1）

- **优先级**：P1
- **依赖**：B01
- **目标**：打通 `SecretRef` 到真实操作系统 Keychain 的安全解析，使 ACP Harness 能够安全提取 API 凭证而无需明文配置文件。
- **涉及文件**：
  - `crates/altior-acp/src/config.rs`
  - `crates/altior-core/src/runtime/adapters/acp.rs`
  - `crates/altior-core/Cargo.toml`
- **In Scope**：
  - 引入经过安全审计的跨平台凭证存储库（如 `keyring`）或平台原生 API（Windows DPAPI/Credential Manager，macOS Security.framework）。
  - 实现 `SecretResolver` trait，为 `AcpHarnessAdapter` 注入真实的凭证解析器。
  - 确保解析错误时以标准 `AcpError::SecretResolutionFailed` 优雅 fail-closed 退出。
- **Out of Scope**：
  - 在数据库或文件系统中持久化任何明文凭证（严厉禁止）。
- **验收标准**：
  - 编写端到端单元测试：通过 OS 临时凭证存储写入测试 Canary，验证 `AcpHarnessAdapter` 成功解析该 SecretRef 并注入子进程环境变量，同时断言日志、崩溃转储和 SQLite 数据库中 0 Canary 泄漏。

---

### B03 — 提供真实 ACP 本地确定性回环 Smoke 门禁（P1/P2）

- **优先级**：P1 / P2
- **依赖**：B01
- **目标**：使 `quality-gate.ps1` 的 Gate 4（真实 ACP Opt-in 检查）在无公网 API 密钥与外部账号的情况下，也能自动化执行确定性的进程间端到端验证，消除 `[SKIPPED]` 盲区。
- **涉及文件**：
  - `scripts/quality-gate.ps1`
  - `crates/altior-acp/tests/smoke.rs`
- **In Scope**：
  - 在 `smoke.rs` 中提供一个不依赖公网外网的本地标准回环 Agent 可执行测试桩（或打包内置的轻量回环测试程序）。
  - 当检测到未配置公网商用代理时，自动回退到本地回环测试，断言子进程启动、流式传输、双向权限确认与优雅退出的完整过程。
- **Out of Scope**：
  - 降低安全要求或跳过真实子进程生成。
- **验收标准**：
  - 执行 `.\scripts\quality-gate.ps1` 时，Gate 4 报告为 `[PASS]`（经本地回环真实子进程验证通过）或明确标明商用模型跳过说明，整仓门禁全绿。

---

### B04 — 桌面 UI、Tauri IPC 专项审计问题修复（Pending）

- **优先级**：待定（Pending）
- **依赖**：B01
- **目标**：承接桌面专项审计员补发的精要报告，修复前端、Tauri 壳与真实 ACP 连续性相关的具体缺陷。
- **涉及文件**：
  - `apps/desktop/` 及其相关组件
  - `apps/desktop/src-tauri/`
- **说明**：当前处于占位状态。待收到桌面审计精要后，由后续协作任务补全具体行号、缺陷表现与修复切片，严禁提前臆造。

---

### B05 — 落实 ADR 0025 多设备同步安全前置基线（P0/P1）

- **优先级**：P0 屏障维持 / P1 研发前置
- **依赖**：B02
- **目标**：在解除 `sync_enabled = false` 之前，彻底消除 `altior-crypto` 中的 Nonce 复用漏洞并实现设备撤销与密钥轮换。
- **涉及文件**：
  - `crates/altior-crypto/src/session.rs`
  - `crates/altior-crypto/src/device.rs`
  - `crates/altior-relay/`
- **In Scope**：
  - 实现基于持久化计数块预留（Reservation Blocks）或 Double Ratchet 棘轮协议的会话管理，保证即使进程重启也绝对不发生 Nonce 复用。
  - 实现基于离线恢复密钥签署的设备撤销证书（Revocation Certificates）与数据密钥就地轮换。
  - 落实“先拉后推”（Pull-Before-Push）长离线设备回归协议与向量时钟墓碑防复活机制。
- **明确禁止项**：
  - 严禁在上述 11 项安全威胁完全闭环前将 `sync_enabled` 设为 `true`。
- **验收标准**：
  - ADR 0025 规定的 4 组确定性测试套件（含 1,000 次会话重启 100,000 条密文零重复 Nonce 断言）全部通过。

---

### B06 — 发布工程：Windows 签名、安装器与升级验证（P5）

- **优先级**：P2
- **依赖**：B01–B04
- **目标**：完成 Windows 原生安装包（MSI/NSIS）构建、Authenticode 代码签名、数据目录权限防护与无损升级验证。
- **涉及文件**：
  - `apps/desktop/src-tauri/tauri.conf.json`
  - `scripts/`
- **外部依赖 / 阻塞**：
  - 需要有效的 Windows 代码签名数字证书（硬件 Token 或 HSM）。
- **验收标准**：
  - 在全新、干净的 Windows 11 主机上安装并运行，无 SmartScreen 阻止拦截；升级时 Personal Vault 数据库完好保留。

---

## 执行状态记录（2026-09-13 收口）

| ID | 状态 | 执行证据摘要 |
|---|---|---|
| **B01** | ✅ 已完成并验证 | `quality-gate.ps1` 每步 `$LASTEXITCODE` fail-fast（失败注入实测非零退出）；npm 经 `cmd.exe /c` 调用规避 `npm.ps1` 策略拦截；空壳 `relevance_eval.rs` 已删除（真实检索测试在 `altior-core` p25）；`cargo test -p altior-storage` 通过。附带完成：`streamDeduplicator.record()` O(W) 全量遍历改为摊销 O(1)（百万事件结构化测试约 35ms）；`check-visual.mjs` 升级为真实 Playwright 截图 + pixelmatch 像素回归（阈值 0.1 / 上限 50 像素，正向 0 diff、扰动 2174 像素非零退出，基线仅显式 `npm run baselines` 可更新）。 |
| **B02** | ✅ 已完成并验证 | ADR 0026 + `crates/altior-core/src/secrets.rs`：Windows Credential Manager 生产驱动（`CredReadW/CredWriteW/CredDeleteW`，UTF-16、`CredFree` 全路径释放、删除幂等、`ERROR_NOT_FOUND`→`NotFound`）；`AcpHarnessAdapter::new()` 默认 `OsSecretStore`，封闭测试显式 `NoSecretsResolver`/`with_no_secrets()`；真实随机条目写入/读取/解析/删除 + RAII 清理测试通过；SecretRef 规范映射拒绝空值/控制字符/超 256 字节；macOS/Linux cfg 门控 fail-closed。第二路独立安全复核（任务 01a098b0）结论 PASS：fmt/clippy/secrets 5 项/acp 5 项测试全过，Debug/Error 无明文泄漏，解析单向，ADR 平台矩阵诚实。 |
| **B03** | ⏳ 未实施（pending） | 真实外部 ACP smoke 仍为 opt-in（`ALTIOR_ACP_SMOKE_AGENTS` 未设置时诚实 `[SKIPPED]`）；本地确定性回环子进程桩未开发。不得将 Mock/fixture 冒充为真实 ACP 验收。 |
| **B04** | ✅ 已完成并验证 | 六项修复：①Activity Rail 6 个语义化 inline SVG（aria-hidden + 本地化 aria-label/title，Projects=P4）；②线程搜索 280ms 可测试防抖 + IME 合成保护（合成态不发 IPC）；③Inspector/Settings `RuntimeDiagnosticsView` 四态脱敏展示；④移除 P0.1 `<details>` 调试区、恢复三行 Grid（协议证据保留为 display:none 测试钩子）；⑤TimelineRowView/ContextPanel/MemoryPane/shell 列举文本全量接入 i18n（Han 字符扫描零残留）；⑥Agents 生命周期按钮不造假（本地化 deferred 说明）。验证：Vitest 23 文件 205 用例、contrast 24/24、显式重建基线后 pixel diff 5/5 全 0、typecheck/lint/format/build 全过；接管方独立代码复核属实。残留 P2 小项：诊断 Refresh 按钮与 App 空态文案未走 i18n。 |
| **B05** | ⏸️ 受控延期 | `sync_enabled = false` 硬屏障维持；ADR 0025 十一项威胁闭环（Double Ratchet/Nonce 防复用/撤销/轮换）与四组验收套件未实施；中继基础设施未部署。 |
| **B06** | ⏸️ 受控延期 | Windows 签名证书（硬件 Token/HSM）未取得；MSI/NSIS 打包、Authenticode 签名、干净主机安装与升级验证未执行。 |
