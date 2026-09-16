# 具体执行任务包

所有任务初始状态为 TODO。A 编号是本评审工作项，不等于项目已有 P0/P1/P2 阶段。每次只交付一个可评审工作项或其中一个完整子切片；不要一次执行 20 项生成一个巨大提交。

根 AGENTS、accepted ADR、长期契约优先。本包提案冲突时先更新决策，不能直接覆盖约束。没有用户要求不要自动提交、推送、发布或改真实 Vault。每项写 `Outcome/Acceptance/In scope/Out of scope/Contracts/Failure/Security/Tests/Migration` 简报及证据。

## A01 — 冻结真实边界并修正 ID（P0；F01、F32）

**依赖：** 无。**目标：** UI 生成的每种正式命令都能被真实 Rust DTO 解码，身份不会随 UI 重启碰撞。

**主要位置：** `altior-domain/src/id.rs`、`altior-protocol/src/command.rs,dto.rs`、`apps/desktop/src/stores/applicationStore.ts`、`ipc/inMemoryTransport.ts`、协议 fixture/export tests。

**具体步骤：**
1. 枚举 store 所有命令的 operation/entity ID、可选字段、返回形状；核对真实 serde 边界，不只看生成 TS 的 string 类型。
2. 增加 frontend→Rust fixture 校验：保存当前真实 command，证明非法 body 被拒绝；将它变成合法请求验收。fake 同样校验。
3. 明确 Core 拥有实体 ID 生成；已有 optional ID 可传 null，由响应取得实际 ID。operation ID 生成/分配放明确边界，记录跨 UI 重启唯一性和重复命令语义；任何新机制先 ADR。
4. 前端只使用已确认返回的真实 ID；不要用 `Math.random` 加字符串或时间戳计数假造已创建事实。正常重试保持逻辑 operation identity，不为同一可能执行的 prompt 生成新 ID 自动重发。
5. Rust-owned TS 重新生成，不手改 DTO 文件；验证双向 fixture 和生成结果无漂移。

**验收：** configure/create/open/list/search/history/start/cancel/permission/context/identity 命令全覆盖；中文名称、同毫秒并发和 renderer 重启不造成冲突；重复 operation 不执行两次；非法命令明确拒绝。

**不做：** 放宽 domain ID 验证、引入新 harness、改同步 ID 格式。**失败/安全：** 未生成合法请求就不可发；错误不携带用户 prompt。**迁移：** 不修改已发布记录 ID；若已有坏记录存在，先证明其可能进入持久层，再设计独立修复工具，禁止扫描改写真实用户数据。

## A02 — 连接入口、Tauri 桥接与订阅生命周期（P0；F02）

**依赖：** A01 的契约测试。**目标：** dev fixture、正式 WebView、不可用浏览器三种环境行为明确，无永久假 connecting。

**位置：** `main.tsx`、`ipc/tauriTransport.ts`、`platform/tauri/`、`applicationStore.init`、`App.tsx` effect、Tauri capability/config tests。

**步骤：**
1. 把入口 transport 决策收敛为一个工厂；Vite 使用明确 DEV 判断，不用 false 和 nullish fallback 混用。
2. fixture 在明确开发/测试入口注入；普通生产浏览器显示不支持/不可用，不自动 fake 成功。
3. 在 `platform/tauri` 使用受支持 invoke/listen API 与最小能力；保留 withGlobalTauri=false 的边界，不开全局对象补洞。
4. subscribe/connect/init 全部失败路径进入状态机；异步 listener 注册要有 ready/取消处理，先订阅完成再发送会产生活动的命令。
5. effect 清理去订阅/释放客户端连接，不能关闭 Core 活动；StrictMode 二次 effect 不重复监听、不重复命令、不丢第一条事件。

**验收：** 真 Vite 页面无 unhandled rejection；fixture ready；mock WebView bridge 缺 invoke/listen 清楚报错；真实 Tauri+隔离 Core 握手；关闭再开 renderer 监听数有界；快速 subscribe/unsubscribe 不泄漏。

