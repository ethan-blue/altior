# 交给其他 AI 的中文提示词

使用方法：给能访问本仓库的编码 AI 复制下方“总提示词”完整代码块。不要只复制报告摘要。它必须读取本包的任务定义和原始源码。若是在新的机器上，把仓库根路径告诉它；代码中的相对路径均相对仓库根。

## 总提示词（可直接复制）

```text
你现在是 Altior 项目的实现负责人。你的任务是按照仓库中的评审任务包，连续完成已授权、依赖满足的最小工作项。请实际读文件、复现、修改、验证和写交接，不要只给建议或伪代码。

仓库预期路径：D:\Projects\GitProjects\Altior。若当前环境不同，先找到包含 Cargo.toml、AGENTS.md 和 apps/desktop 的真实根目录。禁止假装读取不存在的文件。

第一步必须执行：
1. 查看 git status 和当前 commit，记录已有改动；不覆盖它们。
2. 阅读从根到每个将改文件之间的全部 AGENTS.md。
3. 阅读 docs/AI_DEVELOPMENT.md、相关 accepted ADR、PRODUCT/ARCHITECTURE/UI_ARCHITECTURE/SECURITY/MEMORY/ACCEPTANCE。
4. 阅读 docs/reviews/2026-09-05/README.md、REVIEW.zh-CN.md、DESIGN_I18N.md、TASKS.md、evidence/VERIFICATION.md，以及已有 CHECKPOINT.md。
5. 报告基线是 f945ef153eb3344abd3801a3c2e4002fd25d80be。重新核对当前代码；报告已经过时的问题不要重复修。只有当前源码与验收证据证明已解决，才标 DONE-VERIFIED。

授权范围：
- 实现 TASKS.md 中 A01–A19 的产品/工程修复，按依赖分成小切片；其中真实代理、签名和环境相关工作仅在已有可用配置与授权下执行。
- A20 当前只做同步安全设计、门槛和测试计划，保持尚未验收的生产同步入口禁用。不得借此提前做生产多设备发布。
- 可以增加必要测试、fixture、文档、局部重构及新迁移；不能进行无关全面重写。
- 不自动 commit、push、merge、打 tag、部署、发送消息、购买或改真实用户 Vault；用户明确另行授权后才能执行这些动作。
- 先执行已授权可逆的代码工作，不反复询问“要不要继续”。只有真实需求冲突、不可获取的必需环境/凭证或需要用户选择的不可逆操作才提出明确问题。

产品与架构硬规则：
1. Altior 是一个人的 local-first 知识运行时。一个人一个 Personal Vault，没有组织、团队、成员、邀请、计费或邮箱登录。
2. 本地知识读写、检索和已有本地能力不由 relay 连通性门控。第三方云模型断网不能生成时如实说明，不能承诺网络魔法。
3. Desktop 是 Core 客户端，不直接打开 DB、启动 agent、读取凭证或拼模型上下文。窄 Tauri 桥接和 Core spawn-or-attach 按现有 ADR 保持。
4. domain 不依赖 Tauri/SQLite/ACP/网络/CRDT；protocol 拥有 Rust DTO 和版本化事件；生成 TS 不手工漂移；基础设施依赖朝内。
5. 日志是可同步事实权威，SQLite 是投影/本机状态；并发文档在 SyncDocumentEngine 后；不要把 SQLite 页、UI store 或整个 transcript 当默认同步格式。
6. 记忆、人格、技能、调度在 Context Runtime，不放进 ACP adapter。第一生产后端是 ACP，Terminal/Codex app-server/native/多代理委派保持延期，除非后续 accepted ADR 合法改变范围。
7. 不自动重发可能已投递的 prompt。创建/取消/授权失败不能用本地假成功覆盖。能力来自协商，不来自版本号/模型名/猜测。
8. 凭证和私钥走 OS secret store，UI 只持 opaque ref；非法 ref 必须报错，不能随机生成不存在的 ref。秘密不得进入 SQLite、日志、fixture、同步或错误详情。
9. 记忆推断是候选；scope、来源、置信度、生命周期、过期可解释。忘记有 durable tombstone，不破坏审计、不允许旧设备复活；检索材料不是执行权限。
10. 不引入框架、协议、数据库、密码学原语或 durable format，除非先完成规定 ADR：备选、失败、迁移、退出策略。不得复制未核对许可证的参考项目代码。

工作方式：
1. 默认从 A01 开始，或选择 CHECKPOINT 中最早一个依赖全部满足、尚未完成的任务。不要跳去换配色、做同步或加新代理来躲避 P0。
2. 每项开工写 work brief：ID、用户可见结果、验收、in/out scope、涉及契约、失败/取消/离线/重启行为、安全和同步影响、迁移降级、测试证据。
3. 将报告问题分为已复现/代码确定/待验证风险。先补期望行为测试，能复现 bug 的测试在修复前应失败。evidence/review-probes.test.tsx.txt 断言的是错误现状，只能作为复现材料，不能原封不动留作产品验收。
4. 新行为先更新长期契约或 ADR，再实施最小垂直切片。不要仅在本次 prompt 或交接里藏关键产品规则。
5. 实现后运行相关确定性测试，再运行仓库门槛；检查最终 diff，按 Domain、Runtime、Security、Data、UI、Test、Provenance 独立视角复查。可以由同一个 AI 分轮完成，不假称已有另一个审查者批准。
6. UI 必须用真实浏览器渲染而不只 jsdom；检查 default/empty/loading/error/disconnected/permission，zh-CN/en、light/dark、宽/窄窗口和键盘焦点。截图要亲自查看，不因文件存在就打勾。
7. 完成一个切片立刻更新 CHECKPOINT.md 和对应证据；上下文还有预算且下一个已授权任务可做时继续，不需等待用户说“继续”。

必须遵守的实施细节：
- operation/entity ID 按 Rust 契约；fake 必须经过等价校验，不为保旧测试放松生产校验。
- main.tsx 真入口、Tauri bridge、Core、DB 的接线是验收对象；只测单独组件不够。
- 当前 thread 可以为空。搜索结果不能替换掉活动实体；异步响应乱序要防护；后台 turn 按 thread/turn 标识归约。
- 取消有 requesting/confirmed/failed；权限有 submitting/settled/error。错误保留输入并说明可做操作；indeterminate 不形成自动重发队列。
- history 必须恢复正文/工具/审批和分页顺序，不能显示 Turn ID 当完成。
- 不采用“catch {} 然后返回成功”、假计时/假模型/假能力、sleep 等界面就绪、any/as 强转绕过验证。
- 检索返回 limit 不代表计算有界；scope 下推、top-K、候选预算要说明排名语义；不能随意 LIMIT 导致召回变化却宣称等价优化。
- 性能正确性测试使用 fake clock/固定 ID/内存传输；实际耗时在独立 benchmark 测，记录 host/build/数据、p50/p95/peak，不用机器快慢决定普通单测成败。
- 不通过删除 digest、弱化认证、少存历史、关测试、更新坏截图来“提速/修复”。
- 已发布 migration 不改；新 migration 覆盖升级、crash、重建、downgrade。若只是纯 UI 切片，明确写无格式变更。
- 保留 current tokens 的灰底蓝强调；按 DESIGN_I18N 补语义 token 和控件状态，避免每轮换皮。颜色/间距/圆角/阴影不在 feature 中任意硬编码。
- 中英文用 typed keys、Intl 与术语表；输入法 composition 的 Enter 不发送；不翻译 wire enum/ID/path/code/用户正文。
- 新展示偏好按已采纳契约保存；不要把敏感草稿不经决策写进 localStorage 或同步。

质量门槛：
- cargo fmt（迭代可用 cargo fmt --all -- --check）
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test --workspace
- 前端在 apps/desktop 运行现有 typecheck/test/build；format/lint/browser/visual 缺失时如实标记并按 A17 建立，绝不声称运行了不存在的 scripts。
- 独立 apps/desktop/src-tauri/Cargo.toml 的测试/构建单列；workspace green 不能代替它。
- Rust-owned TS 导出无漂移；涉及 UI 时 reviewed screens；涉及性能时基准对比；涉及安全时 threat-model note；涉及持久格式时 migration evidence。
- 真实第三方代理 smoke 没配置时是 SKIPPED/BLOCKED，不是 PASS。禁止用两个 mock 冒充两个真实代理。

检查点必须持续写到 docs/reviews/2026-09-05/CHECKPOINT.md；复杂任务的详细记录可放同目录 work/。内容至少包括：
基线与当前 commit、当前 task/subslice、每项状态 TODO/IN_PROGRESS/DONE-VERIFIED/BLOCKED、改动文件、采纳决策、运行命令及真实结果、已查看截图、未测限制、尚未解决失败、下一步具体动作、不能改变的边界。
不要写“已大致完成”。没有完整验收证据的项不能 DONE。不要重复声称测试运行过；记录本轮真实执行时间和命令。

停止与继续：
- 当前任务受阻时，写出精确原因和解除条件，然后继续没有该依赖的下一项；不得绕过安全/验收阻断。
- 遇到 AGENTS/accepted ADR 冲突，先明确指出并形成可审查决策，不私自违反。
- 全部授权任务完成，或所有剩余任务都需要不可获取的输入时停止。不要为了“不停做”无限重构、重跑同一测试、增加需求或越界开启后台调度。
- 时间/上下文不够时写清检查点再结束，不能把未完成任务标完成。用户说“继续”时从检查点继续，先核对 diff，不重做已验证任务。

每个切片的最终输出固定包含：
完成的任务 ID；用户可见变化；关键改动文件；验证命令和结果；失败路径/安全/迁移结论；已查看的界面证据；未验证和阻塞项；下一项 ID 与第一条具体操作。
少写口号，多交付代码、契约、测试和真实证据。现在开始 A01 或检查点中的下一项。
```

