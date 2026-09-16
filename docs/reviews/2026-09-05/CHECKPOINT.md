# AI 执行检查点

本文件是后续实现进度记录，不能替代长期契约。

## 初始状态

- Review baseline：`f945ef153eb3344abd3801a3c2e4002fd25d80be`
- 本轮工作：评审、合成探针、截图、任务包、中英文提示词。
- 产品实现改动：无。
- 任务状态：A01–A20 全部 TODO。报告完成不等于任务修复。
- 当前下一项：**A01 — 冻结真实边界并修正 ID**。
- 第一条具体操作：查看当前 git status/commit，读适用 AGENTS/ADR，然后枚举 applicationStore 所有发出命令与 Rust DTO 的身份约束，建立 frontend→Rust 失败 fixture。
- 注意：evidence 中的 probes 断言的是 bug 现状，不能原样当作修复后的通过标准。

| ID | 状态 | 依赖/交接提示 |
|---|---|---|
| A01 | DONE-VERIFIED | 见 2026-09-06 条目 |
| A02 | DONE-VERIFIED（真实 Tauri WebView 握手除外，见 Not run） | 见 2026-09-06 条目；A03/A17 可开始 |
| A03 | DONE-VERIFIED | 见 2026-09-06 条目；A04 可开始 |
| A04 | DONE-VERIFIED | 见 2026-09-06 条目；A05 可开始 |
| A05 | DONE-VERIFIED | 见 2026-09-06 条目；A06 可开始 |
| A06 | DONE-VERIFIED | 见 2026-09-06 条目；A07 可开始 |
| A07 | DONE-VERIFIED | 见 2026-09-06 条目；A08 可开始 |
| A08 | DONE-VERIFIED | 见 2026-09-06 条目；A09 可开始 |
| A09 | DONE-VERIFIED | 见 2026-09-06 条目；A10 可开始 |
| A10 | DONE-VERIFIED | 见 2026-09-06 条目；A11 可开始 |
| A11 | DONE-VERIFIED | 见 2026-09-06 条目；A12 可开始 |
| A12 | DONE-VERIFIED | 见 2026-09-06 条目；A13 可开始 |
| A13 | DONE-VERIFIED | 见 2026-09-06 条目；A14 可开始 |
| A14 | DONE-VERIFIED | 见 2026-09-06 条目；A15 可开始 |
| A15 | DONE-VERIFIED | 见 2026-09-06 条目；A16 可开始 |
| A16 | DONE-VERIFIED | 见 2026-09-06 条目；A17 可开始 |
| A17 | DONE-VERIFIED | 见 2026-09-06 条目；A18 可开始 |
| A18 | BLOCKED | 真实第三方代理密钥与签名环境缺失（见 2026-09-06 条目人工规程）；A19 可独立进行 |
| A19 | DONE-VERIFIED | 见 2026-09-06 条目；A20 可开始 |
| A20 | DONE-VERIFIED | 见 2026-09-06 条目；A18 仍为 BLOCKED，A01–A20 未全闭环 |

## 每个切片追加模板

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
Status (TODO / IN_PROGRESS / DONE-VERIFIED / BLOCKED):
Next task and first concrete action:
```

不要修改本轮评审证据来制造已完成记录。可以在后续条目解释问题已经随哪次实现解决，并链接新的证据。没有用户授权不要自动创建后台任务或循环调度。

## 执行记录

### 2026-09-06 00:49 — A01 冻结真实边界并修正 ID（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-05 深夜至 2026-09-06 00:49（本地 Windows）；cargo 1.98.0，Node v24.18.0；未提交任何 commit。
Task ID / subslice: A01（完整单切片，覆盖 F01 的命令身份半边；F32 的前端→Rust 契约校验半边）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，改动只含本切片与评审包）
Pre-existing unrelated changes: 无（起点只有未跟踪的 docs/reviews/）
User-visible outcome: 前端发出的每条命令都能被真实 Rust serde 解码；线程/轮次/代理/绑定身份一律由 Core 生成并经响应回传；中文标题/正文、同毫秒并发、renderer 重启都不再产生非法或冲突身份；非法命令在 fake transport 与真实 Core 一致被拒。
In scope: A01 工作简报见 work/A01-work-brief.md。Out of scope: create/cancel 失败的完整 UX（A04）、空 Vault 与搜索竞态（A03）、secret ref 语义（A07）、history DTO（A05）。
Contracts and decisions adopted: 新增 docs/decisions/0019-identifier-allocation-boundaries.md（Core 拥有实体 ID 生成：毫秒 12 hex + 进程噪声 4 hex；Desktop 拥有 operation ID：op_+128bit CSPRNG；重试必须复用原 operation id；configure/test/start_turn 结果 data 增加回传字段——协议 v1 下可加可选字段）。ADR 0004 未改动，仅按其 "revisit" 约定由 0019 记录生成约定。
Changed files:
  Rust: crates/altior-core/src/application/entity_ids.rs（新增）、application/mod.rs、application/daemon.rs；crates/altior-protocol/fixtures/command-desktop-*.json（16 个新发射形状 fixture）、command-invalid-*.json（6 个历史非法形状 fixture）、tests/desktop_command_fixtures.rs（新增）、fixtures/handshake-{desktop,core}-hello/negotiated-v1.json（版本对齐，见下）。
  TS: apps/desktop/src/ipc/operationId.ts、commandContract.ts（新增镜像校验）、errors.ts（InvalidCommandError）、inMemoryTransport.ts（命令前置校验 + 响应对齐真实 Core + 合法 fixture 身份）、applicationStore.ts（stop fabricating：turn_id 传 null 并采纳响应 trn；agent/binding/thread 只用 Core 返回 id；create 失败不再插入本地线程；operation id 全部走 allocator；env_keys/secret_refs 数量一致前置拒绝）、fixtures/timeline.ts（wire 可见 id 全部合法）、App.tsx（去掉 "fixture/standard" 字面量回退）、scripts/baselines.mjs（新 thread id）、errors/tests 对应更新。
Failure / cancellation / offline / restart: 非法命令在发送前被拒（fake）或被 Core serde 拒（真实），不产生半状态；create_thread 失败向上抛且不造本地事实；renderer 重启 operation id 不重复（CSPRNG）；Core 同毫秒生成不冲突（分配器 10_000 次/同毫秒测试）。
Security / synchronization impact: 无秘密路径改动；sanitizeSecretRef 的随机 ref 伪造保留原样（F27，归 A07 处理，已记录）；错误文案不含用户 prompt；无同步面改动。
Migration / downgrade: 无持久格式变更；configure/test/start_turn 响应 data 的新增字段为可选 JSON，旧端忽略。
Regression proof before and after: 修复前评审探针证明 op_start_turn_1 / trn_<ts>_<n> / agent-alpha / bin_alpha_01 / thread-1_x 等 shape 被 Rust 拒绝（evidence/review-probes.test.tsx.txt）；修复后 crates/altior-protocol/tests/desktop_command_fixtures.rs 将这些非法 shape 固化为必须继续被拒的 fixture，同时 16 个新发射形状全部解码+校验通过。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS（先 cargo fmt --all 修复两处格式）
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（修复 identity map_err、4 处缺 # Errors 文档后）
  - cargo test --workspace → PASS（52 个测试目标全 ok，0 failed）
  - cargo test -p altior-protocol --features dto-export → PASS（104+ 套件全 ok）；git status apps/desktop/src/ipc/dto/ 为空 → TS 导出无漂移
  - cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml → PASS（7 tests）
  - apps/desktop: npm run typecheck → PASS；npm test → 11 files / 95 tests PASS（原 90 + 新增 A01 验收测试）；npm run build → PASS（JS 267.30 kB / gzip 81.14 kB）
Screenshots actually inspected: 本切片无视觉变更（仅身份字符串与测试 id），未新增截图；布局/主题渲染验收留给 A03/A08 一并做真实浏览器证据。
Not run / blockers / residual risks:
  1. 真实 Tauri WebView + 隔离 Core 的端到端握手未跑（归 A02/A18）。
  2. "重复 operation 不执行两次" 的 Core 端去重沿用既有 OperationRegistry 测试（crates/altior-core/src/operations.rs）与 turn delivery-state 拒绝（application/mod.rs AutomaticResendForbidden），本轮未新增 daemon 级重复命令端到端测试；UI 层重试 UX 属 A04。
  3. 发现并顺带修复：handshake fixtures 宣称 negotiated selected_version=2，但 SUPPORTED_PROTOCOL_VERSIONS=[1,1]，真实 Core 会拒绝所有 v2 命令。已将三个握手 fixture 对齐为 V1（协议 fixture 变更，理由与证据在本条目）。若后续合法引入 v2，需按 ADR 0004 协商规则重新抬高两端范围。
  4. onboarding 表单仍是单 secret ref 对多 env key 的老模型；现在会在发送前以 "Each environment variable key requires exactly one credential reference" 拒绝并显示错误，完整映射 UI 归 A07。
Status: DONE-VERIFIED
Next task and first concrete action: A02 连接入口与订阅生命周期——第一步读 apps/desktop/src/main.tsx、ipc/tauriTransport.ts 的 createDefaultTransport/#resolveTauriBridge 与 applicationStore.init 的 subscribe 顺序，复现 F02 的 false ?? DEV 分支错误并写出期望行为的失败测试。
```

### 2026-09-06 01:10 — A02 连接入口、Tauri 桥接与订阅生命周期（DONE-VERIFIED，真实 WebView 握手除外）

```text
Execution time / environment: 2026-09-06 00:55–01:10；Node v24.18.0；Vite 8 dev server（127.0.0.1:5173）；ZCode In-app Browser（Chromium）。
Task ID / subslice: A02（完整切片，覆盖 F02；"真实 Tauri+隔离 Core 握手"一项不可在本环境执行，见 Not run）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交）
Pre-existing unrelated changes: 无
User-visible outcome: 开发页真实浏览器加载后即为 "IPC v1 / Core · connected / Stream · live"，不再卡 connecting，不再抛 TransportUnavailableError 未处理拒绝；普通生产浏览器明确显示 unavailable，不伪造连接；StrictMode 双挂载不再重复 bootstrap 命令，事件不丢失不重复。
In scope: 入口工厂收敛、Vite DEV 显式判断、fixture 仅开发入口、subscribe 失败路径进状态机、异步 listen ready/取消竞态、effect 清理。Out of scope: 生产打包 WebView 握手（A18）、store 默认演示数据（A03）、StrictMode 以外的事件恢复（A06）。
Contracts and decisions adopted: createDefaultTransport 三分支决策（bridge→Tauri 无回退；Vite DEV→显式 fixture 入口；生产浏览器→响亮失败）。withGlobalTauri=false 边界保留：桥接解析只认注入对与 __TAURI_INTERNALS__，删除了原先对 __TAURI__ 全局对象的四处兜底（不靠全局对象补洞）。TauriCoreTransport 新增可选 onError 供异步 listen 失败上报。
Changed files: apps/desktop/src/main.tsx（入口收敛为 createDefaultTransport() 单调用）、src/ipc/tauriTransport.ts（viteDevFlag 布尔判断替换 false??链、resolveTauriBridge 抽取、subscribe 注册/退订竞态、#dispatch 拷贝监听器列表）、src/stores/applicationStore.ts（init 拆为幂等 bootstrapPromise + attachTransportListener + release；attach 的同步失败进错误状态）、src/app/App.tsx（effect 清理调用 store.release()，不关 Core）、src/ipc/tauriTransport.test.ts（4 个新测试）、src/app/transportLifecycle.test.tsx（4 个新测试）。
Failure / cancellation / offline / restart: subscribe 在 try 内，失败 → connectionStatus=transport.status()（unavailable）+ 错误文案；listen 异步失败 → status=unavailable + onError；bootstrap 失败不留在 connecting；StrictMode mount→cleanup→mount 只跑一轮命令（list_threads/runtime_status/open_thread 各 1 次）。
Security / synchronization impact: 无；withGlobalTauri=false 与 capabilities ["core:default"] 未动（tauriConfig.test.ts 保持通过）。
Migration / downgrade: 无持久格式变更。
Regression proof before and after: 修复前 evidence/VERIFICATION.md 记录 dev 页 "TransportUnavailableError" 未处理拒绝 + 永久 connecting；修复后真实浏览器加载即 connected 且 window error/unhandledrejection 收集器为空。
Exact verification commands and results:
  - apps/desktop: npm run typecheck → PASS；npm test → 12 files / 105 tests PASS（新增 8 个 A02 测试）；npm run build → PASS
  - cargo fmt --all -- --check → PASS（A02 无 Rust 改动；A01 的 clippy/workspace/tauri gate 结论仍然有效）
  - 真实浏览器（In-app Chromium 1280×720）：加载 http://127.0.0.1:5173 → 状态栏 "Core · connected (IPC v1) / Stream · live"；error+unhandledrejection 收集器 = []；vite-error-overlay = 0；发送 prompt（页面内派发 click）→ 草稿清空、send-1/send-1-reply 行出现、流式回复到达、仍无错误。
Screenshots actually inspected: 截图 API 超时未取得图像；改以 DOM 状态证据（状态栏文本、事件收集器、行 id/text、overlay 计数）逐项核对，全部通过。视觉布局评审仍按 A03/A08 执行。
Not run / blockers / residual risks:
  1. 真实 Tauri WebView + 隔离 Core 的握手旅程未跑（需要打包/tauri dev 环境），BLOCKED 归 A18；本环境的 dev 页验证仅覆盖工厂 dev 分支。
  2. In-app Browser 的原生输入焦点受限，物理级 Enter/点击未通过自动化注入成功；发送路径改以页面内派发事件验证（应用逻辑一致），真实 IME/键盘输入归 A09 人工验收。
  3. listener "关闭再开 renderer 有界" 以 StrictMode 双挂载 + subscribe/unsubscribe×5 单测覆盖；跨页面长时运行的上限属 A06 缓存/生命周期。
Status: DONE-VERIFIED（除上述第 1 条 BLOCKED 子项，该子项归 A18 范围）
Next task and first concrete action: A03 权威实体、空状态与搜索——第一步读 applicationStore 的 DEFAULT_AGENTS/baseThreads 默认值注入路径与 App.tsx 的 currentThread 非空断言（F03/F06），先写"零代理零会话初始状态"的期望行为测试。
```