**不做：** 把 Tauri 私有 internals 当永久协议、扩大 shell/fs 权限。**迁移：** 无持久格式变更预期。

## A03 — 权威实体、空状态与搜索（P0/P1；F03、F06）

**依赖：** A01–A02。**目标：** 空 Vault 不出现 alpha/beta；搜索不破坏正在阅读的会话。

**位置：** `applicationStore.ts`、`App.tsx`、`ThreadsPane`、`fixtures/timeline.ts`。

**步骤：**
1. 生产初始 agents/threads 为空；fixture 值只能由 fixture 入口显式传入。用生产 ViewModel 替换 ThreadFixture 的业务依赖。
2. 分离 entityById、visible list IDs、search results、selection、pagination cursors；权威空结果不能保留演示 extras。
3. App currentThread 允许 null；分别渲染 loading/empty/error/ready；有活动会话时无搜索结果只影响导航列表。
4. 请求使用 generation 或取消令牌；晚到的 A 结果不能覆盖更新的 B；清空 query 恢复正常列表。保留后端匹配原因，去掉不等价二次过滤。
5. 实现 has_more/next_cursor；搜索中不逐字清空实体；错误就地说明，不偷偷当 client filter 成功。

**验收：** 零代理零会话、50+ 会话、空结果、A/B 乱序、搜索中切会话、清空查询、返回列表后去重；当前会话草稿与选择稳定。探针的空结果 TypeError 改为“不抛出且显示 empty”。

**不做：** 实现本不存在的归档/删除按钮；若后续接入须独立契约。**迁移：** 不把演示数据迁入 DB。

## A04 — 发送、取消、权限与后台会话一致性（P0；F04、F05、F08、F12）

**依赖：** A03。**目标：** UI 不假称保存/停止/授权；后台会话事件不串线。

**位置：** store actions/reducer、App composer、permission rows、Core command response/event contracts。

**步骤：**
1. 创建失败不插入 saved thread；单独显示错误和可保留的输入。
2. active turn 按 thread/turn 映射；capture sending thread/config，事件按 turn_id 路由。单线程一轮活动；新增提交是否允许由能力和契约决定。
3. 发送有 pending/admitted/rejected/indeterminate；返回结构化结果给 Composer。确认前不不可逆清草稿，未确认投递不自动 retry。
4. 取消进入 cancel-requested；只在 Core 权威结算后 terminal。取消 RPC 失败或断线保留“可能仍运行”。
5. 权限动作带真实 permission identity、thread、turn 关系；pending submission 禁用重复点击，失败仍可见且可核对。读屏不要在命令发出瞬间宣称已批准。
6. 全局错误拆成相关操作错误；没有未处理 Promise；诊断摘要脱敏且有长度上限。

**验收：** create 拒绝、prompt 明确未投递/可能已投递、双击发送、A/B 同时输出、切换中发送、cancel 失败/晚确认、权限重复/晚到/不同 turn；关闭 UI 不终止 Core。服务端“单会话一轮”不等于 UI 可以只有一个全局 activeTurn。

**安全：** 新按钮不会自动扩大授权；不确定投递只提示核对。**迁移：** 新存储状态需版本化，旧轮次不得自动恢复成可重发。

## A05 — 正文历史、分页与重启恢复（P0；F07）

**依赖：** A04。**目标：** 重开看到真实正文、工具与权限，不是 Turn ID。

**位置：** protocol DTO/event、Core history handlers、storage journal queries、renderer timeline reducer。

**步骤：**
1. 定义有界 history/timeline record 契约，稳定 message/event identity、turn、顺序、文本/工具/审批类型、cursor 和截断策略。评估重用已存 journal，不复制第二套事实。
2. 快照有对应高水位/epoch；subscribe + snapshot 交接不漏不重复。未知事件保留受限说明，不能导致整个历史失败。
3. Core 查询实际内容；renderer 实时和历史走同一归约与呈现。分页用返回 cursor，加载旧历史 prepend，不 append 成乱序。
4. 已落盘正文在代理不可用时可读；读取历史不能被“启动 ACP 会话失败”阻塞，必要时把读历史和启动会话分离。
5. 定义最大 row/page payload；大内容分块或受限按需读取，不用一个无上限快照塞回全部日志。

