# Altior 完整项目评审

日期：2026-09-05；基线：`f945ef153eb3344abd3801a3c2e4002fd25d80be`。

## 1. 总体结论

Altior 的方向有价值：用户自己拥有可延续的上下文，界面和执行器可以替换，知识不依赖某个聊天服务。现有 Rust 工程中的领域类型、明确的投递状态、持久化 checkpoint、投影重建、确定性测试值得保留。

当前最大的差距不是功能列表短，而是“模块可运行”与“用户路径可信”之间的差距。前端仍大量使用早期 fixture 模型；正常失败可以被显示成成功；桌面协议边界有静态可确认的不兼容；截图脚本成功退出也能产生明显错误的布局。现在继续新增导航项、同步 UI、智能调度或另一个 harness，会扩大未验证的面积。

建议发布定位：**开发者预览，正在补齐 ACP 本地连续性**。面向普通个人用户的 0.1 需要通过 A01–A19 中相应门槛。P3 同步安全阻断单独处理，不应因为模块测试通过提前开放。

不提供虚假的“产品 8.2 分、架构 9.1 分”。以下用可复核的问题、风险和完成条件表达质量。

## 2. 范围、证据与限制

已读取根 AGENTS、AI 开发规范、产品/架构/UI/安全/记忆/验收契约及关键 ADR；沿 Desktop → transport → IPC → Core → harness/storage/context 路径检查；抽查 crypto、CRDT、relay 和现有测试结构。没有逐行证明每一个源文件正确，也没有做独立密码学审计、真实数据迁移压力测试或跨平台安装认证。

本次执行：Rust fmt 检查、全目标全特性 Clippy、workspace tests；前端 typecheck、90 个既有测试、build；独立 Tauri Rust crate 的 7 个测试；5 张浏览器截图；PinchTab 的开发页和弹窗交互；7 个定向问题探针。详见 [验证记录](evidence/VERIFICATION.md)。

证据等级：**V** 实测/截图确认；**C** 明确代码路径与契约不一致；**R** 有实现依据的风险，尚缺目标环境实测；**D** 文档或产品决策缺口。

严重程度：**P0** 阻断当前核心用户旅程或特定能力的安全发布；**P1** 高影响正确性、数据、隐私或可用性问题；**P2** 产品质量、维护性、体验完善。同步 P0 表示“阻断同步上线”，不表示用户今天已在使用不安全的生产同步。

## 3. 从各角色看项目

| 角色 | 已有优势 | 当前主要问题 | 决策建议 |
|---|---|---|---|
| 产品经理 | 一个用户一个 Vault，定位清晰 | 完成定义偏模块；普通人无法确定什么真的可用 | 按首次配置、首次对话、审批、取消、重启、找回六段验收 |
| 用户研究 | 个人知识连续性需求具体 | 尚无实际用户任务观察证据 | 先用开发者/知识工作者 5 人观察首次成功；不冒充统计结论 |
| 交互设计 | 编辑器式五区域契约合理 | 实现错位、失败不可见、配置暴露底层参数 | 主路径低认知负担，诊断渐进展开 |
| 视觉设计 | 中性底色与蓝色强调适合长时间工作 | 令牌覆盖不全；选中与焦点混用；深色原生按钮突兀 | 保留色系，修结构和语义，不整体换皮 |
| 国际化/无障碍 | 基础 aria 与键盘支持存在 | 英文硬编码、IME 误发送、弹窗焦点协议不足 | zh-CN/en 双语、输入法与键盘先于翻译完成 |
| 前端工程 | 有 transport fake 和独立行订阅 | 巨型 store、fixture 混生产、异步竞态、空结果崩溃路径 | 垂直修复后逐步拆分状态，保持一套归约逻辑 |
| 后端/架构 | 边界总体正确，Rust 类型约束强 | 前端绕过契约，历史 DTO 不够，上下文规范漂移 | 以端到端契约闭环为重心，避免推倒重写 |
| 算法 | FTS、确定性排序、来源解释可追溯 | 无候选资源边界；BM25 下限削弱相关性；中文未验收 | 先建立离线相关性数据集，再改排序和分词 |
| 性能工程 | 帧、通道等已有多处上限 | 累积状态、全量 digest、测高重建不受结果条数约束 | 先计数和剖析，再做保语义优化 |
| 安全/隐私 | 本地 IPC 鉴权、secret scanner、标准 crypto 库 | 伪造 secret ref、scope 边界、记忆注入信任、同步 spike 不能上线 | 不静默降级；安全改动必须说明威胁模型 |
| 测试/发布 | 大量确定性后端场景与 mock 子进程 | fake 与真实协议不等价；截图无断言；真实安装验收未做 | 分成单测、协议一致性、浏览器、打包 app、第三方代理证据 |
| 文档/维护 | ADR 和工程纪律基础好 | README/阶段状态/实际接线及参数不同步 | 只凭可执行证据更新完成状态；历史 release notes 保持历史语境 |