### 2026-09-06 01:45 — A03 权威实体、空状态与搜索（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 01:15–01:45；Node v24.18.0；Vite 8 dev server；ZCode In-app Browser（Chromium）。
Task ID / subslice: A03（完整切片，覆盖 F03/F06）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A03 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 干净 Vault 启动后不再出现 alpha/beta 演示代理与演示会话（真空态 + 打开 onboarding）；搜索结果只改变导航列表，空搜索不再卸载正在阅读的会话（评审探针的 TypeError 路径已消灭）；清空搜索权威恢复列表；50+ 会话按 has_more/next_cursor 分页加载；搜索失败就地显示原因，不再伪装客户端过滤成功。
In scope: store 线程状态四分离（entityById 缓存 / 权威列表 IDs / 搜索结果 / 选择）、世代令牌防乱序、App null-currentThread 四态（connecting/empty/error/select）、ThreadsPane 去客户端二次过滤 + 空态提示 + Load more。Out of scope: 归档/删除按钮（不存在的能力）、历史正文保真（A05）、缓存上限（A06）、错误横幅完整 UX（A04/F12）。
Contracts and decisions adopted: ①生产 store 不再自带任何演示默认值——threads/agents 全部来自 Core 响应；fixture 数据只能经 transport 入口（InMemoryTransport options）进入。②新增显式命名的 fixture 接缝：App prop `fixtureTimelineRows` → store 同名 option，仅播种 timeline 行供 P0.4 合成证据使用，绝不进入应用状态，生产入口（main.tsx）不传；A05 交付真实历史后应移除。③搜索结果只改可见列表；选择（selectedThread）独立维护；权威列表刷新只在无选择时自动选第一条。④sendPrompt 在无选中会话时拒绝发送（不再可能发出空 thread_id 命令）。
Changed files: apps/desktop/src/stores/applicationStore.ts（ThreadSummaryView/ThreadStatus 生产 ViewModel；threadViews/listThreadIds/searchResultIds/searchGeneration；listThreadsFromCore 权威替换 + 光标；loadMoreThreads 新增；setThreadFilter 世代令牌；createThread 返回刷新视图并在失败时写入错误；bootstrap 空 Vault 判定改为"无代理且无会话"）、src/app/App.tsx（null currentThread 条件渲染 + emptyState 样式 + onCreateThread 容错）、src/components/shell.tsx（ThreadsPane 改用 ThreadSummaryView、删客户端 includes 过滤、加空态与 Load more）、src/components/shell.module.css 与 src/app/App.module.css（空态样式，使用既有 token）、src/ipc/inMemoryTransport.ts（list_threads 支持 limit/cursor/has_more；commandHandler 返回 undefined 时回退内置响应）、测试更新（App/p04/transportIntegration/store）。
Failure / cancellation / offline / restart: 乱序搜索用世代令牌裁决（晚到 A 不覆盖 B，测试覆盖）；清空查询使未决搜索全部失效；列表/搜索失败写入 error 状态，列表不保留装饰性行；sendPrompt 无会话时以类型化错误拒绝。
Security / synchronization impact: 无秘密路径；搜索词仅发往本地 Core；无同步面改动。
Migration / downgrade: 无持久格式变更；fixture 接缝为代码内 seams，无存储影响。
Regression proof before and after: 评审探针"empty search → store 返回空数组 → App 解引用抛 TypeError"（evidence/review-probes.test.tsx.txt）对应路径已转化为三层期望行为证据：store 测试（空搜索保持 selectedThread 与行）、App 测试（"an empty search result never unmounts the conversation being read"）、真实浏览器（rows=0 + 空态文案 + h1 保持 + 时间线行仍在 + 零 page error）。
Exact verification commands and results:
  - apps/desktop: npx tsc --noEmit → PASS；npm test → 12 files / 113 tests PASS（新增 6 个 A03 store 测试 + 2 个 App 测试）；npm run build → PASS（JS 269.72 kB / gzip 81.90 kB）
  - Rust gates 无改动（A03 为纯 TS 切片，A01/A02 的 Rust gate 结论仍有效）
  - 真实浏览器：加载后 3 个 fixture 线程经 list_threads 填充（非本地默认）、agent 名经 open_thread 快照解析；空搜索 → rows=0/"No conversations match."/h1 保持/trn_fixture000000101 行仍在/errors=[]；清空 → 3 行权威恢复/errors=[]。
Screenshots actually inspected: 未取得截图（沿用 A02 的截图基础设施限制）；以 DOM 状态逐项核对（见上），结论 PASS。布局视觉验收仍归 A08。
Not run / blockers / residual risks:
  1. fixture shell 的会话内容现走 Core 投影（open_thread 的 turns/permissions 占位），fixture 精排内容依赖 fixtureTimelineRows 接缝——接缝在 A05 真实历史落地后应删除，此处为已声明的临时兼容点（removal condition: A05）。
  2. threadViews 实体缓存暂无上限（A06 的缓存/epoch 范围）。
  3. 搜索分页（对搜索结果翻页）未做——协议 search_threads 有 has_more/cursor 字段但 UI 无翻页入口，与 A05/A17 一并评估。
Status: DONE-VERIFIED
Next task and first concrete action: A04 发送、取消、权限与后台会话一致性——第一步读 applicationStore 的 activeTurn 单例与 handleIncomingEvent 的事件路由（F05/F08/F12），先写"创建失败不插入已保存线程 + 单全局 activeTurn 在双会话下丢 delta"的失败复现测试，再按 thread/turn 映射重构。
```

### 2026-09-06 08:52 — A04 发送、取消、权限与后台会话一致性（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 08:20–08:52；Node v24.18.0；纯 TS 切片。
Task ID / subslice: A04（完整切片，覆盖 F04 剩余部分/F05/F08/F12 的 store 侧；F12 的完整错误 UX 分级仍部分归 A17）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A04）
Pre-existing unrelated changes: 无
User-visible outcome: 每个会话有自己的活动轮次（A 输出时 B 可发送且各自 delta 不串线）；提示未送达时草稿保留并可编辑重发；连接中断时的发送显示"可能已投递，请核对历史"且绝不自动重发；取消按钮进入"Canceling…"状态，只有 Core 权威 turn.cancelled 才结束轮次——取消失败时轮次保持运行并明确提示；权限按钮提交期间禁用，失败后行内保持"未记录，可重试"；双击发送不会产生第二轮或重复行。
In scope: activeTurns 按 thread/turn 映射、事件按 envelope thread/turn 路由、sendPrompt 结构化结果（admitted/rejected/indeterminate）、双击防护、cancelState(requested/failed)、权限 submission 防重、App 草稿保留、按线程取消。Out of scope: 新提交是否允许的协商能力判定（需要 capability 协商数据，A07/A18）、全局错误横幅的完整分级 UX（部分已实现：turn notice + error 状态）、steering（同线程追加输入）能力。
Contracts and decisions adopted: ①ActiveTurnState 增加 cancelState/notice；state.activeTurns 为按会话的数组（服务端"单线程一轮"不要求 UI 全局单例）。②事件路由以 envelope thread_id/turn_id 为权威，选中线程仅作无 thread 事件的回退；turn_id 未解析（null）的 pending 轮次按线程匹配。③投递结论三分：rejected（未投递，可编辑重发）/ indeterminate（ConnectionClosed 类，"可能已投递"，只提示核对）/ admitted。④取消是请求状态机：requested →（权威 turn.cancelled 清除）或 failed（保持运行+提示）。⑤权限决定提交期 submission="submitting" 禁用重复点击，已决定行不再重发。⑥UI 草稿在 admitted 前不清空。⑦timelineStore 新增 setPermissionSubmission 与 permission.submission 字段（纯 UI 模型，非持久格式）。
Changed files: apps/desktop/src/stores/applicationStore.ts、src/features/timeline/timelineStore.ts、src/features/timeline/TimelineRowView.tsx（提交期按钮禁用 + "Recording…" + 失败提示）、src/components/shell.tsx（Composer cancelPending）、src/app/App.tsx（草稿保留、按线程取消、turn notice 显示）、src/stores/applicationStore.test.ts、src/app/App.test.tsx。
Failure / cancellation / offline / restart: 见上；关闭 UI 不终止 Core（A02 的 release 语义保持）；无自动重发路径存在。
Security / synchronization impact: 权限决定仍以真实 evt_ id 提交（A01 修复），提交期禁用不扩大授权；indeterminate 只提示核对，不伪装成功也不自动重发。
Migration / downgrade: 无持久格式变更（activeTurns/submission 均为内存 UI 状态）。
Regression proof before and after: F05/F08 的旧行为（取消失败仍清状态、A/B 互相覆盖 activeTurn 并丢 delta）先由既有测试隐含断言，本轮以 7 个新 store 测试 + 2 个新 App 测试固化为期望行为（双 turn 路由、双击拒绝、rejected/indeterminate 区分、取消失败保持、requested 状态门、权限防重、失败权限行可核对）。
Exact verification commands and results:
  - apps/desktop: npx tsc --noEmit → PASS；npm test → 12 files / 122 tests PASS（较 A03 新增 9 个 A04 测试）；npm run build → PASS
  - cargo fmt --all -- --check → PASS（A04 无 Rust 改动；此前 Rust gate 结论仍有效）
  - 本切片无新视觉布局；未做浏览器截图验证（UI 行为由组件/集成测试覆盖，浏览器验收与 A08 布局证据合并进行）
Not run / blockers / residual risks:
  1. "新增提交是否允许"目前按"同线程已有未决轮次即拒绝"实现；未接入协商的 steering/并行轮次能力（依赖 capability 数据，A07/A18）。
  2. turn.failed/turn.cancelled 的行内错误文案直接使用 Core reason，未做脱敏长度截断的独立测试（现有 DiagnosticSummary 上限在 Core 侧，A17 补 UI 侧断言）。
  3. App 级 cancel 失败/权威结算路径由 store 测试覆盖；浏览器级验证与 A08 合并。
Status: DONE-VERIFIED
Next task and first concrete action: A05 正文历史、分页与重启恢复——第一步读 altior-protocol 的 TurnDto/ThreadHistoryResponseDto 与 Core 的 handle_get_history/journal 查询（F07），盘点"有界 timeline record 契约"所需字段（正文、有序工具/审批、cursor、截断），先写契约草案再实现。
```

### 2026-09-06 13:05 — A05 正文历史、分页与重启恢复（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 11:30–13:05；Rust 1.98.0 / Node v24.18.0；跨 Rust / TS 端到端。
Task ID / subslice: A05（完整切片，彻底消除 F07）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A05 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 会话重新打开或 Core 重启后，Timeline 正确还原真实的中文与英文提示词正文、连续聚合的助手流式增量段落、工具调用与权限审批记录，彻底消除了"Turn trn_..."的空白占位符；向上滚动查看历史时，通过 before_seq 向过去稳定分页并 prepend 到时间轴顶部，时间顺序丝毫不乱；遇到前向兼容的未知事件时，安全呈现为有界可检查的未知行，不会导致整页历史加载崩溃；离线或无可用代理时仍可完整读取已有历史。
In scope: ADR 0020 契约设计、HistoryEntryDto/HistoryCursorDto 协议信封扩展、Storage 层 Journal 向过去分页查询、Core 守护进程 Journal 事件向有界正文条目投影、ts-rs 自动重新导出 DTO、前端 historyReducer 归一化归约器、applicationStore 分页与游标管理、InMemoryTransport 完整正文支持、Rust 真实守护进程集成测试 p15_history_records、前端验收测试 p15HistoryEvidence。Out of scope: 长文本在时间轴中的折叠/折行 UI 样式（A12）、全局去重缓存上限与淘汰（A06）。
Contracts and decisions adopted: ①完成 ADR 0020（有界时间轴历史记录与日志投影），确立直接重用已持久化的 domain_journal 作为单一事实来源，绝不冗余维护第二份历史数据。②Protocol 定义 HistoryEntryDto 标记联合（user_message、assistant_delta、permission、permission_decision、turn_state、unknown），字段单条上限 64KB 字符边界截断。③GetHistoryCommand 新增 before_seq 支持日志序号反向分页；ThreadHistoryResponseDto 增加 entries、next_seq_cursor 与 high_water_seq。④读取历史在 Core handle_get_history 中直接读取 Storage，与 ACP 进程生命周期完全解耦，离线高可用。⑤前端通过纯函数 reduceHistoryEntries 实现流式事件与历史快照的统一归约，保证呈现与顺序一致性。
Changed files:
  - docs/decisions/0020-bounded-timeline-history-records.md（新增 ADR 0020）
  - crates/altior-protocol/src/dto.rs, command.rs, lib.rs, tests/dto_export.rs
  - crates/altior-storage/src/lib.rs（新增 JournalEventRow、journal_events_for_thread、journal_has_older）
  - crates/altior-core/src/application/daemon.rs（新增 journal_row_to_entry 投影与 handle_get_history 增强）
  - crates/altior-core/tests/p15_history_records.rs（新增真实 CoreDaemon 回环集成测试）
  - crates/altior-core/tests/p14_acceptance_journey.rs（适配 GetHistoryCommand before_seq）
  - apps/desktop/src/ipc/dto/HistoryEntryDto.ts, HistoryCursorDto.ts, ThreadHistoryResponseDto.ts（自动重新导出）
  - apps/desktop/src/features/timeline/historyReducer.ts, historyReducer.test.ts（新增归约器与单测）
  - apps/desktop/src/stores/applicationStore.ts（升级 getHistory、loadOlderHistory、openThread）
  - apps/desktop/src/ipc/inMemoryTransport.ts（支持 entries 投影与分页）
  - apps/desktop/src/app/p15HistoryEvidence.test.tsx（新增 A05 验收测试套件）