**验收：** 合成中英消息、代码、多段 delta、工具、审批、取消；UI 关闭/重开、Core 重启、断网、不存在代理、重放 gap、50+ 分页；比较归一化时间线内容与顺序；100k 数据不要求一次全送 UI。

**安全/迁移：** full transcript sync 仍 opt-in；本地 DTO 扩展不自动成为同步 wire。任何持久改变加新 migration 与降级拒绝/只读策略。

## A06 — epoch、重放、缓存上限（P1；F09）

**依赖：** A05。**目标：** Core 重启不丢低序号新事件，长时运行状态受限。

**位置：** IPC session/recovery contracts、transport cursor、applicationStore 去重与恢复。

**步骤：** 定义 epoch+sequence；收敛 greeting/restarted/gap 处理先后；恢复窗口外走快照；去重按已确认高水位/窗口维护；后台会话可按需重建；为 timelineStores/测高缓存设置按条数与字节的策略，固定正在显示和活动会话；释放不可见历史只丢 derived UI 数据。

**验收：** 旧 epoch seq=100 后新 epoch seq=1；重复/乱序/gap；快照期间新事件；暂停订阅后恢复；固定 100 万合成事件中集合不线性无界增长；不依赖真实 sleep。

**禁止：** 清空去重后盲接旧 epoch；缓存淘汰导致丢草稿、审批或持久历史。**迁移：** 保存的 UI cursor 版本化；不识别时重新协商，不复用错 epoch。

## A07 — 可用且诚实的 ACP 配置（P1；F11、F27）

**依赖：** A01–A04。**目标：** 普通用户能配置真实 ACP，界面不承诺未支持模型/harness/凭证能力。

**位置：** AgentOnboarding、agent store/actions、ConfigureAgent/TestBinding DTO、Core secret port。

**步骤：**
1. harness 仅 ACP；不要将 provider 任意字符串推断成 terminal/native。程序选择与显示名称分开。
2. 参数独立数组编辑；Windows 带空格路径与 Unicode 原样透传，不经 shell。
3. 每个 env key 对应一个 opaque secret ref，边界一次校验；无效引用拒绝，不伪造。
4. 固定 model 默认移除；模型/模式仅使用协商且会实际发送的值。区分“配置可保存”“连接测试成功”“外部认证可用”。
5. 测试结果带配置版本/指纹；字段改动失效。保存/测试/关闭并发安全，错误保持表单，重开不展示过期成功。
6. 当前 resolver 不可用就明确说明；如范围允许接 OS secret store，单独形成子切片，Core 窄接口、引用返回和脱敏测试齐全才开放。

**验收：** 两个带空格参数、多 env 映射、非法 ref、测试后修改 program、保存失败、中文名称、协商无 model、真实测试成功后实际 prompt 使用该 binding。

**不做：** Altior 邮箱登录、收费、自动安装未知代理、默认把用户系统全部环境透传给代理。

## A08 — 修复工作台与窄窗几何（P0/P1；F13、F14）

**依赖：** A03。**目标：** Inspector 在正确区域，最小窗口可完成对话/权限操作。

**位置：** App layout、shell CSS、uiStore pane sizing；先采纳 DESIGN_I18N 第 2 节到 UI_ARCHITECTURE。

**步骤：** 显式 grid areas/四个中部区域；rail 收窄、nav 保持约定；main min-width/min-height；inspector 按可用宽度并排或抽屉；抽屉完整焦点/关闭/遮罩协议；拖动 pane 后重新判断布局；修复独立滚动与 tab order。