## 4. 问题清单：运行、功能与协议

### F01 — 前端构造的 ID 与 Rust 契约不兼容（P0，V+C）

位置：`apps/desktop/src/stores/applicationStore.ts:959,1203,1239`；`crates/altior-domain/src/id.rs:1`。

域 ID 要求 `<prefix>_<16..64 个小写字母或数字>`。前端出现 `agent-...`、`trn_<时间>_<计数>`、`op_start_turn_1`，前缀或 body 均不合约；很多其他命令 ID 同样包含额外下划线。Rust serde 会校验这些值，fake 不等于真实边界。探针验证了发送命令的实际输出不符合契约。

修复：建立一种合法的命令身份分配契约；需要 Core 生成的实体 ID 使用已有可选字段或明确的分配响应，不能让客户端猜。客户端 operation ID 如何跨重启唯一须写入 ADR，不能放宽域校验来适配坏客户端。新增 Rust 反序列化前端真实命令的测试。

### F02 — 开发环境分支与连接异常处理失效（P0，V+C）

位置：`apps/desktop/src/main.tsx:18`、`ipc/tauriTransport.ts` 的工厂；`applicationStore.ts:871`。

浏览器没有 `process` 时，`typeof process !== "undefined" && ...` 得到 `false`；`false ?? import.meta.env.DEV` 不会取后者。开发页实际走 Tauri transport，控制台报告 `TransportUnavailableError`。同时 `subscribe()` 在 init 的 try 外，异常使界面停在 connecting。已在开发服务器输出及探针中确认。

另需核对打包桥接：配置 `withGlobalTauri:false`，代码却从 globals 找 `listen`，没有直接导入正式事件 API。此处是 C/R：结构不一致明确，尚未运行真实 WebView 完整握手。不要以启用全局 API 和扩大权限来掩盖接线问题。

### F03 — 生产 store 默认包含演示代理和会话（P0，C）

位置：`applicationStore.ts:180,293,597`。

真实 transport 与 fake 都走默认 alpha/beta 和三条 fixture 会话；list 返回空也保留旧值，列表合并还保留不存在于新结果中的 extras。干净 Vault 会被装饰成已有代理和成功活动，空状态和删除后的权威状态不能准确表达。

修复：演示数据只在明确的 fixture 入口注入；正常启动为空、加载、失败、就绪四类真实状态。fake 必须用合法 DTO 和同一状态规则。

### F04 — 创建失败后仍伪装成功（P0，V）

位置：`applicationStore.ts:1132`。模拟 command 拒绝后，函数仍返回本地 `thread-...`，加入列表并标记 running。后续可能不能保存或恢复。

修复：命令失败显示可操作错误且不制造持久化事实。如果需要待提交草稿，必须有独立类型和显式“尚未保存”状态；Core 不可用不是已持久化成功。