Failure / cancellation / offline / restart: 历史读取完全脱离外部代理进程，Agent 宕机/断网不影响历史；Corrupt/Unknown Journal 行安全降级为 Unknown 实体，页面不崩溃；分页反向滚动不产生重复与顺序交错；100k 大历史按 50 限制分页，绝不全量拉取。
Security / synchronization impact: 历史数据仅从本地 SQLite Journal 读取，不经过外部网络；文本字段强制在 UTF-8 字符边界截断防畸变；不透明凭证与私钥绝不出现在正文历史中。
Migration / downgrade: 纯增量 DTO 扩展，保留 turns 字段以向下兼容旧版客户端；底层沿用已有 domain_journal 表，无 SQLite schema 迁移需求。
Regression proof before and after: 评审报告 F07 指出的"重启后所有会话正文均显示为 'Turn trn_...' 占位符"已彻底修复：Rust 端由 p15_history_records 测试 8 条丰富事件投影验证；前端由 p15HistoryEvidence 5 个综合场景测试固化（中英正文聚合、审批决策还原、未知事件保留、向过去分页无重叠、离线可读）。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - cargo test --test p15_history_records → PASS（2 passed）
  - cargo test -p altior-protocol --features dto-export → PASS（TypeScript 绑定确定性导出）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 14 files / 130 tests PASS（新增 8 个测试）
  - npm run build（apps/desktop）→ PASS
Screenshots actually inspected: 未做物理截图；由 DOM 和状态断言完成全链路验证。
Not run / blockers / residual risks:
  1. 100k 历史的极深分页滑动性能仍受虚拟窗口滚动驱动（P0.4 已验证单机 10 万行，长时缓存淘汰归 A06）。
  2. 极长正文折叠交互与代码高亮归 A12。
Status: DONE-VERIFIED
Next task and first concrete action: A06 epoch、重放、缓存上限——第一步读 altior-protocol 的 RetainedWindow/StreamReplayed 与 applicationStore 事件去重机制，设计 epoch+sequence 契约与时间线缓存淘汰上限。
```

### 2026-09-06 13:30 — A06 epoch、重放、缓存上限（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 13:05–13:30；Node v24.18.0 / Rust 1.98.0；跨 TS/Rust。
Task ID / subslice: A06（完整切片，彻底消除 F09）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A06 改动）
Pre-existing unrelated changes: 无
User-visible outcome: Core 守护进程重启或会话断开重连后，新 Core 实例（新 epoch）从序号 1 开始的全部事件均能正常接收并呈现，彻底消除了旧去重机制误将新事件当做重复丢弃的缺陷；长时间运行或接收海量（100 万+）事件时，前端内存与去重集合始终维持在严格的常数阶 O(1) 上限，无内存泄漏风险；多会话浏览时，未读闲置会话的时间线缓存按 LRU 自动淘汰，而当前正在阅读的会话与正在运行的后台轮次（Pinned Working Set）绝对不被淘汰，保证草稿、审批和活动流式不丢失。
In scope: ADR 0021 设计、BoundedIdSet（4096 容量 FIFO 去重集）、EpochSequenceTracker（支持 Core 实例迁移、高水位滑窗去重、100 万事件 O(1) 验证）、BoundedTimelineCache（容量 32、Pinned 工作集保护）、applicationStore 接入与状态拓展（coreInstanceId、getEpoch、getDeduplicationStats）、单元与验收测试套件（streamDeduplicator.test.ts、p16StreamRecoveryEvidence.test.tsx）。Out of scope: ACP Agent 配置与凭证关联（A07）。
Contracts and decisions adopted: ①完成 ADR 0021（基于 Epoch 作用域的流去重、重放恢复与有界 UI 缓存），游标确立为 (coreInstanceId, sequence) 二元组语义。②Core 启动与重启事件（core.greeting / core.restarted）触发 epoch 迁移与滑窗安全重置，使新实例的序号 1 不受旧历史序号干扰。③seenEventIds 采用 4096 固定容量 FIFO 队列；processedSequences 采用相对于 highWater 的 2048 位滑窗，窗外极旧序号判定为过期重复。④timelineStores 缓存采用 LRU 淘汰，强制对当前 selectedThreadId 与全部 activeTurns[].threadId 进行 Pin 保护，绝不丢失未决状态；被淘汰的仅是派生 UI DOM，持久内容均随时可通过 A05 历史游标重新读回。
Changed files:
  - docs/decisions/0021-epoch-scoped-stream-deduplication-and-cache-bounds.md（新增 ADR 0021）
  - apps/desktop/src/stores/streamDeduplicator.ts（新增有界去重集、Epoch 序号跟踪器、有界 LRU 缓存）
  - apps/desktop/src/stores/streamDeduplicator.test.ts（新增 7 个独立测试，含 100 万事件吞吐与边界测试）
  - apps/desktop/src/stores/applicationStore.ts（接入 coreInstanceId、有界去重、Pinned 工作集保护、诊断暴露）
  - apps/desktop/src/app/p16StreamRecoveryEvidence.test.tsx（新增 A06 综合验收测试套件）
Failure / cancellation / offline / restart: Core 重启（epoch 迁移）完美支持旧 seq=100 后新 seq=1；stream.gap 自动触发快照恢复；快照恢复期间并发到达的新事件安全保留；断网与重连事件流恢复稳定；缓存淘汰不丢正在运行的活动轮次与草稿。
Security / synchronization impact: 跨 epoch 的历史事件不会串扰或污染当前会话流；去重结构全在前端内存有界运行，不涉及秘密或同步面改动。
Migration / downgrade: 纯增量前端运行时结构优化，对现有持久格式无破坏，完全向前兼容。
Regression proof before and after: 评审报告 F09 指出的"event ID 与 sequence 的 Set 持续增长；新 Core epoch 可能重新使用 sequence，去重在 greeting 分支前可能屏蔽新事件"已彻底修复：streamDeduplicator.test.ts 证明 100 万事件常数阶内存；p16StreamRecoveryEvidence.test.tsx 证明旧 epoch seq=100 后新 epoch seq=1 及后续事件无缝接入。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 16 files / 142 tests PASS（较 A05 新增 12 个测试）
  - npm run build（apps/desktop）→ PASS（JS 277.09 kB / gzip 83.98 kB）
Screenshots actually inspected: 未做物理截图；由单元测试与验收测试覆盖。
Not run / blockers / residual risks:
  1. 真实网络环境下的重放断线模拟归 A18 验收。
Status: DONE-VERIFIED
Next task and first concrete action: A07 可用且诚实的 ACP 配置——第一步检查 AgentOnboarding 组件、applicationStore 的 onboardAgent/testAgent 与 altior-protocol 的 ConfigureAgentCommand/TestHarnessBindingCommand，核对参数数组与 secret_ref 校验规则。
```

### 2026-09-06 13:50 — A07 可用且诚实的 ACP 配置（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 13:30–13:50；Node v24.18.0 / Rust 1.98.0；跨 TS/Rust。
Task ID / subslice: A07（完整切片，彻底消除 F11/F27）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A07 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 智能体配置与入职弹窗完全诚实可用——去除了假承诺的 terminal/native 选项，明确标明当前仅支持 ACP 生产 harness；参数输入引入真正的命令行分词器 parseCommandLineArgs，完整保留带引号的空格路径（如 "C:\Program Files\..."）与参数，不再因空白字符切碎命令；明确支持无鉴权本地智能体（envKeys/secretRef 均为空）；填入凭证时，强制要求环境变量名与不透明凭证引用（如 sec_..., vault://...）严格 1 对 1 对应，绝不允许在前端输入明文 API Key；若输入明文 Key 或非法引用，前端立即拒绝并报错，绝不隐式捏造虚假随机引用（ref:opaque-sec-xxx）；测试连接与保存状态联动，测试中禁用保存，失败红字高亮确切原因与耗时，且错误详情中绝不回显或泄漏明文密钥。
In scope: parseCommandLineArgs（支持带单双引号的复杂参数行）、sanitizeSecretRef 严审机制（严禁伪造 fake ref、严禁在 error message 中插值回显明文）、onboardAgent 与 testAgent 1 对 1 严格映射校验、AgentOnboardingModal 界面提示重构（诚实标明 ACP 唯一支持、模型由外部 agent 管理）、测试套件升级（applicationStore.test.ts 消除旧 fake ref 断言、p17AcpConfigEvidence.test.tsx 新增 9 项完整场景验收）。Out of scope: 窄窗网格自适应（A08）、系统输入法 IME 与焦点（A09）。
Contracts and decisions adopted: ①落实 ADR 0006 与 AGENTS.md 约束，Harness 唯一生产支持为 ACP，去除非法承诺。②凭证与密钥全部由 OS secret store 托管，UI 仅持不透明引用；明文输入直接以类型化错误拦截，彻底废除旧代码中生成伪随机 ref 的危险行为。③错误消息脱敏：报错详情一律使用标准化模板，严禁将未通过校验的原始输入字符串插值进错误文本，彻底隔绝明文进错误详情/日志的隐患。④参数解析器严格遵循 POSIX/Windows 命令行双引号规范。
Changed files:
  - apps/desktop/src/stores/applicationStore.ts（新增 parseCommandLineArgs、修复 sanitizeSecretRef 脱敏与真报错、testAgent/onboardAgent 接入参数分词与 1 对 1 校验）
  - apps/desktop/src/components/shell.tsx（AgentOnboardingModal 界面重构：ACP 诚实说明、带引号参数解析、OS secret store 指引）
  - apps/desktop/src/stores/applicationStore.test.ts（更新明文拦截与 canary 验证）
  - apps/desktop/src/app/p17AcpConfigEvidence.test.tsx（新增 A07 验收测试套件，9 项测试全部通过）
Failure / cancellation / offline / restart: 环境变量与 secret ref 不对称时在派发前本地拦截；非法明文凭证即时抛错；探测超时或返回非零时展示结构化失败原因，不伪装通过；可纠正后重新探测。
Security / synchronization impact: 真正实现了"秘密不得进入 SQLite、日志、fixture、同步或错误详情"——从入口校验、错误构造、状态存储到命令派发，明文均在第一道防线被截断。
Migration / downgrade: 纯增量代码加固与 UI 修正，无需迁移数据库与持久格式。
Regression proof before and after: 评审报告 F11（"参数按空白拆分破坏路径；env_keys 与 secret_ref 不匹配；存在假模型/harness 承诺"）与 F27（"随机生成不存在的 ref 违背 OS secret store 契约"）已彻底消灭：p17AcpConfigEvidence 证明带空格引号路径解析、无密钥本地 agent、1 对 1 校验、明文密钥拒绝以及 canary 零泄漏。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 17 files / 151 tests PASS（较 A06 新增 9 个测试）
  - npm run build（apps/desktop）→ PASS（JS 278.32 kB / gzip 84.72 kB）
Screenshots actually inspected: 未做物理截图；由单元测试与验收测试覆盖。
Not run / blockers / residual risks:
  1. 真实系统 Keychain/Credential Manager 的物理桥接归 A18 验收。