**验收：** 1280×800/760×800/720×480、inspector 开/关、nav 拖动极值、长标题、审批、200% 缩放；坐标断言 inspector 与 main 并排时同 top；无页面水平滚动；composer 和审批按钮可点击且不被盖住；浅深色截图人工审查。

**不做：** 全项目 CSS 重写、新设计框架、移动端新产品。**迁移：** 保存的旧 pane size 读取时按当前 bounds clamp。

## A09 — 中文输入、弹窗、焦点与权限可达性（P1；F16、F17）

**依赖：** A04、A08。**目标：** 输入法不误发送，键盘能完成流程。

**位置：** Composer、Dialog 原语、Inspector Tabs、TimelineRowView。

**步骤：** composition start/end 与 key.isComposing 防护，兼容 Windows 229 路径；Shift+Enter 换行；弹窗初始焦点、Tab 环、Escape、返回焦点、关闭名称；Tabs tabpanel/aria-controls/方向键；分隔器键盘方向符合真实运动；虚拟行焦点保留；权限/失败/完成以聚合 live announcement 告知。

**验收：** 探针改为组合输入不调用 send；实际 Windows 中文输入法确认候选不发出请求；Tab/Shift+Tab/ESC；鼠标与键盘权限一致；长日志虚拟回收后焦点有效；不逐 token 播报整个输出。读屏人工 evidence 不由自动扫描替代。

**不做：** 默认单字全局快捷键自动批准；快捷键在输入字段中抢按键。**迁移：** 无。

## A10 — 语义主题与共享原语（P2；F15、F19）

**依赖：** A08–A09。**目标：** 保留当前色系，所有关键控件在两主题一致且可辨识。

**位置：** tokens.css/reset.css、primitives、shell/timeline/context CSS。

**步骤：** 按 DESIGN_I18N 补 control-border/accent-foreground/status-surface/overlay；Button/Dialog/Tabs 共用，不跨模块偷借 class；原生 color-scheme；selected/focus/hover 分离；移除任意 shadow/z-index/rgba；收敛图标与主操作层级；版本/阶段/协议流移入诊断。

**验收：** token 组合报告，审批与配置按钮深色截图，focus-visible、forced-colors、reduced-motion、disabled/error/pressed；没有因删除颜色而丢状态文字；新增 token 有语义用法。对比度脚本检查实际背景组合，不只默认 token。

**不做：** 改品牌名、重造 logo、添加炫光/渐变、增加设计库。**迁移：** 主题 source 由 A11 管理。

## A11 — zh-CN/en 与设备本地偏好（P2；F18、F20）

**依赖：** A09–A10。**目标：** 两种语言完整可用，主题跟随系统与字号偏好重启有效。

**位置：** i18n 模块、所有用户文案、index lang、Settings、uiStore 与 Core 窄设置契约。

**步骤：** typed key 字典；按 DESIGN_I18N 术语统一；Intl 时间/数字/复数；error code→本地化说明；system/light/dark source；locale 自动/显式覆盖；设置标“仅本机”；仅保存展示设置，先与 UI_ARCHITECTURE 的 Core-confirmed preference 所有权统一。

**草稿子项：** 确认重启草稿保留需求、敏感内容处理和本地存储边界后再实现；主题/语言可以持久化，不因此授权把所有用户输入写入 localStorage。草稿不进入同步。

**验收：** 两字典 key/参数一致；扫描未抽离 UI 字符串（允许代码/fixture/技术标识明确 allowlist）；切换语言不中断轮次/不改正文；document.lang 更新；重启保存；中文/英文窄窗不裁按钮；未知 locale 可控回退。

**不做：** 翻译用户记忆、远程翻译服务、全库机械改名。**迁移：** 设置 schema/default 明确；旧值未知时回退 system，不 crash。

## A12 — 长正文与工具呈现（P2；F20、F25）

**依赖：** A05、A08–A11。**目标：** 恢复后的中英正文、代码和工具结果可读可复制，不破坏滚动。