### F05 — 取消失败也清除运行状态（P0，V）

位置：`applicationStore.ts:1269`。失败被吞掉后仍 finishStreaming 和 activeTurn=null，用户无法判断代理是否继续执行工具。

修复：取消请求中/确认取消/取消失败分开，等待 Core 权威事件。断线不能宣称停止，恢复后对账实际轮次；不自动重发 prompt。

### F06 — 搜索空结果可破坏当前会话（P1，V+C）

位置：`applicationStore.ts:1101`、`App.tsx` 的 currentThread 与 `getTimelineStore(currentThread.id)`。

搜索把整个 threads 替换为返回数组；空结果时当前会话和第一条都不存在，非空断言不会保护运行时。探针复现同一路径的 TypeError；未在真实 Core 浏览器搜索端到端复现。

修复：实体缓存、列表页、搜索结果 ID、当前工作面分开；无结果只改变列表，不卸载正在阅读的对话。处理 A/B 请求乱序、清空搜索、分页与缓存失效；前端第二次 includes 过滤不能删掉后端合法命中。

### F07 — 历史恢复仅显示轮次编号（P0，C）

位置：`crates/altior-protocol/src/dto.rs` 的 TurnDto/ThreadHistoryResponseDto；`applicationStore.ts:670`。

DTO 无消息正文和有序工具/审批内容，renderer 构造 `text: Turn ${turn.id}`。Core journal 已有事件存储，问题不是简单“数据库没数据”，而是没有闭合可用的历史投影、分页协议和统一归约。

修复：一个版本化的 timeline/history 契约同时服务快照、分页、重放和实时事件。验收必须比较重启前后真实合成正文、工具、审批和顺序，而非“返回了一条 turn”。

### F08 — 单个全局 activeTurn 无法表示后台多会话（P1，C）

位置：`applicationStore.ts:127,479,1192`。A 会话正在输出时对 B 发送会覆盖唯一 activeTurn；A 后续 delta 因 thread 不匹配被丢弃。同一会话发送按钮也未按活动轮次或 steering 能力约束。

修复：按 thread/turn 标识维护状态；事件必须匹配 envelope.turn_id；按钮行为由协商能力与 Core 状态决定。这是既有后台会话连续性，不是新增多代理委派功能。

### F09 — 序列去重缺少 epoch 与生命周期界限（P1，C/R）

位置：`applicationStore.ts:288,421,573`。event ID 与 sequence 的 Set 持续增长；新 Core epoch 可能重新使用 sequence，去重在 greeting 分支前可能屏蔽新事件。当前恢复主要补选中会话，不能据此保证所有后台工作恢复。

修复：以 `(core_instance/epoch, sequence)` 为游标语义；按已确认的保留窗口裁剪，并保持旧事件安全拒绝。先设计 handshake/gap/重放原子交接，不能简单清空所有去重集合后接收任意事件。

### F10 — ContextPanel 有组件，但主应用未接入数据（P1，C）

位置：`App.tsx` 的 Inspector 调用；`shell.tsx:335,428`；store 的 getContextSnapshot。

App 没传 contextSnapshot，也没有按选中轮次发起查询。独立 ContextPanel 测试通过，不等于主应用可查看发送上下文。需处理轮次切换、过期响应、无快照、拒绝、断线和请求错误。

### F11 — Agent 配置混淆 harness、provider、model（P1，C）

位置：`shell.tsx:611` 附近表单；`applicationStore.ts:957`。

Provider 输入暗示 terminal/native 已可用，model 默认为固定型号且不来自能力协商；model 未进入 configure_agent 的模型配置契约。Args 按空白拆分破坏带空格路径；多个 env_keys 与最多一个 secret_ref 长度不匹配；测试结果没有与完整配置快照绑定。