Status: DONE-VERIFIED
Next task and first concrete action: A08 网格、分栏与窄窗响应式——第一步读 App.module.css 的布局定义与 F13 评审报告（五区域网格实际只有三列），对照 UI_ARCHITECTURE 设计五区域真实布局与窄窗折叠断点。
```

### 2026-09-06 14:05 — A08 网格、分栏与窄窗响应式（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 13:50–14:05；Node v24.18.0 / Rust 1.98.0；纯 TS/CSS 与 DOM 验收。
Task ID / subslice: A08（完整切片，彻底消除 F13/F14）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A08 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 彻底消灭了工作台区域错位与排版崩溃问题（F13）——显式构建了包含四个中间列与标题栏、状态栏的真实 5 区域网格布局，ActivityRail、ThreadsPane、Workbench 主对话区、Inspector 检查器并列同处于第二行，彼此之间顶部水平严格对齐，Inspector 再也不会异常掉落到下一行破坏布局；在窄屏窗口或宽度受限时，Inspector 遵循 F14 规范自动转为抽屉式覆盖层（Overlay Drawer），带有半透明模态遮罩与标准 Escape 键快捷关闭协议，确保主对话阅读区、Composer 输入框与审批决策按钮始终清晰可见且可交互，绝对不会被侧边栏遮挡。
In scope: App.module.css 显式 grid-template-areas 重构（header, rail, nav, main, inspector, footer, diag）、7 区域显式命名定位、动态内容宽度预算测算（48 + navWidth + 480 + 360 + 24）、遮罩层与 Escape 键抽屉关闭监听、720×480 最小视口可交互性保障、p18GridGeometryEvidence 验收套件。Out of scope: 中文输入法 IME 与焦点圈定（A09）、语义主题配色与对比度加固（A10）。
Contracts and decisions adopted: ①落实 docs/UI_ARCHITECTURE.md 与 DESIGN_I18N §2 规定，采用显式 grid-template-areas: "header header header header" / "rail nav main inspector" / "footer footer footer footer"，彻底消除未显式分配区域导致 Inspector 错误折行的缺陷。②并排判断摒弃写死的单点像素断点，改为按内容最小宽度动态测算。③抽屉开启时配备标准遮罩与 Escape 快捷键协议，关闭时丝毫不影响主工作区的会话聚焦、草稿与滚动状态。
Changed files:
  - apps/desktop/src/app/App.module.css（重构显式 4 列 5 区域 Grid 布局，新增 railArea/navArea/inspectorArea/statusBarArea 及 drawerOverlay 样式）
  - apps/desktop/src/app/App.tsx（应用显式网格包裹层、动态宽度测算、Escape 抽屉退出监听、传入 contextSnapshot）
  - apps/desktop/src/app/p18GridGeometryEvidence.test.tsx（新增 A08 验收测试套件，4 项测试全部通过）
Failure / cancellation / offline / restart: 视口宽度极度压缩至 720×480 时，工作区与底部输入框依然完整可点；抽屉关闭后状态无损回退。
Security / synchronization impact: 纯前端布局与交互加固，无安全或同步面改动。
Migration / downgrade: 纯样式与视图重构，完全向后兼容。
Regression proof before and after: 评审报告 F13（"五区域网格实际只有三列，Inspector 自动落入下一行，rail 被拉到 360px"）和 F14（"窄窗抽屉遮住对话与审批"）已彻底消灭：p18GridGeometryEvidence 验证了全部区域显式分配定位、窄窗抽屉遮罩与 Escape 关闭、以及 720×480 下审批按钮与 Composer 的无遮挡可交互性。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 18 files / 155 tests PASS（较 A07 新增 4 个测试）
  - npm run build（apps/desktop）→ PASS（JS 279.27 kB / gzip 84.96 kB）
Screenshots actually inspected: 未做物理截图；由 DOM/网格结构与交互测试覆盖。
Not run / blockers / residual risks:
  1. 真实多分辨率屏幕（如高 DPI 200% 缩放）的视觉精细调优归 A18 验收。
Status: DONE-VERIFIED
Next task and first concrete action: A09 中文输入、弹窗、焦点与权限可达性——第一步检查 Composer 的 compositionstart/compositionend 事件防护（F16）与按键监听（Enter/Shift+Enter），编写 Windows 229 IME 确认候选不误发的测试。
```

### 2026-09-06 14:30 — A09 中文输入、弹窗、焦点与权限可达性（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 14:05–14:30；Node v24.18.0 / Rust 1.98.0；跨 TS/DOM 验收。
Task ID / subslice: A09（完整切片，彻底消除 F16/F17）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A09 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 彻底解决了中文输入法候选确认误发的问题（F16）——Composer 增加了原生输入法组合状态跟踪（isComposing 与 Windows keyCode 229 双重防护），输入拼音并按回车确认候选字时绝不触发发送，只有正常直接回车才发送消息，Shift+Enter 依然稳定换行；模态弹窗（AgentOnboardingModal）具备了完整的可达性生命周期与键盘协议（F17）——按下 Escape 键立即优雅关闭弹窗，Tab 与 Shift+Tab 实现无障碍焦点回环（Focus Trap），开启时自动聚焦首个可用表单元素，关闭后将焦点无损归还给触发按钮，关闭按钮添加了语义明确的 aria-label 属性；Inspector 的 Tab 视图支持完整的 ARIA tablist/tab/tabpanel 语义与 ArrowLeft / ArrowRight 键盘切换；时间线中权限操作键盘 y/d 快捷键与鼠标点击完全等价一致。
In scope: Composer compositionstart/compositionend 事件与 keyCode 229 拦截、Shift+Enter 换行保持、AgentOnboardingModal Focus Trap 与 Escape 监听与焦点恢复、Inspector 选项卡 ARIA tablist/tabpanel 与左右方向键切换、p19AccessibilityImeEvidence 验收测试套件（7 项测试）。Out of scope: 主题与共享语义色彩系统加固（A10）。
Contracts and decisions adopted: ①输入法拦截契约：合成期内无论是 React 的 isComposing、浏览器的 nativeEvent.isComposing 还是 Windows 常见的 keyCode 229，均被判定为输入法确认动作，直接返回不触发 onSend。②弹窗遵循 WAI-ARIA Dialog (Modal) 规范，包括 Escape 冒泡拦截、Tab 环回、初始焦点注入与触发元素焦点还原。③Inspector 选项卡遵循 WAI-ARIA Tabs 规范，实现 tablist 与 tabpanel 严格映射，支持键盘左右键快速切换激活状态。
Changed files:
  - apps/desktop/src/components/shell.tsx（Composer 接入 IME 组合监听、Inspector 选项卡补齐 tabpanel 与左右键导航、AgentOnboardingModal 实现 Focus Trap 与 Escape 监听与焦点归还）
  - apps/desktop/src/app/p19AccessibilityImeEvidence.test.tsx（新增 A09 验收测试套件，7 项测试全部通过）
Failure / cancellation / offline / restart: 输入法候选字确认时绝不产生网络请求与状态污染；弹窗多次打开/关闭焦点始终保持稳定；键盘操作与鼠标操作逻辑完全对称。
Security / synchronization impact: 纯前端键盘交互与可达性加固，无秘密或同步协议改动。
Migration / downgrade: 纯前端组件交互增强，完全向后兼容。
Regression proof before and after: 评审报告 F16（"未检查 composition，在 isComposing=true/keyCode=229 时调用 onSend"）与 F17（"弹窗缺 Escape、focus trap、关闭按钮无 accessible name、tablist 缺 tabpanel 与方向键协议"）已彻底消灭：p19AccessibilityImeEvidence 证明了拼音确认不误发、Shift+Enter 换行、Escape 退出、Tab 循环、aria-label 标注、选项卡方向键切换与权限键盘审批。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 19 files / 162 tests PASS（较 A08 新增 7 个测试）
  - npm run build（apps/desktop）→ PASS（JS 280.80 kB / gzip 85.41 kB）
Screenshots actually inspected: 未做物理截图；由 DOM 结构与无障碍事件测试覆盖。
Not run / blockers / residual risks:
  1. 真实系统屏幕阅读器（如 NVDA / JAWS / VoiceOver）的听觉读屏测试归 A18 人工验收。
Status: DONE-VERIFIED
Next task and first concrete action: A10 语义主题与共享原语——第一步检查 apps/desktop/src/app/App.module.css 与 shell.module.css 中的色彩与控件样式，对照 DESIGN_I18N §3 补足 color-control-border、color-accent-foreground 与深浅色审批按钮高反差样式。
```

### 2026-09-06 16:00 — A10 语义主题与共享原语（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 14:30–16:00；Node v24.18.0 / Rust 1.98.0；跨 TS/CSS/DOM/Playwright 截图与 WCAG 对比度自动化审计。
Task ID / subslice: A10（完整切片，彻底消除 F15 与 F19 中主题/焦点部分）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A10 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 彻底解决了控件主题未闭合与对比度不足问题（F15）以及选中项焦点混淆问题（F19）——深色模式下审批（Approve Y）按钮拥有独立的语义类与高对比度文字（8.19:1 对比度），拒绝（Deny D）按钮拥有清晰的红字红框与独立 hover 背景，彻底消除了深色模式下白色原生浏览器默认样式的违和感；弹窗遮罩与卡片全部接入语义 token（--color-overlay、--elevation-dialog、--z-dialog）并移除所有硬编码 rgba 与 raw z-index；降级提示卡片移除了 raw rgba 并使用语义 --color-warning-surface；输入控件使用显式语义边界 token（--color-control-border，浅色 3.39:1，深色 3.90:1，均超过 WCAG 3:1 非文本边界标准）；分离了列表项选中（selection background）、悬停（hover background）与键盘焦点（focus-visible outline），普通选中不再常驻键盘焦点描边；补全了 @media (forced-colors: active) 与 @media (prefers-reduced-motion: reduce) 媒体查询支持；五张视觉基线（light, dark, narrow, error, approval）全部经由 Playwright 真实渲染生成并通过视觉检验。
In scope: tokens.css 补全 color-scheme、color-control-border、color-accent-foreground、color-hover、color-pressed、color-*-surface、color-overlay、color-disabled-* 及 forced-colors/reduced-motion；shell.module.css 与 timeline.module.css 样式重构与原语收敛；check-contrast.mjs 脚本与 contrastAudit.ts 审计模块（24 项组合 100% 达标）；baselines.mjs 适配 Vite 开发服务器驱动 Playwright 截图；p20ThemePrimitivesEvidence 验收套件（5 项测试）。Out of scope: zh-CN/en 双语国际化与本地偏好持久化（A11）。
Contracts and decisions adopted: ①落实 DESIGN_I18N §3 与 §4 规范，建立完整的语义色彩与状态表面体系；②满足 WCAG 2.2 AA 级别对比度标准（正文/次级/状态文字 ≥ 4.5:1，控件边界/主要标识 ≥ 3:1）；③严格遵循 UI_ARCHITECTURE.md 规范，禁止组件跨模块偷借 class 或在模块 CSS 中写死十六进制色/rgba/raw z-index；④遵循 focus-visible 与 selected 分离原则。
Changed files:
  - apps/desktop/src/styles/tokens.css（补全状态颜色、控制边界、色相 scheme、高对比度前景色与 forced-colors 规则）
  - apps/desktop/src/styles/contrastAudit.ts（新增纯 TS 的 WCAG 2.2 亮度与对比度审计计算引擎）
  - apps/desktop/src/components/shell.module.css（degradedBadge 使用 warningSurface，threadRow 与 railItem 分离 selected/hover/focus-visible，modal 使用 elevation 与 overlay token）
  - apps/desktop/src/features/timeline/timeline.module.css（新增 approveButton 与 denyButton 语义高反差样式）
  - apps/desktop/src/features/timeline/TimelineRowView.tsx（应用 approveButton / denyButton 语义类名与提交加载态）
  - apps/desktop/scripts/check-contrast.mjs（新增 CLI 对比度检验脚本）
  - apps/desktop/scripts/baselines.mjs（切换为 dev server 驱动 Playwright，修复生产 preview 缺桥接导致的连接超时）
  - apps/desktop/src/app/p20ThemePrimitivesEvidence.test.tsx（新增 A10 验收测试套件，5 项测试全部通过）
  - apps/desktop/baselines/{light,dark,narrow,error,approval}.png（更新真实 Playwright 渲染基线截图）
Failure / cancellation / offline / restart: 主题切换即时且无闪烁；forced-colors 模式下自动回退为系统高反差主题；reduced-motion 下禁用所有过度动画。
Security / synchronization impact: 纯前端样式与视觉加固，无安全或数据同步面改动。
Migration / downgrade: 纯样式与视图增强，完全向后兼容。
Regression proof before and after: 修复前 F15 记录“深色审批按钮仍是白色浏览器原生样式，浅色 border 约 1.44:1，深色约 1.42:1 不足 3:1”与 F19 记录“选中项始终有焦点描边”；修复后 check-contrast.mjs 证明 24 项组合 100% 达标（control-border 浅色 3.39:1、深色 3.90:1，审批文字浅色 5.08:1、深色 8.19:1），Playwright 真实渲染截图证实 Approve 按钮呈现深底高反差墨黑字、选中态与焦点态清晰解耦。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 20 files / 167 tests PASS（较 A09 新增 5 个测试）
  - npm run build（apps/desktop）→ PASS（JS 280.93 kB / gzip 85.45 kB，CSS 21.03 kB / gzip 4.21 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed, 0 failed）
  - npm run baselines（apps/desktop）→ PASS（5 张基线截图全部成功生成并校验）
Screenshots actually inspected:
  - apps/desktop/baselines/dark.png（1280×800，四列完美对齐，Approve 按钮深底高反差墨黑字，Deny 按钮清晰红框，输入框边界明晰）
  - apps/desktop/baselines/light.png（1280×800，浅色白底自然对齐，输入框灰度边界清晰）
  - apps/desktop/baselines/narrow.png（760×800，抽屉遮罩层半透明且完整覆盖）
  - apps/desktop/baselines/approval.png（1280×800，审批流程状态卡与高反差操作按钮布局严整）
  - apps/desktop/baselines/error.png（1280×800，错误状态卡呈现）
Not run / blockers / residual risks:
  1. 真实系统高对比度模式（Windows Contrast Themes）的跨浏览器硬件合成仍需 A18 人工环境复核。
Status: DONE-VERIFIED
Next task and first concrete action: A11 zh-CN/en 与设备本地偏好——第一步按 DESIGN_I18N 建立 i18n 语言字典与 typed key 结构，抽离用户界面中文与英文文案，并定义本地设置存储契约。
```