**位置：** timeline renderer、shared content components、Core 大输出查询契约。

**步骤：** 明确支持的 Markdown 子集；默认不执行 raw HTML；安全 URL scheme/外链行为；代码块语言标签与复制；工具摘要/展开/状态；长输出按需分页和字节限制；每种动态高度改变通知虚拟布局并合并测量。

**验收：** 长代码/表格/路径/emoji、恶意 HTML/链接、折叠→展开、流式跨行、用户滚在历史中；clipboard 只复制当前用户选中的文本；不引入脚本注入与自动网络 fetch。远程图片默认策略先文档化，避免阅读记忆时暴露访问。

**不做：** 新 terminal 执行、完整 Git workbench、浏览器产品。**迁移：** 呈现升级不改变已存消息原文。

## A13 — 上下文作用域、信任和预算契约（P1；F26、F28、F29、F30）

**依赖：** A05–A06。**目标：** 注入什么、发给谁、为什么和保留多久有可执行规则。

**位置：** MEMORY/SECURITY、ADR 0018 的后继/修订、Core context/mod.rs/start_prompt、storage snapshot。

**步骤：**
1. 固定 Off/Session/LongTerm 与 Global/Person/Project/Thread 的允许集合；当前项目不可静默召回另一项目；Session 是否包含 Global 必须明确决策。
2. 明确身份指令、当前请求、检索引用的信任优先级；转义/封装不可信材料，注入最小 provenance/scope 标识；审批不能由记忆授权。
3. 统一 512/1024 与 1024/2048 文档差异、framing、tie-breaker；给估计器版本，显示 estimated；区分字节边界、注入预算和外部 agent 实际窗口。
4. 快照记录真实选中/丢弃和排除原因；按 turn immutable；scope 不允许的材料不得进入序列化 prompt。
5. 定义 forget 的未来检索排除、历史审计保留、保留期/导出和未来擦除；不要删除 append-only journal 破坏同步墓碑。

**验收：** 两项目同词、Session/Off 对照；恶意标题/换行/“忽略规则”材料；预算 0/很小/边界/中英 emoji；所选顺序与解释 rank 一致；快照写失败零外部投递；忘记后新轮次不再注入，历史显示符合保留策略。

**不做：** 自研 tokenizer/embedding 模型、把检索指令转成工具许可、承诺密码级不可恢复擦除。

## A14 — 把记忆与 Context 接到真实产品路径（P1；F10、F20）

**依赖：** A07、A11、A13。**目标：** 用户可以控制记忆，查看实际发送上下文。

**位置：** protocol memory/identity commands（缺的先定义）、Core memory ports、feature memory、ContextPanel/App wiring。

**步骤：** 明确所需命令并 Rust 生成 DTO；记忆列表分页/过滤和 shared inspector；候选确认/拒绝、纠错/忘记通过 Core 持久生命周期；设置 agent memory mode 的真实通路；选定 turn 后 get_context_snapshot 并把数据传给 Inspector；pending/null/error 不混同；切换 turn 的旧响应忽略。

**验收：** remember→新会话 recall→查看 why→correct→新轮次仅新内容→forget→不再 recall；真实隔离 Core+DB 重启；秘密形状数据拒绝零持久写；中文/英文/两主题、无/加载/拒绝/错误快照；没有假 memory cards 数据。

**不做：** 为了演示自动把所有模型输出确认成记忆；同步按钮、云端 embedding。**迁移：** 只加新迁移；已有生命周期事件不可重定义。

## A15 — 检索相关性、中文与有界候选（P1；F21–F23）

**依赖：** A13。**目标：** 相关性可测，中文有明确支持语义，top-K 不掩盖无界工作。

**位置：** storage memory search、MemoryRetriever port、FTS migration、deterministic fixtures。