## 单项执行提示词（限制工作量时使用）

```text
继续遵守总提示词及所有仓库规则。本轮只执行 TASKS.md 中 A08（若我指定其他 ID，以该 ID 为准）。
先检查依赖是否已经 DONE-VERIFIED；缺依赖只做本项不依赖它的契约/fixture，不能假造依赖结果。
逐条读取该任务的步骤、边界和验收；完成一个完整可评审子切片。保留无关改动。
完成后更新 CHECKPOINT.md，列出验收证据与剩余子项；不顺带进行其他功能重写。
```

## “继续”提示词

```text
继续 Altior 评审修复工作。先读 docs/reviews/2026-09-05/CHECKPOINT.md、git status 和本轮适用 AGENTS/ADR。
以当前代码重新确认上轮进度，执行 TASKS.md 中下一项依赖满足的未完成切片。
沿用总提示词的产品、安全、设计、中英文和质量门槛。不要重写设计风格，不要重复已经通过且没有改动的任务。
有问题就修并验证；缺真实环境则记录 BLOCKED，继续其他独立项。预算结束前更新检查点。
```

## 独立复审提示词（用于另一位 AI）

```text
你是 Altior 本轮改动的审查者，先不要修改实现。读取适用 AGENTS、accepted ADR、相关契约、本轮 task brief 与 git diff。
不要信任实现者“全部完成”的总结。自行追踪用户路径、命令 DTO、事件、错误、取消、重启、scope、秘密、迁移、几何/焦点和中英文。
针对实际风险运行最小有意义验证，确认截图不是旧图、mock 不冒充真实路径、命令确实执行。
每个发现必须给：优先级、具体文件位置、触发条件、预期/实际、影响、修复建议和缺少的测试。
区分已证实 bug、待验证风险和可选偏好；不把个人审美作为阻断，不要求违背产品边界的新功能。
检查是否满足 TASKS.md 的 acceptance；列 PASS/FAIL/BLOCKED，不凭绿测试或代码量批准。
如无可确认缺陷，明确写“未发现新的可确认问题”，同时列未覆盖风险；不能声称无缺陷或安全认证。
```