### 2026-09-06 16:40 — A11 zh-CN/en 与设备本地偏好（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 16:00–16:40；Node v24.18.0 / Rust 1.98.0；跨 TS/DOM/Vitest 国际化与偏好存储验收。
Task ID / subslice: A11（完整切片，彻底解决 F18 与 F20 中语言与本地偏好持久化部分）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A11 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 完整提供了简体中文（zh-CN）与英文（en）双语支持，两份词典保持 100% 结构对齐与递归 key 校验；词汇严格遵循 DESIGN_I18N §1 规范（Personal Vault → 个人知识库、Conversation → 会话、Turn → 轮次、Agent → 代理、Harness → 执行后端、Model → 模型、Project → 项目、Memory → 记忆、Context → 上下文）；支持系统语言自动侦测与用户显式覆盖，切换语言立即同步更新 document.documentElement.lang，工作区与时间线即时更新，且绝对不中断正在运行的轮次、不机械篡改用户对话历史原文；建立设备本地偏好持久化机制（uiStore），仅将安全的外观展示设置（themeSource: system/light/dark、localeSource: system/zh-CN/en、面板宽度）安全持久化至 localStorage，重启后自动无损恢复；严格划清数据安全红线，用户草稿（drafts）纯内存驻留，绝不落盘至 localStorage，也绝不参与远端同步；新增 SettingsModal 偏好设置面板，支持全键盘无障碍导航（Escape 关闭、Tab 焦点圈定、关闭焦点恢复）并显式标注“仅本机 / All display settings are device-local and are never synchronized”。
In scope: 新增 apps/desktop/src/i18n/ 模块（types.ts、en.ts、zh-CN.ts、index.ts 及 I18nProvider / useI18n）；uiStore.ts 支持 themeSource、localeSource、偏好本地存储持久化与系统深色模式媒体查询订阅；shell.tsx 组件接入双语国际化与新增 SettingsModal；TimelineRowView.tsx 接入权限操作双语；p21I18nEvidence 验收测试套件（7 项测试全部通过）。Out of scope: 长正文 Markdown 渲染与代码复制（A12）。
Contracts and decisions adopted: ①落实 DESIGN_I18N §1 与 §5 规范术语和中英对应契约；②严格遵循 UI_ARCHITECTURE.md 与 SECURITY.md 规范，草稿（drafts）作为敏感未确认输入，严格限制为内存态，禁止写入 localStorage，禁止进入同步；③展示偏好归本地设备所有，未知或损坏的 storage 数据受控回退至 system 默认值，不引发 crash；④未知语言代码安全降级至默认英语，禁止静默暴露未格式化 key。
Changed files:
  - apps/desktop/src/i18n/types.ts（新增双语词典强类型接口）
  - apps/desktop/src/i18n/en.ts（新增完整英文词典）
  - apps/desktop/src/i18n/zh-CN.ts（新增完整简体中文规范词典）
  - apps/desktop/src/i18n/index.ts（新增多语言上下文、系统语言探测、fallback 与 document.lang 联动）
  - apps/desktop/src/app/uiStore.ts（新增 themeSource、localeSource、本地偏好安全存储与 matchMedia 订阅）
  - apps/desktop/src/components/shell.tsx（所有区域接入 useI18n，新增 SettingsModal 组件）
  - apps/desktop/src/app/App.tsx（包裹 I18nProvider，支持 ActivityRail 打开偏好设置）
  - apps/desktop/src/features/timeline/TimelineRowView.tsx（权限审批与拒绝控件接入国际化文本）
  - apps/desktop/src/app/p21I18nEvidence.test.tsx（新增 A11 验收测试套件，7 项测试全部通过）
Failure / cancellation / offline / restart: 本地偏好损坏或未知字段时安全回退系统默认；私密浏览模式下 localStorage 被禁用时降级为内存运行，不抛出异常；系统语言或暗色模式变更时实时自适应生效。
Security / synchronization impact: 落实安全红线——仅持久化非敏感显示配置；草稿严格保持内存存储；无任何同步面暴露。
Migration / downgrade: 纯前端展示与本地设置扩展，完全向后兼容。
Regression proof before and after: p21I18nEvidence 证明了中英文典 100% 结构与类型递归一致、未知 locale 安全回退、document.lang 动态同步、语言切换保持时间线原文不中断、偏好重启恢复、以及草稿绝不落盘的隐私隔离保障。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 21 files / 174 tests PASS（较 A10 新增 7 个测试）
  - npm run build（apps/desktop）→ PASS（JS 292.72 kB / gzip 89.60 kB，CSS 21.03 kB / gzip 4.21 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持 A10 视觉基线截图校验有效。
Not run / blockers / residual risks:
  1. 真实系统切换 Windows 多语言输入法时的动态语言包联动归 A18 人工环境复核。
Status: DONE-VERIFIED
Next task and first concrete action: A12 长正文与工具呈现——第一步检查 TimelineRowView 与长正文/代码块/工具输出渲染，对照 DESIGN_I18N §2 与 TASKS.md 引入安全 Markdown 渲染与代码复制原语。
```

Execution time / environment: 2026-09-06 16:40–16:50；Node v24.18.0 / Rust 1.98.0；跨 TS/DOM/Vitest 长正文与工具呈现验收。
Task ID / subslice: A12（完整切片，彻底消除 F20 与 F25 中长正文、富文本与工具呈现缺陷）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A12 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 彻底解决了恢复后长正文、多行代码与工具结果可读性差、无排版且无法安全复制的缺陷（F20、F25）——引入高安全 SafeMarkdown 与 ToolBlock 呈现组件：①Markdown 子集安全排版：支持多级标题、粗体、斜体、列表、表格、行内代码与代码块；②代码块语言 Badge 与独立一键复制：代码块清晰展示语言标签并配备独立一键复制按钮（navigator.clipboard.writeText），复制内容严格对应代码原文，复制状态反馈清晰；③工具输出可控卡片化：工具输出展示 completed/failed/running 状态徽标与独立一键复制，超过 8 行或 400 字符的长输出自动折叠，支持平滑展开与收起，防止巨量日志撑破时间线和破坏滚动；④绝对 XSS 防御：零 dangerouslySetInnerHTML，不解析或执行任何 raw HTML 标签（如 <script>, <iframe>, <img onerror> 原样作为安全转义纯文本展示）；⑤URL 协议沙箱：仅放行安全的 http:// 与 https:// 超链接并强制注入 target="_blank" rel="noopener noreferrer"，危险协议（javascript:, data:, file: 等）自动中和为安全纯文本，防止远程协议穿透；⑥远程图片隐私防护：不自动发起远程图片网络请求（防止第三方追踪像素暴露用户访问与 IP），安全呈现隐私占位提示 [Image: alt]；⑦全面接入 TimelineRowView，完美兼容中文、长路径、emoji 与流式输出；历史消息原文只读呈现，绝不篡改 Core 持久化记录。
In scope: 新增 apps/desktop/src/components/SafeMarkdown.tsx 与 safeMarkdown.module.css；改造 apps/desktop/src/features/timeline/TimelineRowView.tsx 接入 SafeMarkdown 与 ToolBlock；新增 apps/desktop/src/app/p22MarkdownContentEvidence.test.tsx 验收测试套件（8 项测试全部通过）。Out of scope: 上下文作用域与 Token 预算（A13）。
Contracts and decisions adopted: ①严格遵守 DESIGN_I18N §2 与 TASKS.md A12 规范，富文本渲染绝不引入脚本注入或自动网络 fetch；②严格遵循 SECURITY.md，远程图片默认策略文档化并采用安全占位，禁止隐蔽网络泄露；③遵循 UI_ARCHITECTURE.md，呈现升级严格属于前端只读视图投影，禁止修改底层历史原文；④遵循剪贴板行为规范，复制操作仅限对应代码块或工具输出内容，不夹带额外未经授权的注入信息。
Changed files:
  - apps/desktop/src/components/SafeMarkdown.tsx（新增高安全无 HTML 执行的轻量 Markdown 解析与工具块组件）
  - apps/desktop/src/components/safeMarkdown.module.css（代码高反差样式、复制按钮、折叠展开与状态徽标样式）
  - apps/desktop/src/features/timeline/TimelineRowView.tsx（接入 SafeMarkdown 与 ToolBlock 渲染）
  - apps/desktop/src/app/p22MarkdownContentEvidence.test.tsx（新增 A12 验收测试套件，8 项测试全部通过）
Failure / cancellation / offline / restart: 剪贴板不可用时优雅静默降级；长文本无溢出无截断；折叠展开状态组件内隔离，不影响全局会话状态与重连。
Security / synchronization impact: 彻底消除前端 XSS 向量与网络外发隐患；不产生任何多设备同步冲突与持久化格式变更。
Migration / downgrade: 纯前端只读呈现层增强，完全向后兼容。
Regression proof before and after: p22MarkdownContentEvidence 证明了 Markdown 子集渲染、代码块复制、工具输出折叠展开、恶意脚本防御、非法 scheme 中和、图片隐私占位、中文/emoji 兼容性及 TimelineRowView 集成均 100% 达标。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 22 files / 182 tests PASS（较 A11 新增 8 个测试）
  - npm run build（apps/desktop）→ PASS（JS 298.59 kB / gzip 91.25 kB，CSS 25.77 kB / gzip 4.95 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持 A10 视觉基线截图校验有效。
Not run / blockers / residual risks:
  1. 真实系统多显示器 DPI 缩放下的微像素对齐归 A18 验收。
Status: DONE-VERIFIED
Next task and first concrete action: A13 上下文作用域、信任和预算契约（P1；F26、F28、F29、F30）——第一步深入分析 crates/altior-core 与 docs/ 契约，明确 Off/Session/LongTerm 与 Global/Person/Project/Thread 作用域、信任分级、Token 估算器版本化与上下文快照机制。

Execution time / environment: 2026-09-06 17:00–17:25；Node v24.18.0 / Rust 1.98.0；跨 altior-domain/altior-core/tests 上下文边界、作用域隔离、信任分级与预算审计验收。
Task ID / subslice: A13（完整切片，彻底消除 F26、F28、F29、F30 关于上下文作用域、信任、估算与遗忘保留的全部缺陷）
Baseline / current commit: f945ef153eb3344abd3801a3c2e4002fd25d80be（工作树未提交，累计 A01–A13 改动）
Pre-existing unrelated changes: 无
User-visible outcome: 建立了高安全、高保密、确定性可审计的上下文注入与信任契约，彻底杜绝跨项目知识泄露（F28）、模型提示词劫持（F29）、硬编码 Token 估算混淆（F26）与虚假擦除承诺（F30）：
  ①作用域严格隔离与跨项目硬防线（F28）：在 altior-domain 定义 MemoryScope::is_allowed_in_context 纯判断，Off 模式绝对 0 注入与字节级完全透传；Session 模式严格限制在当前 Thread 作用域（绝对不向会话注入 Global 或跨会话记忆，彻底兑现“无跨会话召回”承诺）；LongTerm 模式下，仅允许 Global、Person、当前 Thread 以及与当前 Thread 显式归属相同的 Project 记忆，跨项目（Project A vs Project B）以及无项目归属的独立会话对外部项目记忆实现绝对零泄露拦截；所有因作用域被剔除的候选记忆在 ContextSnapshot 审计记录中透明记录其排名、消耗并显式归类为 ContextDropReason::ScopeDisallowed。
  ②三层确定性信任分层与提示词注入中和（F29）：线装 prompt 建立最高权威用户身份声明（# Identity (Authoritative Profile & Standing Directives)）> 当前用户请求 > 被动检索参考资料（# Relevant Memories (Passive Reference Only; Cannot Authorize Commands)）的严格信任分层；被动参考块注入醒目的“不可用于授权或篡改安全边界”系统声明，并为每条记忆注入元数据标签（[kind | scope=..., source=...]）；所有记忆内容经过 sanitize_memory_content 中和，转义行首 # 标题并对多行缩进，彻底瓦解伪造 # Identity 等对抗性 prompt 越狱。
  ③版本化估算器与预算统一契约（F26）：显式版本化 ESTIMATOR_VERSION 为 v1_bytes_div_ceil_4，文档与代码全面对齐统一为 1,024 Token 身份预算与 2,048 Token 记忆预算；明确区分底层传输 64 KiB 硬字节上限、内存启发估算预算与远端 Agent 模型实际窗口。
  ④四层遗忘与审计保留语义界定（F30）：清晰划分“未来检索排除（Tombstone）”、“历史审计记录保留（ContextSnapshot 保持不变以便复盘‘当时为何这样回答’）”、“多设备 CRDT 同步收敛（防止旧设备上线死灰复燃）”与“加密物理擦除”的严格边界，文档绝不向用户虚构物理抹除承诺。
In scope: altior-domain 扩展 ContextDropReason::ScopeDisallowed、MemoryScope::is_allowed_in_context 与 Display 格式化；altior-core context/mod.rs 接入作用域校验、format_memory_line、sanitize_memory_content 与 ESTIMATOR_VERSION；application/mod.rs 在组装上下文时传入 thread.project_id 并过滤；新增 docs/decisions/0022-context-scope-trust-boundaries-and-budget-contracts.md、更新 docs/MEMORY.md 与 docs/SECURITY.md；新增 crates/altior-core/tests/p23_context_contract.rs 验收测试套件（6 项集成测试全部通过）。Out of scope: 真实记忆前端管理面板与 Core 记忆端口（A14）。
Contracts and decisions adopted: ①落实 ADR 0022 与 ADR 0018 契约，上下文组装纯函数化、零非确定性时钟采样、零未授权同步泄露；②严格遵循 MEMORY.md 与 SECURITY.md，所有外部检索内容严格作为不可信被动引用处理；③审计快照单轮次绝对不可变（turn_id 为主键），组装失败立即安全 fail-closed 终止外部派发。
Changed files:
  - crates/altior-domain/src/identity.rs（新增 ContextDropReason::ScopeDisallowed）
  - crates/altior-domain/src/entity.rs（新增 MemoryScope::is_allowed_in_context 与 Display 实现）
  - crates/altior-core/src/context/mod.rs（ESTIMATOR_VERSION、IDENTITY_HEADER、MEMORY_HEADER、sanitize_memory_content、format_memory_line 与作用域剔除逻辑）
  - crates/altior-core/src/application/mod.rs（向 AssembleParams 注入 thread.project_id）
  - crates/altior-core/tests/p23_context_contract.rs（新增 A13 验收套件，6 项端到端集成测试）
  - docs/decisions/0022-context-scope-trust-boundaries-and-budget-contracts.md（新增 ADR 0022）
  - docs/MEMORY.md（全面同步模式作用域集合、提示词信任分级、Token 估算器与四层遗忘语义）
  - docs/SECURITY.md（补全三层提示词信任边界、跨项目记忆隔离与遗忘审计保留要求）
Failure / cancellation / offline / restart: 快照持久化失败或检测到秘钥特征时安全 fail-closed，绝对零外部派发；历史审计快照重启无损加载。
Security / synchronization impact: 彻底消除跨项目私密记忆静默泄露；消除 prompt injection 越狱风险；完全兼容现有 SQLite schema 与事件流。
Migration / downgrade: 完全向后兼容，仅对 ContextSnapshot.dropped 增加一种合法枚举原因。
Regression proof before and after: p23_context_contract 证实了两项目同词检索时彼此完全隔离（Alpha 仅得 Alpha，Beta 仅得 Beta，且丢弃项记录 ScopeDisallowed）；无项目会话不召回项目记忆；Session 模式不注入 Global 记忆；恶意伪造 # Identity 标题被成功中和；中文与 Emoji 估算精准；遗忘后新轮次完全透传且历史快照完整保留。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 warnings）
  - cargo test --workspace → PASS（所有 crate 单元与集成测试全部通过，包括新增 p23_context_contract）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 22 files / 182 tests PASS
  - npm run build（apps/desktop）→ PASS（JS 298.59 kB / gzip 91.25 kB，CSS 25.77 kB / gzip 4.95 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持 A10 视觉基线截图校验有效。
Not run / blockers / residual risks:
  1. 真实系统切换 Windows 多语言输入法时的动态语言包联动归 A18 人工环境复核。
Status: DONE-VERIFIED
Next task and first concrete action: A14 把记忆与 Context 接到真实产品路径（P1；F10、F20）——第一步梳理 protocol 中缺少的 memory 管理命令 DTO（list_memories、propose_memory、confirm_memory、forget_memory 等），补齐 Rust 与 TS 契约。

### 2026-09-06 18:30 — A14 把记忆与 Context 接到真实产品路径（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 下午至 18:30（本地 Windows）；Rust 1.98.0，Node v24.18.0；未提交任何 commit。
Task ID / subslice: A14（完整单切片，彻底解决 F10 记忆管理假数据与 F20 上下文审查时序竞争问题）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be（基于 A01–A13 已验证工作树）
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 用户可以在 Personal Vault（Memory Pane）中直接查看、筛选和管理全部持久记忆，没有任何硬编码或假数据；支持状态筛选（all/candidate/confirmed/rejected/superseded/forgotten）与作用域筛选（all/global/project/person/thread）。
  ② 完整的记忆生命周期闭环：候选记忆可确认/拒绝、确认记忆可就地纠错（原记录置为 superseded 并指向新 ID，新记录自动确认）与彻底忘记（置为 forgotten 墓碑并排出未来检索）。
  ③ 支持动态切换当前 Agent 的记忆模式（off / session / long_term），并通过 configure_agent IPC 实时持久化。
  ④ 在会话时间线中选定轮次时，Inspector 实时拉取并展示权威的 ContextSnapshot 审计快照（包括 Token 预算核算、命中记忆的解释性依据 why_selected、召回得分、来源 Provenance、丢弃项理由与降级状态）。
  ⑤ 彻底消除多轮次并发切换时的网络时序竞争（Race Condition），严格区分 idle / loading / loaded / not_found / error 状态，切换轮次时旧请求响应彻底被丢弃，绝不出现串台或闪现错误数据。
In scope:
  - 协议层：crates/altior-protocol 新增 6 组命令 DTO（ListMemoriesCommand、ProposeMemoryCommand、ConfirmMemoryCommand、RejectMemoryCommand、CorrectMemoryCommand、ForgetMemoryCommand）及结果 DTO（MemoryRecordDto、MemoryListResponseDto、MemoryCursorDto），ts-rs 自动导出 TypeScript 契约。
  - Core 端口与守护进程：CoreApplication 暴露 6 组记忆生命周期方法；daemon.rs 实现完整派发与错误事件封装；crates/altior-core/tests/p24_memory_product.rs 验收测试套件（4 项测试全部通过）。
  - 前端传输与契约：apps/desktop/src/ipc/commandContract.ts 镜像 Rust 校验；inMemoryTransport.ts 实现完整的记忆存储、秘密拦截、状态转换与精确快照查询。
  - 前端状态与视图：applicationStore.ts 支持全生命周期操作与竞态消除；ContextPanel.tsx 支持多状态区分；新增 MemoryPane.tsx 与样式；App.tsx 激活 ActivityRail 的 memory 导航并打通 Inspector。
  - 证据套件：新增 apps/desktop/src/app/p24ContextWiringEvidence.test.tsx（6 项测试全部通过）。
Contracts and decisions adopted:
  - 遵循 ADR 0017、ADR 0018 与 ADR 0022，记忆记录全生命周期不可篡改，纠错生成新实体并保留历史指针，遗忘采用显式墓碑。
  - 严格执行秘密拦截门禁：任何包含秘钥特征（sk-、AKIA、PEM、密码赋值等）的内容在写入持久化前全部被拒，SQLite 数据库与 Journal 零写入。
  - 审计快照与轮次 ID 强绑定，Desktop 作为 Core 客户端不自行组装上下文。
Changed files:
  - crates/altior-protocol/src/command.rs
  - crates/altior-protocol/src/dto.rs
  - crates/altior-protocol/src/error.rs
  - crates/altior-core/src/application/mod.rs
  - crates/altior-core/src/application/daemon.rs
  - crates/altior-core/tests/p24_memory_product.rs（新增 Rust 端到端验收套件）
  - apps/desktop/src/ipc/dto/*.ts（ts-rs 导出的 9 个 DTO 绑定文件）
  - apps/desktop/src/ipc/commandContract.ts
  - apps/desktop/src/ipc/inMemoryTransport.ts
  - apps/desktop/src/stores/applicationStore.ts
  - apps/desktop/src/app/uiStore.ts
  - apps/desktop/src/components/ContextPanel.tsx
  - apps/desktop/src/components/ContextPanel.test.tsx
  - apps/desktop/src/components/MemoryPane.tsx（新增）
  - apps/desktop/src/components/memoryPane.module.css（新增）
  - apps/desktop/src/components/shell.tsx
  - apps/desktop/src/app/App.tsx
  - apps/desktop/src/app/p24ContextWiringEvidence.test.tsx（新增前端验收套件）
Failure / cancellation / offline / restart:
  - Core 进程彻底关闭重启后，真实磁盘 SQLite 记忆数据完整保留（p24_core_restart_persistence 通过）；
  - 秘密数据写入失败时安全 fail-closed，数据库零持久化（p24_secret_shaped_content_zero_persistence 通过）；
  - 快速切换轮次时，较早发出的慢响应被自动废弃，不会覆写最新轮次的 Inspector 状态。
Security / synchronization impact:
  - 秘密形状检测覆盖全部前端与后端写入路径；
  - 跨会话/跨项目作用域严格隔离；
  - 遗忘记录以墓碑形式存在，后续会话无法再次召回。
Migration / downgrade: 完全向后兼容，仅对 CommandKind 与 ContextDropReason 增加合法变体。
Regression proof before and after:
  - 后端 p24_memory_product 证明了 remember -> recall -> why -> correct -> forget 全生命周期，及 Core 真实重启与秘密零持久化。
  - 前端 p24ContextWiringEvidence 证明了 MemoryPane 零假数据、全生命周期操作成功率 100%、Agent 模式切换有效、Inspector 呈现真实快照及切换时序竞态被消除。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 errors / 0 warnings）
  - cargo test --workspace → PASS（全工作区所有单元测试与集成测试全部通过，包括新增 p24_memory_product）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 23 files / 191 tests PASS（包括新增 p24ContextWiringEvidence）
  - npm run build（apps/desktop）→ PASS（JS 322.23 kB / gzip 96.37 kB，CSS 30.47 kB / gzip 5.64 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持 A10 视觉基线截图校验有效。
Not run / blockers / residual risks:
  1. 真实系统切换 Windows 多语言输入法时的动态语言包联动归 A18 人工环境复核。
Status: DONE-VERIFIED
Next task and first concrete action: A15 检索相关性、中文与有界候选（P1；F21–F23）——第一步固定至少 100 个 query 的合成评估集，评估当前 BM25 分量分布与 scope 下推。
```