修复：ACP 启动程序、参数数组、环境键→opaque ref 映射、协商结果分层展示。模型/模式仅在支持且真正下发时出现；编辑参数立即使旧测试结果失效。不要把命令行改成 shell 字符串执行。

### F12 — 输入丢失和异步错误反馈不足（P1，C）

位置：`App.tsx:101` 先清草稿再等待发送；`applicationStore.ts` 多个空 catch；App 不展示全局 error。

修复：未确认投递失败保留原文；不确定投递保留只读原文并解释核对途径，不能恢复成可自动重发队列。表单保存错误就地展示，不出现未处理 Promise；错误码、用户建议与受限诊断分离。

## 5. 界面、色彩与中英文

### F13 — 五区域网格实际只有三列（P0，V+C）

位置：`App.module.css:6`；截图 [light](evidence/light.png)、[dark](evidence/dark.png)。

rail、nav、main、inspector 是四个中间层兄弟元素，但网格 `auto 1fr auto` 只有三列，没有明确区域。Inspector 自动落入下一行，rail 被拉到约 360px，线程列表变窄，主内容下方留出大块空白。应使用显式 grid areas/列，并验证元素相对坐标；不是把某个宽度从 360 改到 320 就算修复。

### F14 — 窄窗抽屉遮住对话与审批（P1，V）

[narrow 截图](evidence/narrow.png) 760×800：默认打开的固定右面板盖住内容，导航没有完整收起策略。最小窗高 480 也需要验证表单与按钮可达。

修复：根据内容最小宽度收起 pane；明确 overlay 是模态还是非模态；模态需 Escape、焦点圈定/归还、底层不可交互。首开不强占阅读区域。

### F15 — 基础颜色合格，控件主题未闭合（P2，V+C）

当前 text/muted/accent/danger/warning/success 在 surface 上的纯色对比度均超过 4.5:1；主色无需盲目更换。浅色 border/surface 约 1.44:1，深色约 1.42:1：可作为装饰分割，但不能单独承担必需的输入边界或焦点识别。

深色审批按钮仍是白色浏览器原生样式；CSS Modules 中 shell 的 permissionControls button 不会自动覆盖 timeline 模块的类。新增弹窗有硬编码 shadow、z-index=1000 和 rgba；设置 color-scheme、补语义状态 token 并统一原语。不要把装饰边框不达 3:1误报为所有边框都违规。