**步骤：**
1. 固定至少 100 个 query 的合成评估集，建议中文/英文各 ≥40，另有混合/符号/短词；写可复核 gold relevance 0–3，并单独保留测试集。数据量另设 1k/10k/100k。
2. 输出当前 BM25 分量分布、clamp 命中比例、nDCG@8/Recall@8、scope 漏入=0、过期/忘记漏入=0；gold 只是项目基准，不冒充真实用户偏好事实。
3. 比较保持语义的 scope 下推、延迟构造 explanation、top-K；若截断候选，明确 rank 改变与 candidate budget，不能随意 LIMIT 100。
4. 对中文比较现有分词、适合短词的方案/子串方案；评估二字词、三字词、体积、索引更新时间；新 tokenizer 需 ADR 和可回滚迁移。
5. 排名策略变化记录算法版本；解释使用真实计算值和匹配依据，不用 contains 冒充词干匹配解释。

**验收：** 准确性门槛在修改前写入，至少关键回归 query 全通过；deterministic tie；没有 scope/forgotten/expiry 泄漏；有候选计数、物化条数、耗时与内存报告。若预算影响 Recall，公开取舍，不能藏降级。

**不做：** 直接加向量库、默认调用网络、以 machine-speed 测试替代确定性正确性测试。

## A16 — 数据与流式性能（P1；F24、F25）

**依赖：** A05–A06、A12；检索指标与 A15 协调。**目标：** 找到瓶颈且不削弱 durability。

**位置：** storage append/digest、event_pump、timelineStore/virtualWindow/Timeline、store subscriptions。

**步骤：**
1. 先记录 host/OS/build/toolchain/dataset/冷暖；衡量 100k messages+100k memories、1/10/100 delta/s、1/多后台会话。正确性测试不依赖真实等待；独立 benchmark 可测实际时间。
2. 计数每次 append 的 digest 扫描、事务次数、行物化、React commit、height index 重建、cache bytes；定位主要成本。
3. snapshot/append 批量化和增量索引必须保留顺序、锚点、幂等；流式合并按有界 paint cadence，Core durable events 不因 UI 合并被丢。
4. 若改 digest/投影校验，先 ADR 说明 crash/rebuild/损坏发现时机；不能删除校验换速度。若改高频持久策略，明确已显示与已 durable 的承诺。

**验收：** 原预算 useful history ≤2s、idle Desktop+Core <150MiB 在参考主机测量，达不到标记 FAIL 并解释；建议搜索 p95≤200ms、UI 交互 p95≤100ms 是待冻结目标，不冒充原约定；记录 p50/p95/peak，独立 profiler evidence；单 row delta 不使全部可见 row 重渲染；动态高/历史 prepend 保锚。

**不做：** 为满足数值删除历史、关安全校验、改 benchmark 数据。**迁移：** 增量 digest 或持久策略改变需新 schema/version 与恢复测试。

## A17 — 自动质量门禁与证据结构（P1；F32–F34）

**依赖：** A02 的 fixture 入口；其他任务随修随补，最终整合在此。**目标：** 测试可以发现本次已确认问题。

**位置：** package scripts、browser/visual config、CI（若仓库采用）、tools 脚本、CONTRIBUTING/AI_DEVELOPMENT。

**步骤：** 前端补明确 format/lint/typecheck/unit/browser/visual 命令；代码静态规则与旧问题制定明确处理范围；Rust gates 保留；Tauri 独立 manifest 单列。截图脚本改用 fixtureReady、禁止 pageerror、等待状态/字体、try/finally 清进程；固定 screenshot 环境；加 load-bearing geometry 断言和图像比较。

**验收：** 故意引入非法 ID、空搜索、三列布局、IME Enter、主题漏样式时相应 gate 会失败；干净 checkout 文档命令可执行；网络/真实代理测试独立 opt-in，未配置显示 SKIPPED 而不是发布证据 PASS。

**不做：** 自动更新 baseline，禁用 test 降门槛，因 repo 没有 CI 而宣称远程 CI 已坏。是否接入 GitHub/GitLab 按实际远端与授权；先提供本地可执行脚本。