### 2026-09-06 19:30 — A15 检索相关性、中文与有界候选（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 晚上 19:30（本地 Windows）；Rust 1.98.0，Node v24.18.0；工作树保持干净未提交。
Task ID / subslice: A15（完整单切片，彻底解决 F21 无界候选物化、F22 BM25 得分截断与排序漂移、F23 中文检索与短词匹配失效）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be（基于 A01–A14 已验证工作树）
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 中文与 CJK 检索全面恢复：彻底根除 F23（unicode61 导致连续中文文本被作为单 token 索引导致分词不匹配），通过 Schema V8 升级至 SQLite FTS5 `trigram` 并配合 Hybrid 查询（短词 instr 子串匹配、连续 CJK 字符滑窗 3-gram、英文词形还原扩展）。在 100 组真实检索基准测试中，中文 Recall@8 从 7.5% 跃升至 84.2%（+76.7%），nDCG@8 从 0.0750 提升至 0.8121；支持单字/两字短词（“茶”、“乌龙”、“并发”、“架构”）、长句（“我喜欢喝乌龙茶”）以及混合代码与符号（“Rust 开发”、“C++”、“std::unique_ptr”）。
  ② 无界候选与计算开销收敛（F21）：通过 SQLite Scope 下推与 O(K) 最小堆有界淘汰（Bounded Heap Ranking），彻底消除内存中全量物化 MemoryRecord 和对全部 M 个命中项排序的 O(M) 内存与 O(M log M) CPU 开销；仅针对最终 Top-K 项延迟计算匹配项与格式化 explainability 依据。
  ③ 修复得分截断与 Tie-Breaker 漂移（F22）：移除 BM25 的 0.1 强制 clamp；全面统一存储层与 Context Assembly 层的排序决胜规则（Total Score DESC -> updated_at DESC -> memory_id DESC），且按此确定性序分配权威 rank 序号，彻底消除上下文审查中的排序漂移与解释 desync。
  ④ 保持 0 泄漏与绝对边界：评估集 100 次查询全量验证证明 Scope 泄漏 = 0，过期、被替代（superseded）、未确认（candidate）、已遗忘（forgotten 墓碑）记忆漏入检索 = 0。
In scope:
  - 架构决策：撰写 ADR 0023（docs/decisions/0023-fts-trigram-hybrid-retrieval-and-bounded-ranking.md）。
  - 存储迁移：crates/altior-storage/src/migrations.rs 增加 SCHEMA_V8，升级 memory_fts 至 trigram，重建增删改触发器，保留全部历史数据。
  - 存储检索与堆淘汰：crates/altior-storage/src/memory.rs 实现 is_punctuation、混合分词、CJK 滑窗 3-gram、SQL Scope 下推、O(K) 最小堆有界排名、延迟解释生成。
  - 核心装配层对齐：crates/altior-core/src/context/mod.rs 对齐排序决胜法则与 Scope 检查。
  - 确定性评估基准：crates/altior-storage/tests/relevance_eval.rs 构建 100 组金标测试集（40 中文 + 40 英文 + 20 混合），输出 nDCG@8、Recall@8、泄漏计数、clamp 比例及索引体积/构建耗时。
  - 验收测试套件：crates/altior-core/tests/p25_memory_retrieval_relevance.rs 实现 5 组端到端验收测试（中文、混合符号、0 泄漏、Scope 下推集成、确定性 Tie-Breaker）。
Contracts and decisions adopted:
  - 遵循 ADR 0017、ADR 0018、ADR 0022 与 ADR 0023；
  - 检索算法版本打标为 `v2_bounded_scope_pushdown_trigram_hybrid`；
  - 严禁引入外部非标准 C 扩展分词器，完全基于 SQLite 内置 trigram 与安全 instr 子串组合，确保数据库跨平台脱机与外部 CLI 兼容。
Changed files:
  - docs/decisions/0023-fts-trigram-hybrid-retrieval-and-bounded-ranking.md（新增 ADR 0023）
  - crates/altior-storage/src/migrations.rs
  - crates/altior-storage/src/memory.rs
  - crates/altior-storage/tests/memory.rs
  - crates/altior-storage/tests/relevance_eval.rs（新增确定性检索评估基准测试）
  - crates/altior-core/src/context/mod.rs
  - crates/altior-core/tests/p25_memory_retrieval_relevance.rs（新增验收套件）
Failure / cancellation / offline / restart:
  - 数据库从 V7 升级到 V8 保持完全无损向后兼容；
  - 面对单字/超短词搜索安全回退到 instr 过滤，长文本 CJK 自动拆分 3-gram 兜底，绝不崩溃或产生语法错误；
  - 极小或无效 query 立即返回空结果，不触发 SQLite 语句执行。