对比度与目标尺寸规范依据：[W3C WCAG 2.2](https://www.w3.org/TR/WCAG22/)。具体建议和测试矩阵见 [设计规范](DESIGN_I18N.md)。

### F16 — 中文输入法 Enter 会发送（P1，V）

位置：`shell.tsx:297`。未检查 composition，定向探针在 isComposing=true/keyCode=229 时调用 onSend。修复需原生 composition 状态与兼容行为，不仅把按钮翻译成中文；补真实 Windows 中文输入法手工验收。

### F17 — 弹窗键盘与输入语义不完整（P1，V+C）

PinchTab 按 Escape 后 Agent Onboarding 仍打开；代码未实现 focus trap、初始焦点与返回焦点。关闭按钮只有“×”；Inspector tablist 缺关联 tabpanel 与方向键协议。时间线 role=log 也需检查是否逐字播报、虚拟回收是否丢焦点，不因存在 aria 属性就判通过。

### F18 — 国际化未形成系统（P2，C）

`index.html` lang=en；界面、aria、错误、时间线 label 多处英文硬编码；尚无 locale 字典与 Intl 管道。应先支持 zh-CN/en，统一“会话/轮次/个人知识库”等术语，不翻译代码/路径/模型标识/用户正文。保留日后其他语言的出口，但当前不加机器翻译服务。

### F19 — 视觉信息层级和用户文案仍偏工程样机（P2，V+C）

首屏 IPC v、P3、Provisional UI decision、协议流占据用户界面；rail 用字母缩写又重复文字；选中项始终有焦点描边；主题控制挤在每个 thread header。未交付导航禁用展示与 UI 契约“不要靠占位扩大产品”不符。

建议：可用的 Threads/Agents 先上线；Memory/Settings 按工作项真实接通后出现；Devices 在 P3 准备好后出现。主状态说“本地可用”“连接已断开”，版本/错误栈放诊断；局部选中与键盘焦点分开。

### F20 — 呈现偏纯文本，长期阅读与操作缺口（P2，C）

TimelineRowView 基本输出 span；代码块、表格、工具展开、复制、长输出截断/按需读取未成为产品控件。主题/draft/anchor 只在内存中，重启恢复没有完整契约。

先保证正文不丢和定位稳定，再增加安全 Markdown、代码复制和本地呈现设置。富文本不执行 HTML/脚本；草稿是否落盘必须明确敏感信息策略，不把原始草稿随手写入 localStorage。

## 6. 算法、性能与上下文

### F21 — 有返回上限，没有检索工作量上限（P1，C）

位置：`crates/altior-storage/src/memory.rs:742–853`。SQL 取全部 FTS 命中，Rust 再 scope 过滤、构造完整记录、解释文本和 Vec、全排序，最后 truncate。M 个命中需要 O(M) 内存和 O(M log M) 排序；返回 8 条不代表只处理 8 条。

优先把 scope/状态条件下推，在保留完整综合排名的前提下减少复制和排序。使用 top-K 堆只能解决保留结果内存，不消除全扫描；先按 BM25 LIMIT 候选会改变最终综合排名，必须有明确候选策略和召回率证据，不能偷偷当等价优化。

### F22 — BM25 截断让常见词相关性趋同（P1，C/R）

`text_score = (-raw_bm25).max(0.1)`：低于 0.1 的数值全部变为 0.1。常见词的具体分布需要用本项目数据集测量，不能声称当前所有查询都失效。最终上下文又以 created_at/ID 升序重排相同分数，而 storage 按 updated_at/ID 降序，解释 rank 和实际插入顺序可能不同。

修复：先固定 zh/en 相关性集与 gold judgments，记录各分量分布、nDCG@8/Recall@8；统一 tie-breaker 和解释顺序。不要靠随意调权重制造“更智能”的观感。

### F23 — 中文全文检索未经过产品级验收（P1，C/R）

memory FTS 使用 `porter unicode61`；查询按空白切词。对不含空格的中文词组检索，不能把英文分词功能直接当作中文语义/子串检索。需用“我喜欢乌龙茶”→“乌龙茶”、混合中英、短词、全角符号、文件名固定样本核验。

[SQLite FTS5 官方文档](https://www.sqlite.org/fts5.html) 区分 token 匹配与 trigram 子串能力。不要未经比较就全面换 trigram：短词、索引体积、召回与迁移都需测。当前报告未运行 Rust 内置 SQLite 的中文基准，列为风险而非实测失败。

### F24 — 每次领域追加重算完整 projection digest（P1，C/R）

位置：`altior-storage/src/lib.rs:586,658,4315`，`event_pump.rs:113`。

追加事件后 digest 扫描 projection；message delta 会持久化。数据规模 N、输出事件频率 E 同时增大时，可能产生近似 O(E×N) 的累积工作。启动也要比较 digest。没有测量就不能宣布满足 2 秒启动/150 MiB。

修复前先用计数/trace 定位比例。优化不得取消 journal 原子性、自愈或篡改检测；任何增量校验或后台校验策略都需 ADR、崩溃窗口、受损索引测试和恢复语义。

### F25 — 虚拟列表并不等于流式性能已经达标（P1，C/R）

HeightIndex 每次高度变化全量 O(N) 重建；测高只挂在父组件 layout effect，行级流式更新/折叠变化不一定触发父测高。真实事件还更新全局 appState 和每行 inline callback，可能抵消 memo 的收益。appendRow 也全量重建 snapshot rows；seen sets/timelineStores 没有明确总量淘汰。

需验证实际 transport 的流式路径而非只调用 appendDelta。尺寸变化用合并测量信号/ResizeObserver 或同等机制，按证据决定增量索引；使用未测量高度估计时必须保留行内偏移锚点。不能先引入新虚拟列表框架再希望问题消失。

### F26 — token 是启发估计，却被写成硬预算（P1，C/D）

代码按 UTF-8 bytes/4 向上取整。对不同模型、中文、emoji、代码都不是精确 tokenizer；注入预算也不等于代理完整窗口预算。ADR 0018 写 512/1024，代码常量为 1024/2048；framing 文案也不同。

显示“估算”，版本化估计策略；总线字节限制与注入估计预算分别执行；考虑用户 prompt 与 harness 自身上下文的边界。测量支持模型误差后才能声称实际 token 上限。统一 ADR/实现，不能简单修改测试期望把差异抹掉。

## 7. 安全、隐私与数据语义

### F27 — 非法凭证输入被伪造为不存在的引用（P1，V）

位置：`applicationStore.ts:229`。任意无 scheme 字符串被 Math.random 改成 `ref:opaque-sec-*`，没有存入 secret store，也没有可解析目标。这不是泄漏证明，却是错误的配置成功暗示。合法 scheme 也不能只凭 startsWith 判有效。

应拒绝并显示“请输入凭证引用”；若实现凭证录入，走 Core 的 OS secret-store 窄接口并只返回 opaque ref，失败不保存配置；不能要求 UI 自行读取密钥。当前 NoSecretsResolver 明确是占位 seam，不应宣称系统钥匙串已经集成。

### F28 — LongTerm 检索未限定当前项目/线程上下文（P1，C/R）

`CoreApplication::start_prompt_envelope` 对 LongTerm 传 scope_filter=None，按代码可能取不同项目/线程的同词记忆。虽然是同一个人，也不意味着所有私密项目材料都应发给所有外部代理。

先在 MEMORY.md 定义允许 scope 集合：Global、匹配 Person、当前 Project、当前 Thread 的组合及用户显式扩展。Session 通过 matches_scope 还会包含 Global，需与“无跨会话召回”的承诺核对。不能把检索质量修复变成扩大数据披露。

### F29 — 注入文本缺少足够的信任与来源边界（P1，C/R）

`context/mod.rs` 向 wire_prompt 拼 `# Relevant Memories` 和 `- [kind]: content`。snapshot 有 scope、来源解释不等于发送给模型的文本带这些标记。内容可包含换行和伪造标题，必须当不可信引用处理。

明确身份指令、当前用户请求、检索材料的优先级；对结构分隔做编码或转义；发送最小 scope/source 标记；覆盖“忽略之前指令”等合成对抗样本。提示词不能成为权限授权机制；本地工具审批仍是硬边界。

### F30 — 忘记、审计保留与彻底删除需要明确区别（P1，D）

memory tombstone 将内容从后续检索排除，但 immutable journal 和 context_snapshot.rendered_prompt 可能保留历史副本。这符合部分审计需要，却不能向用户承诺“所有字节已擦除”。

文档和 UI 区分“以后不再使用”“审计记录保留”“安全擦除/密钥销毁”。审计保留期、导出、备份、全局忘记及同步墓碑应统一设计。不能直接删除 journal 以实现一个按钮，不能把纯逻辑 tombstone 描述为加密擦除。

### F31 — 同步 crypto/relay 仍是 spike（同步发布 P0，C）

Session 从同一静态身份导出方向密钥，构造时 send_counter=0、replay window 重置。重建同一身份对的 Session 会重用计数起点；当前代码自身不能保证跨实例/重启 nonce 唯一。README 已说明 counter persistence 等未完成。

同步上线前需设计持久化 counter/epoch、崩溃与备份回滚、认证与 contributory 检查、撤销轮换、旧设备恢复、全局资源限额、relay 持久 ack/cursor、恶意压缩或导入包限制。不得把纯 relay 状态机当成已部署的安全服务。保留已有库，不自研 crypto，不拿前端 UI 工作顺便开放同步。

## 8. 工程、验收与发布

### F32 — 测试绿不代表真实接线通过（P1，V+C）

测试多在 App 直接注入 InMemoryTransport，绕过 main.tsx 环境判断和 Tauri globals；当前 90 个测试可全部通过而真实开发页仍 connecting。真实 smoke 在没有环境变量时直接 return，Rust 显示 ok 不能代表真实代理跑过。

新增四层证据：真实前端命令→Rust 校验；真实浏览器 fixture 入口；真实 Tauri 与隔离 Core；两种真实第三方 ACP 的人工/opt-in 验收。最后一项需要本机有效外部配置，不应捏造或为了自动过测改成 mock。

### F33 — 截图脚本缺少 ready/error 与比较门槛（P1，V+C）

baselines.mjs 等到 `[data-row-id]` 就截图，有固定 250ms sleep，无 pageerror 失败策略，无像素/几何比较。实际生成 5 张图并成功退出，但 connecting 与布局错误都在图中。它是采集器，不是合格回归门禁。

保留此次图作为失败证据；先修 fixture 专用入口和可观测 ready 状态，再建立 reviewed baseline 与几何断言。禁止自动覆盖参考图来消除差异。

### F34 — 完成状态、质量门禁与打包覆盖漂移（P1/P2，C/D）

README 停在 P1.3/P1.4 next，而 IMPLEMENTATION_PLAN 标 P1.4/P2.2 complete；独立 Tauri crate 不属于 workspace，edition=2021（根规则是 2024，需要核对 ADR 的例外）；package scripts 无 format/lint/browser compare；本 checkout 未找到仓库内 CI workflow；bundle.active=false。

这些不都等于 bug：独立 workspace 是明确 ADR 决策，历史 0.0.1 release notes 不应被改写成当前状态。但是必须单列 packaged-shell gate、支持平台、编译工具链、签名/安装/更新/回滚与真实版本状态。不能声称 cargo workspace tests 已覆盖 Tauri。

## 9. 产品规划建议

### 发布顺序

1. 真实链路可信：合法 ID、正确 transport、空 Vault、失败可见、配置可用。
2. 连续性可信：真实正文历史、重启恢复、权限/取消、后台会话、无重复投递。
3. 交互可用：正确布局、中文输入、键盘焦点、主题、设置与安全正文呈现。
4. 记忆可解释：scope、预算、来源、候选确认/纠错/忘记、ContextPanel 接线与相关性评估。
5. 稳定发布：基准、安装、两个真实代理、平台矩阵。
6. 多设备：独立威胁模型与同步验收后开放。终端/native/委派继续遵守原发布边界。

### 建议量化的用户目标（提案，不是已有测量）

- 已安装并完成外部认证的 ACP 用户，首次打开到首条有效回复的步骤可解释；记录卡点，不把提供商耗时归因于 Altior。
- 关闭 Desktop 后 Core 活动不中断；重开可核对全部已落盘正文和正在等待的审批。
- 空、错、慢、离线时仍能回答：发生了什么、哪些内容安全、下一步可做什么。
- 用户能查看一条记忆来自哪里、为何被选中、怎样更正，以及“忘记”确切影响。
- 不默认发送遥测；本地诊断和合成评估即可起步，若未来收集外部指标需单独明确选择与最小化。

## 10. 下一步

按 [TASKS.md](TASKS.md) 执行。修复 F01–F07、F13 前，不接受“新增更强 AI 功能”作为替代。每一项完成都必须带可复现证据；本次 7 个问题探针断言的是错误现状，后续应转换成期望行为测试，不能保留错误现状作为永久验收。