## A18 — 真实打包桌面与两个 ACP 的连续性验收（发布 P0；F32）

**依赖：** A01–A12、A17；记忆发布另要求 A13–A16。**目标：** 从用户桌面走完真实路径。

**步骤：** 使用独立测试 profile/data dir；记录 OS、app/core build、两个已授权安装的真实 ACP 版本及协商能力；配置→probe→创建→stream→审批允许/拒绝→cancel→UI 关闭/重开→Core 重启→历史/搜索→indeterminate 禁重发。区分 mock 旅程、真代理 smoke 和打包 UI 旅程。

**验收：** 每步截图/脱敏轨迹/数据库可见结果一致；关闭 Desktop 不杀 Core turn；结束 Core 后子进程清理，含后代树能力未完成则写阻断；中文/英文最小窗可用；无模型支持的控制不存在。

**阻塞规则：** 真实代理/认证/签名环境缺失写 BLOCKED，提供精确人工运行说明；不得自动购买、登入、上传用户内容或改成 mock 说通过。可以继续其他独立任务，但不能把该任务 DONE。

## A19 — 发布定位、文档和安装升级门槛（P1/P2；F34）

**依赖：** A17–A18 的可用证据。**目标：** README 和发行包承诺与实际一致。

**步骤：** 当前状态表分 Backend/API/Renderer/Packaged/Third-party acceptance；更新 README/IMPLEMENTATION_PLAN，保留历史 release notes 日期语境；确认 Tauri edition 和根规则差异，按 ADR 解决；列支持系统、构建依赖、Core 路径发现、日志清理；安装、升级、旧 schema 拒绝、卸载保留/删除数据流程；签名/更新渠道未就绪不称正式生产版。

**验收：** 新环境按说明成功，未通过项明确；迁移从上一支持版本到新版本；崩溃重启数据有效；支持的 downgrade 行为可复现；中英文状态表一致；LICENSE/THIRD_PARTY_NOTICES 对新增依赖/代码复用有记录。

**不做：** 修改过去已经发布的 migration；自动发布、推送 tag 或覆盖用户安装。

## A20 — 同步上线前决策与安全阻断单（同步发布 P0；F31）

**依赖：** 可独立做设计；实现必须遵循已有 ACP-first 发布边界和后续批准范围。**当前任务范围：只做可审查设计、测试计划与禁用边界，不自动把生产同步上线。**

**位置：** SECURITY/SYNC、crypto/relay/CRDT ADR、现有 spike tests。

**步骤：** 针对 static-session 计数重启、低阶公钥/contributory 检查、nonce 唯一、持久 counter/epoch reservation、崩溃/磁盘回滚、撤销/轮换、签名认证、relay ack/cursor/全局配额、导入限额/压缩炸弹、30 天离线旧设备、forget tombstone/compaction 逐项建威胁树。列备选方案、失败模式、迁移与退出策略。

**验收：** 明确生产门槛与保持禁用的入口；模拟三个设备离线修正/忘记/回归不复活的固定测试设计；同 key 下重建 Session 不重用 nonce 的可执行回归要求；relay 看不到正文与恢复密钥的检查方法；独立安全审查所需证据清单。

**禁止：** 自研密码学、新框架未经 ADR、把内存 relay 当 durable 服务、把 ratchet 等未实现技术写成完成。现有 reviewed library 的使用不等于整个协议已被审计。

## 统一完成标准

每项必须有：失败复现→契约→最小实现→修复验收→适用失败路径→最终 diff review→实际命令结果→文档更新。应用 UI 改动有渲染审查；协议/存储改动有迁移/兼容；安全改动有威胁模型说明。

最终必需命令以仓库当前 scripts 为准；原有根门槛不能少：`cargo fmt`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --workspace`。前端已有 typecheck/test/build；缺失 format/lint/browser/visual 由 A17 补齐，不能宣称不存在的命令运行成功。独立 Tauri manifest 的检查不得漏掉。