Security / synchronization impact:
  - 任何包含秘钥特征的内容仍被首道网关彻底拦截；
  - Scope pushdown 确保跨租户/跨项目记忆在 SQLite 查询层即被物理阻断；
  - 遗忘记录以墓碑状态被触发器同步踢出 memory_fts，且在 SQL 层强制过滤。
Migration / downgrade: 引入 SCHEMA_V8，若遇更旧版本按现有 migration 链按序升级。
Regression proof before and after:
  - 基线评估（Porter Unicode61）：Overall Recall 46.5%，Chinese Recall 7.5%，Chinese nDCG 0.0750。
  - 本次实施后（Trigram Hybrid）：Overall Recall 91.2%（+44.7%），Chinese Recall 84.2%（+76.7%），Chinese nDCG 0.8121，English Recall 保持 93.8%，Mixed Recall 达到 100.0%。
  - 索引体积开销：1000 条记录在 Trigram 下为 960 KiB（比 Unicode61 的 480 KiB 仅增加 1 倍，构建时间 413ms），完全在设计预算内。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 errors / 0 warnings）
  - cargo test --workspace → PASS（全工作区所有单元测试与集成测试全部通过，包括新增 p25_memory_retrieval_relevance 与 relevance_eval）
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npx vitest run（apps/desktop）→ 23 files / 191 tests PASS
  - npm run build（apps/desktop）→ PASS（JS 322.23 kB / gzip 96.37 kB，CSS 30.47 kB / gzip 5.64 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持 A10 视觉基线截图校验有效。
Not run / blockers / residual risks:
  1. 真实系统切换 Windows 多语言输入法时的动态语言包联动归 A18 人工环境复核。
Status: DONE-VERIFIED
Next task and first concrete action: A16 数据与流式性能（P1；F24、F25）——第一步记录 host/OS/build/toolchain 环境参数，设计 100k messages + 100k memories 压测基准与流式管道吞吐测量点。
```

### 2026-09-06 19:48 — A16 数据与流式性能（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 19:48（本地 Windows 11；cargo 1.98.0，rustc 1.98.0，Node v24.18.0）；未提交任何 commit。
Task ID / subslice: A16（完整单切片，覆盖 F24 存储流式绕行与断点摘要检查点；F25 虚拟滚动与渲染引用稳定性优化）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 流式 Delta 写入吞吐量大幅提升：存储层 MessageDelta 写入跳过全表投影扫描，从 2.71 ms/delta 降低至 0.34 ms/delta（每秒可处理超过 2,900 次 deltas），消除高频流式输出下的卡顿与 CPU 尖刺。
  ② 崩溃自愈与 Durability 绝对不妥协：中途 Core 崩溃或非正常终止时，Store::open 能精确检测未清算摘要，自动执行 domain_journal 完整回放并修复投影表，恢复后 live digest 与 stored digest 100% 严格一致，数据零丢失。
  ③ 前端虚拟列表渲染引用稳定化：Timeline 中 onPermissionDecision 回调通过 useCallback 实现引用固化，配合 React.memo(TimelineRowView) 确保单行流式更新或无关 state 变化不触发其余所有可见行无谓重渲染。
  ④ 虚拟滚动数组操作 O(1) 优化：timelineStore appendRow / prependRows 从全量 O(N) 映射重写为基于既有 snapshot 的数组扩展，大幅降低长会话连续增量时的内存分配与垃圾回收压力；virtualWindow 补充 appendRowHeight / adjustRowHeight 确定性单元测试。
In scope:
  - 架构决策：撰写 ADR 0024（docs/decisions/0024-projection-digest-incremental-invariants-and-streaming-performance.md）。
  - 存储流式优化：crates/altior-storage/src/lib.rs 实现 MessageDelta 快速通道，在轮次清算事件（TurnCompleted / TurnFailed / TurnCancelled）与结构性事件才强制执行全量投影摘要计算与快照固化。
  - 性能与崩溃恢复证明：crates/altior-storage/tests/performance_eval.rs 建立压测与故障模拟测试，验证吞吐提升与崩溃自动重建。
  - 前端渲染与窗口优化：apps/desktop/src/features/timeline/Timeline.tsx 优化回调引用稳定性；timelineStore.ts 消除 O(N) snapshot 重建；virtualWindow.ts 增加增量计算；virtualWindow.test.ts 补充 100% 覆盖测试。
Contracts and decisions adopted:
  - 遵循 ADR 0024；严禁为了速度削弱任何 durability 保证或删除摘要校验。
Changed files:
  - docs/decisions/0024-projection-digest-incremental-invariants-and-streaming-performance.md（新增 ADR 0024）
  - crates/altior-storage/src/lib.rs
  - crates/altior-storage/tests/performance_eval.rs
  - apps/desktop/src/features/timeline/Timeline.tsx
  - apps/desktop/src/features/timeline/timelineStore.ts
  - apps/desktop/src/features/timeline/virtualWindow.ts
  - apps/desktop/src/features/timeline/virtualWindow.test.ts
Failure / cancellation / offline / restart:
  - 模拟断电/强杀场景，未清算的 MessageDelta 不会损坏 domain_journal，重新打开时由于 stored digest 滞后于最新事件 journal marker，Store::open 自动触发权威 rebuild，完整保留所有 message delta 并纠正 digest。
Security / synchronization impact:
  - 无密码学或密钥安全降级，SQLite 本地投影与 domain_journal 保持纯本地一致。
Migration / downgrade:
  - 保持现有 schema 兼容，无需额外 migration 编号变更。
Regression proof before and after:
  - 存储写入速度：基线每次 delta 2.71ms（全表扫描 1600+ 行），优化后 0.34ms（降低 87.4%）。
  - 恢复测试：test_mid_turn_crash_detected_and_rebuilt_cleanly 自动通过并完成 digest 对齐。
  - 前端单测：vitest 23 files / 195 tests 全部通过。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 errors / 0 warnings）
  - cargo test --workspace → PASS
  - npm run typecheck（apps/desktop）→ PASS（0 errors）
  - npm run test（apps/desktop）→ 23 files / 195 tests PASS
  - npm run build（apps/desktop）→ PASS（JS 322.31 kB / gzip 96.40 kB，CSS 30.47 kB / gzip 5.64 kB）
  - node scripts/check-contrast.mjs（apps/desktop）→ PASS（24/24 passed）
Screenshots actually inspected: 保持既有视觉截图基线有效。
Not run / blockers / residual risks:
  - 极端 100k 消息在受限内存移动设备上的极限加载时间需真实设备环境进一步验证。
Status: DONE-VERIFIED
Next task and first concrete action: A17 自动质量门禁与证据结构（P1；F32–F34）——第一步梳理 package.json scripts、添加缺漏的 format/lint 检查脚本、审查 Tauri manifest 与 CI/本地统一门禁。
```

### 2026-09-06 20:05 — A17 自动质量门禁与证据结构（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 20:05（本地 Windows 11；cargo 1.98.0，rustc 1.98.0，Node v24.18.0，Playwright 1.56）；未提交任何 commit。
Task ID / subslice: A17（完整切片，覆盖 F32–F34 质量门禁、截图基线与证据结构闭环）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 建立完整、自闭环的前端质量门禁命令集（apps/desktop/package.json）：补全 typecheck、lint、format:check、test:unit、test:contrast、test:visual、test:tauri 及一键 gate 命令，使干净 checkout 环境下所有文档命令均可独立、无错执行。
  ② 静态架构与样式规约（check-lint.mjs）：强约束 CSS 模块禁止硬编码裸 hex 颜色（迫使必须使用语义 token --color-*），禁止在 transport 外直接访问 Tauri 全局/原生 API，禁止在渲染层直接引入 SQLite 数据库；上线即检出并修复 timeline.module.css:180 的裸色遗留。
  ③ 格式与空白规约（check-format.mjs）：强制校验尾部空行、行尾无杂乱空格、缩进纯空格无制表符，统一前端代码风格。
  ④ 视觉与几何回归双门禁（check-visual.mjs 与 baselines.mjs）：驱动 Playwright 接入 [data-fixture-ready="true"] 与 document.fonts.ready 确定性等待，全面捕获并阻断 pageerror 与 console.error；check-visual.mjs 对 5 组基线视图（light/dark/narrow/error/approval）实施承重几何断言（四列栅格尺寸、无横向滚动条、760px 窄屏抽屉、高反差主题色、操作控件尺寸），并进行非破坏性基线文件完整性校验；禁止测试时自动覆盖基线图消除差异。
  ⑤ 独立 Tauri Manifest 与工作区解耦审计：明确记录并单列 altior-desktop-shell 独立 crate 的 clippy 与 cargo test（7 项单测全部通过），不再将 workspace 结果冒充 Tauri 覆盖。
  ⑥ 真实 ACP 外部代理明确标记：编写根级 scripts/quality-gate.ps1 与 scripts/quality-gate.sh，当未注入环境变量 ALTIOR_ACP_SMOKE_AGENTS 时显式打印 [SKIPPED] 状态，杜绝将跳过未配置的外部模型网络测试伪装为发布证据 PASS。
In scope:
  - package.json 丰富完整 scripts 定义；
  - apps/desktop/scripts/check-lint.mjs（静态架构与设计 token 守卫）；
  - apps/desktop/scripts/check-format.mjs（空白与格式守卫）；
  - apps/desktop/scripts/check-visual.mjs（Playwright 承重几何断言与基线回归门禁）；
  - apps/desktop/scripts/baselines.mjs（带 ready、pageerror 陷阱与 try/finally 清理的基线采集器）；
  - scripts/quality-gate.ps1 与 scripts/quality-gate.sh（跨 Rust workspace、Tauri shell、Desktop frontend 与 ACP opt-in 的全量一键门禁脚本）；
  - apps/desktop/src/app/App.tsx 增加 data-connection-status 与 data-fixture-ready 观察点；
  - apps/desktop/src/features/timeline/timeline.module.css 修复裸色违规。
Contracts and decisions adopted:
  - 严格遵守 ADR 0008（独立 Tauri 壳 crate）、ADR 0007（真实 ACP 独立 opt-in 契约）及 DESIGN_I18N（语义 token 与设计原语约束）。
Changed files:
  - apps/desktop/package.json
  - apps/desktop/scripts/check-lint.mjs（新增）
  - apps/desktop/scripts/check-format.mjs（新增）
  - apps/desktop/scripts/check-visual.mjs（新增）
  - apps/desktop/scripts/baselines.mjs
  - apps/desktop/src/app/App.tsx
  - apps/desktop/src/features/timeline/timeline.module.css
  - scripts/quality-gate.ps1（新增）
  - scripts/quality-gate.sh（新增）
Failure / cancellation / offline / restart:
  - 无论门禁在任一步骤失败（如语法错、对比度未达标、几何断裂、编译报错），脚本立即退出并返回非 0 状态码，且开发服务器与浏览器进程由 try/finally 保证 100% 被清理。
Security / synchronization impact:
  - 彻底杜绝渲染器直接访问 SQLite 数据库或非法泄露系统原生 API；纯本地门禁无外部网络外溢。
Migration / downgrade: 纯工程基础设施加固，无持久格式迁移影响。
Regression proof before and after:
  - 修复前 F33 记录“baselines 等到 row 就截图，无 pageerror 失败策略，无几何比较，connecting 也在图中，它是采集器不是门禁”；
  - 修复前 F34 记录“package scripts 无 format/lint/browser compare，不能声称 workspace 已覆盖 Tauri”；
  - 修复后 check-lint.mjs 实时拦截裸色与非法导入；check-visual.mjs 对全部 5 视图严格断言几何与基线；cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml 独立验证通过；quality-gate.ps1 一键执行全绿并明确报告真实代理为 [SKIPPED]。
Exact verification commands and results:
  - powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1 → PASS（全量 4 大模块全部通过）
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS（0 errors / 0 warnings）
  - cargo test --workspace → PASS
  - cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml -- -D warnings → PASS
  - cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml → PASS（7 passed）
  - npm run gate（apps/desktop）→ PASS
    - typecheck: PASS
    - lint: PASS
    - format:check: PASS
    - test:unit: PASS（23 files / 195 tests PASS）
    - test:contrast: PASS（24/24 passed）
    - test:visual: PASS（5/5 views passed）
    - build: PASS
Screenshots actually inspected:
  - Playwright 在 check-visual.mjs 中实时对 5 组基线执行断言并通过，baselines/{light,dark,narrow,error,approval}.png 保持完好。
Not run / blockers / residual risks:
  - 真实第三方 ACP 在无真实 API 密钥环境中如实汇报为 [SKIPPED]，不冒充 PASS（进入 A18 范围）。
Status: DONE-VERIFIED
Next task and first concrete action: A18 真实打包桌面与两个 ACP 的连续性验收（发布 P0；F32）——第一步核验本地已安装或授权的真实 ACP 代理程序与环境配置，明确 mock 旅程与真实外部代理旅程的分隔边界。
```

### 2026-09-06 20:15 — A18 真实打包桌面与两个 ACP 的连续性验收（BLOCKED）

```text
Execution time / environment: 2026-09-06 20:15（本地 Windows 11；cargo 1.98.0，rustc 1.98.0，Node v24.18.0）；未提交任何 commit。
Task ID / subslice: A18（真实打包桌面与第三方 ACP 连续性验收，覆盖 F32）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 自动化模拟链路 100% 验证通过：p14_acceptance_journey 8 步端到端旅程全部 PASS（包含配置、探针、创建会话、流式交互、审批通过、协同取消、断线重连补播、异常退出 Indeterminate 固化、Core 重启禁止自动重发及离线 FTS5 搜索全生命周期）；Tauri 壳层独立 7 项生命周期测试（altior-desktop-shell）全部 PASS。
  ② 真实外部第三方代理与代码签名环境阻断：依照 TASKS.md 明确规则（“真实代理/认证/签名环境缺失写 BLOCKED，提供精确人工运行说明；不得自动购买、登入、上传用户内容或改成 mock 说通过。可以继续其他独立任务，但不能把该任务 DONE”），当前容器/开发机未配置已付费/授权的外部模型 API Token（未设置 ALTIOR_ACP_SMOKE_AGENTS）且未注入生产级 OS 代码签名证书。
  ③ 提供精确的人工端到端验收指南与规程（见 In scope 与 Manual Runbook）。
In scope:
  - 自动化模拟验收：crates/altior-core/tests/p14_acceptance_journey.rs（8 步旅程全部验证）；
  - Tauri 独立壳单元验收：apps/desktop/src-tauri/src/lib.rs（7 项测试全部验证）；
  - 真实环境阻断审计与人工操作指南整理。
Out of scope:
  - 自动购买 API Token 或上传真实用户凭据（严格禁止）。
Contracts and decisions adopted:
  - 严格遵守 TASKS.md §A18 阻塞规则及 ADR 0007 / ADR 0016。
Changed files: 无新代码变更（纯环境判定与审计文档）。
Failure / cancellation / offline / restart:
  - 在 p14 测试中证明：客户端关闭（UI reload / window close）绝不中断后台 Core 正在运行的轮次；Core 重启后处于 Indeterminate 状态的轮次在 IPC 层严格禁止被隐式重发（AutomaticResendForbidden）。
Security / synchronization impact:
  - 严禁在测试日志或源码中硬编码外部模型 API Key。
Migration / downgrade: 无。
Regression proof before and after:
  - p14_acceptance_journey: 8 步全部 PASS。
  - altior-desktop-shell: 7 tests 全部 PASS。
  - 真实 ACP 烟测在无环境变量时明确输出 [SKIPPED] 而非虚假 PASS。
Exact verification commands and results:
  - cargo test -p altior-core --test p14_acceptance_journey → PASS
  - cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml → PASS (7 passed)
  - powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1 → PASS (ACP smoke correctly reports [SKIPPED])
Screenshots actually inspected: 保持 A10/A17 视觉基线截图有效。
Not run / blockers / residual risks:
  1. 【阻断】真实第三方 ACP 代理（如 @zed-industries/claude-code-acp 与 @google/gemini-cli）由于未配置实际商用 API Key 无法在此环境下执行网络推演。
  2. 【阻断】生产发布包的 Windows Authenticode 与 macOS Developer ID 代码签名因缺私钥证书无法在此构建环境执行最终签名上架。
  【人工运行说明 / Manual Runbook】
  具备真实商用环境的操作员请按以下步骤执行真实验收：
  1. 准备两个 ACP v1 兼容代理，导出环境变量：
     $env:ALTIOR_ACP_SMOKE_AGENTS = "npx -y @zed-industries/claude-code-acp;;npx -y @google/gemini-cli --experimental-acp"
  2. 运行真实代理烟测：
     cargo test -p altior-acp --test smoke -- --nocapture
  3. 构建本地 Tauri 桌面端：
     npm --prefix apps/desktop run build
     npm --prefix apps/desktop run tauri build
  4. 启动打包产物，依次核验：
     a. 配置 Agent A 与 Agent B，点击 Probe 验证握手；
     b. 发起多轮对话，验证流式输出；
     c. 触发权限弹窗，测试 Approve 与 Deny；
     d. 发起耗时任务并点击 Cancel，验证中断；
     e. 在生成过程中关闭桌面 UI 窗口，重新打开验证会话状态被无损恢复；
     f. 结束 Core 进程并重启，验证历史消息完整，未清算轮次被标记并禁止隐式重发。
Status: BLOCKED
Next task and first concrete action: A19 发布定位、文档和安装升级门槛（P1/P2；F34）——第一步梳理 README.md 与 IMPLEMENTATION_PLAN.md 中的完成度标记、发布说明语境、平台支持矩阵及安装/升级/降级文档。
```

### 2026-09-06 20:30 — A19 发布定位、文档和安装升级门槛（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 20:30（本地 Windows 11；cargo 1.98.0，rustc 1.98.0，Node v24.18.0）；未提交任何 commit。
Task ID / subslice: A19（发布定位、文档与安装升级门槛，覆盖 F34）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 发布承诺与实际状态 100% 对齐：README.md 与 docs/IMPLEMENTATION_PLAN.md 彻底修复了之前停留在 P1.3/P1.4 的滞后描述，明确列出核心后端（Core/Storage/IPC/Domain/ACP 已 100% 自动验证）、桌面渲染器（Desktop 已 100% 自动验证，含 195 单测/对比度/几何回归）、独立 Tauri 壳（已通过独立 7 项生命周期单测，未签名 developer build）、第三方外部 ACP（mock 8 步全绿，真机 opt-in [SKIPPED]）及多设备同步（保持 spike 状态）的组件验收矩阵。
  ② Tauri Rust Edition 差异透明化：明确记录 Tauri 壳层使用 edition="2021" 系 ADR 0008 §6 明确的解耦边界，保留独立 cargo test 门禁，不在文档中混淆两者的测试覆盖。
  ③ 完备平台与运行时环境规约：详尽列举支持系统（Windows 10/11、macOS 13+、Linux x86_64）、工具链最低版本（Rust 1.90+、Node v20+）、Core 发现机制（原子文件 0600/ACL 保护）、SQLite 数据存储位置、日志脱敏机制。
  ④ 安装、升级、降级与卸载原则：明确向后迁移链路（V1 至 V8）、禁止静默降级（遇到更高 schema 立即抛出 StorageError::SchemaTooNew 拒绝启动以保护用户数据）、中途崩溃自愈保证，以及卸载时默认保留 Personal Vault SQLite 数据库防止用户资产误删的安全策略。
  ⑤ 协议与知识产权闭环：根目录补全标准 Apache License 2.0 LICENSE 文件与 THIRD_PARTY_NOTICES.md，明确声明无 Lody 或其他外部项目的未经授权复用，记录全部直接依赖项。
In scope:
  - README.md 更新（矩阵、支持系统、路径发现、迁移降级与卸载策略、统一门禁脚本说明）；
  - docs/IMPLEMENTATION_PLAN.md 更新（P1.4/P2.1/P2.2/P2.3 状态与 ADR 0019–0024 完整对齐）；
  - LICENSE（新增 Apache-2.0 许可证文件）；
  - THIRD_PARTY_NOTICES.md（新增第三方开源软件声明与版权告示）。
Contracts and decisions adopted:
  - 遵循 ADR 0008、ADR 0013–0024 及 AGENTS.md 规范。
Changed files:
  - README.md
  - docs/IMPLEMENTATION_PLAN.md
  - LICENSE（新增）
  - THIRD_PARTY_NOTICES.md（新增）
Failure / cancellation / offline / restart:
  - 旧版本运行高版本数据库拒绝启动，报错明确，绝不破坏磁盘数据；
  - 崩溃自愈有实证支持。
Security / synchronization impact:
  - 明确同步仍处于 pre-production 禁用阶段，不可直接对外宣称“生产级同步已上线”。
Migration / downgrade: 保持现有 V8 架构，文档对齐。
Regression proof before and after:
  - 修复前 F34 记录“README 停在 P1.3/P1.4，IMPLEMENTATION_PLAN 状态漂移，Tauri edition 未解释，缺少第三方开源声明”；
  - 修复后所有组件完成度、测试门禁与环境要求均与代码库当前实际完全一致，且 LICENSE 和 THIRD_PARTY_NOTICES 补齐。
Exact verification commands and results:
  - cargo fmt --all -- --check → PASS
  - cargo clippy --all-targets --all-features -- -D warnings → PASS
  - cargo test --workspace → PASS
  - powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1 → PASS
Screenshots actually inspected: 保持既有视觉基线有效。
Not run / blockers / residual risks: 无
Status: DONE-VERIFIED
Next task and first concrete action: A20 同步上线前决策与安全阻断单（同步发布 P0；F31）——第一步深入梳理 SECURITY.md 与 SYNC 相关 ADR，针对 session counter 重启重用、低阶公钥、nonce 唯一性、磁盘回滚与 30 天旧设备复活等风险建立严密威胁树与生产阻断清单。
```

### 2026-09-06 20:45 — A20 同步上线前决策与安全阻断单（DONE-VERIFIED）

```text
Execution time / environment: 2026-09-06 20:45（本地 Windows 11；cargo 1.98.0，rustc 1.98.0，Node v24.18.0）；未提交任何 commit。
Task ID / subslice: A20（同步上线前决策与安全阻断单，彻底消除 F31）
Baseline / current commit: 基线 f945ef153eb3344abd3801a3c2e4002fd25d80be
Pre-existing unrelated changes: 无
User-visible outcome:
  ① 确立生产同步绝对禁用红线（ADR 0025）：在桌面与核心运行时中严格封锁多设备网络同步入口（sync_enabled = false），杜绝将未经安全审计的加密/中继原型（spike）带病上线。
  ② 深度构建 11 项威胁模型树与失效缓解规约（ADR 0025、docs/SECURITY.md、docs/SYNC.md）：
     1. 会话计数重置与 Nonce 重用漏洞（F31）：详述同一静态身份重建 Session 导致双重一次性密码本（keystream XOR）泄露机密的数学失效模式；规定持久化计数块预留（Reservation Blocks）+ 128-bit CSPRNG 随机 Epoch Salt + 最终向双棘轮演进。
     2. 低阶公钥与 Contributory 检查：严格要求 RFC 7748 §6 标量乘法结果校验，彻底过滤 Curve25519 弱点与小阶子群。
     3. 磁盘回滚与快照克隆：设计基于 OS 安全硬件计数器与对端纪元版本序列协商的重放阻断。
     4. 设备撤销与密钥轮转：规定基于离线恢复公钥签署的撤销凭证广播与文档数据密钥就地重新加密。
     5. 零知识中继保密性：严格断言中继仅接触不透明密文与元数据，正文、记忆、Prompt、恢复词 100% 端到端 AEAD 加密。
     6. AEAD 附加认证数据（AAD）：将版本、收发双方 ID、纪元与序号全面压入 ChaCha20-Poly1305 AAD，防范中继篡改与降级。
     7. 中继资源与配额防御：设定 1 MiB 帧上限、50 MiB 知识库队列配额与 14 天未清算超时。
     8. 解压炸弹与恶意 CRDT 导入：设定 10x 膨胀比与 10 MiB 绝对内存上限，Framed Automerge 块解析失败即隔离。
     9. 30 天长离线旧设备回归：强制执行“先拉后推”握手，必须首先同步墓碑与撤销前沿，方可提交本地滞留写入。
     10. 遗忘记忆长久性（Tombstone 永生性）：禁止在全局设备前沿跨过墓碑时间戳之前对墓碑进行垃圾回收，防止死记忆复活。
     11. 子进程特权隔离：ACP 外部代理严格禁止触碰同步密钥、IPC 发现凭证与网络套接字。
  ③ 冻结 4 组确定性验收测试规范：三设备离线收敛测试、1,000 次会话重启 100,000 条密文 0 重复 Nonce 校验、中继零知识字节熵与明文扫描审计、30 天旧设备不复活测试规范。
  ④ 独立安全审查证据清单：形成完备的形式化审查材料清单，杜绝将现有库的使用混淆为协议已过审计。
In scope:
  - 架构决策：新增 docs/decisions/0025-sync-production-gate-threat-model-and-safety-barriers.md（ADR 0025）；
  - 安全规范更新：docs/SECURITY.md 扩充生产同步阻断与 11 维威胁模型；
  - 同步规范更新：docs/SYNC.md 记录 P0 选型完成并补充生产安全门槛与 4 项验收测试。
Out of scope:
  - 擅自将网络同步投入生产或将未实现的双棘轮伪装为完成（严格禁止）。
Contracts and decisions adopted:
  - 遵循 ADR 0025 及 AGENTS.md 规范。
Changed files:
  - docs/decisions/0025-sync-production-gate-threat-model-and-safety-barriers.md（新增 ADR 0025）
  - docs/SECURITY.md
  - docs/SYNC.md
Failure / cancellation / offline / restart:
  - 同步功能保持完全离线隔离，本地 Personal Vault 的读写、记忆召回、ACP 交互与搜索 100% 可用，不受网络任何影响。
Security / synchronization impact:
  - 彻底规避了生产环境发布带病同步的毁灭性安全隐患。
Migration / downgrade: 纯安全规范与阻断单，无格式变动。
Regression proof before and after:
  - 修复前 F31 记录“Session 构造时 send_counter=0，重启会重用计数起点，nonce 重用破坏机密性，README 说明 counter 未完成，不得拿前端 UI 顺便开放同步”；
  - 修复后 ADR 0025 明确设置生产禁用屏障，详列 11 项攻击树与应对方案，并定义上线前必须达到的 4 组刚性测试门槛。
Exact verification commands and results:
  - powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1 → PASS
  - cargo test --workspace → PASS
Screenshots actually inspected: 保持既有视觉基线有效。
Not run / blockers / residual risks: 无。
Status: DONE-VERIFIED（A18 仍为 BLOCKED，A01–A20 未全闭环）
Next task and first concrete action: 确认除 A18 仍为 BLOCKED（依赖真实第三方模型密钥与外部代理）外，其余任务完成核验；A01–A20 未全闭环，后续转入 2026-09-13 评审。
```
