# Altior 设计与中英文规范（待采用提案）

本文件落实 REVIEW 的设计建议。已有 `docs/UI_ARCHITECTURE.md`、accepted ADR 和 AGENTS 优先；实施者先将采纳的行为更新到长期契约，再实现。所有新尺寸/令牌集中定义，不能散落在组件里。以下是明确目标，不是已验证的现状。

## 1. 产品气质与界面信息

定位：个人知识与代理工作台；安静、紧凑、清楚、适合长时间读写。以对齐、分隔线、字体层级组织内容。保留当前中性灰底和蓝色强调。

禁止：AI 渐变光晕、大头像聊天气泡、占满首屏的欢迎 hero、所有内容装成同尺寸卡片、用状态色装饰无状态信息、未接通按钮、团队/邀请/付费登录页面。禁止引入与当前图标/组件体系重复的第二套体系。

首屏主要回答四个问题：正在什么会话里、由哪个代理执行、执行到哪里、我现在可以做什么。版本号、binding ID、协议原文、环境键、内部阶段代号归入有界诊断面板，不常驻主路径。

首发入口按真实完成度开放：会话、代理；设置完成时开放设置；记忆生命周期 UI 真正接通时开放记忆。项目在注册与关联完整时开放；设备等到同步门槛满足。不得用一排不可点击导航显得功能很多。

## 2. 布局契约

采用明确命名区域：title、rail、nav、workbench、inspector、status。禁止依赖 grid 自动放置决定 pane 的行列。

### 宽度与响应规则

| 区域 | 提议默认值 | 最小/最大及行为 |
|---|---:|---|
| 活动栏 rail | 56px | 图标居中，tooltip 和 accessible name 含文字；不同时显示字母缩写和重复名称 |
| 会话导航 nav | 256px | 208–420px；键盘/拖拽可调，独立滚动 |
| 对话工作区 | 剩余宽度 | 尽量维持至少 480px；不能被 inspector 挤没 |
| Inspector | 360px | 280–640px；默认关闭直到有上下文需求；允许恢复用户已保存偏好 |
| 状态栏 | 内容适配 | 单行优先；窄窗允许有规则地换行，不横向撑页面 |

宽窗并排是否成立按 `rail + nav + workbench minimum + inspector + dividers` 实际计算，不只写死一个 900px 断点。默认合计约 1152px 加分隔线；如果用户拖宽 pane，应重新评估并排条件。

- 1280×800 默认能并排显示 nav、main、inspector；inspector 的 left 大于 main 的 left 且 top 与 main 对齐。
- 空间不足先关闭/抽屉化 inspector；继续不足再收起 nav，并给可见展开按钮。
- 720×480（当前窗口最小值）必须能看到 composer、发送/停止、权限决策和关闭抽屉；不能出现整个页面水平滚动。
- modal 抽屉打开时有遮罩、Escape、焦点圈定和触发器返回焦点；关闭后活动行/草稿/滚动位置不丢。
- 不选模态时必须保证抽屉不遮挡关键按钮，也不能设置假的 aria-modal。两种行为择一形成契约。
- 对话/导航/inspector 各自 `min-height:0` 并独立 overflow；不靠 `overflow:hidden` 隐藏无法访问的内容。

代码可用显式 grid-template-areas 与 CSS 自定义 pane 宽度，具体实现由现有组件布局决定。测试应检查结果坐标，不把 CSS 字符串相同当作验收。

### 长消息和工具内容

- 用户/代理消息用可读文档流，不做左右浮动气泡。
- 代码块可横向局部滚动；页面本身不横向滚动。
- 超长 URL、路径和无空格文本折行或局部横向滚动，关键路径可以复制完整值。
- 工具日志默认有限摘要和展开按钮；展开后按需加载，正文数据仍由 Core 保存。
- Streaming 行高度变化时重新测量；用户在底部才跟随；用户阅读历史时不抢滚动。锚点包含行 ID 和行内像素偏移。
- 新活动通知按实际新消息/事件聚合，不把 React 重渲染次数显示成“新消息数”。

## 3. 色彩和主题

### 保留的基础色

| Token | 浅色 | 深色 | 用法 |
|---|---|---|---|
| canvas | `#f6f7f9` | `#14161a` | 工作区底色 |
| surface | `#ffffff` | `#1b1e24` | 导航、表单、工具栏 |
| elevated | `#ffffff` | `#22262d` | 菜单/对话框 |
| text | `#1c1f24` | `#e4e7ec` | 正文和主要标签 |
| muted | `#5b616b` | `#9aa1ac` | 次级说明 |
| accent | `#2f5fd7` | `#7ba0f0` | 主动作、链接 |
| danger | `#b3362b` | `#e2766c` | 错误、危险操作 |
| warning | `#946200` | `#d8a53f` | 待注意/降级 |
| success | `#2e7d43` | `#6fbf8a` | 确认成功 |
| selection | `#dbe4fb` | `#26314a` | 当前选中背景 |
| border | `#d4d7dc` | `#333841` | 装饰分割线 |
| focus | `#2f5fd7` | `#7ba0f0` | 键盘焦点 |

不要因为换了 AI 就重新选择主色。先修已知不足，设计变更必须有对比图和理由。

### 必须补足的语义

候选 token 名称（最终命名和设计契约统一）：

- `color-control-border`：需要可辨识边界的输入框，候选浅 `#858c97`、深 `#707b8c`；与真实背景重新计算对比度后采用。
- `color-accent-foreground`：实心主按钮文字；浅色蓝底可用 `#ffffff`，深色浅蓝底建议 `#14161a`，不能两种主题都机械用白字。
- `color-hover`、`color-pressed`：与 selected 分离，允许组件共享明确状态。
- `color-warning-surface`、`color-danger-surface`、`color-success-surface`：提示背景，按主题设计，不在 feature CSS 写 rgba。
- `color-overlay`：模态遮罩；`elevation-dialog` 与 `z-dialog` 使用已有变量。
- `color-disabled-text` / `color-disabled-surface`：禁用不冒充交互就绪，不用全元素 opacity 误伤说明文案。

主题设置只提供 `system / light / dark`（跟随系统/浅色/深色）；保存的是 source，解析后的 light/dark 不覆盖 system 偏好。系统变化使用媒体查询订阅；组件不能各自猜主题。设置 `color-scheme` 使原生控件合理匹配，仍要检查浏览器/Tauri 实际渲染。

### 对比度基线（本次计算）

| 前景 / surface | 浅色 | 深色 |
|---|---:|---:|
| 正文 text | 16.52:1 | 13.47:1 |
| 次级 muted | 6.24:1 | 6.41:1 |
| accent | 5.62:1 | 6.46:1 |
| danger | 6.04:1 | 5.60:1 |
| warning | 5.24:1 | 7.45:1 |
| success | 5.08:1 | 7.55:1 |
| border | 1.44:1 | 1.42:1 |

这些是 tokens 的 sRGB 纯色组合，不包含 opacity、叠层、hover、选中背景、字体抗锯齿，也不代表整页无障碍认证。

规则：普通文字目标 ≥4.5:1，必要的控件边界/状态标识目标 ≥3:1；装饰线不强制 3:1。状态同时配文字或图标。焦点清晰且不被遮挡。以 [WCAG 2.2](https://www.w3.org/TR/WCAG22/) 为参照，项目主动提供至少 24×24 CSS px 点击区域，拖拽分隔线视觉可以 1–4px，但可操作区要扩大或提供可达键盘等效入口。

`forced-colors` 使用系统颜色与清晰边框；不要用 box-shadow 作为唯一焦点。尊重 reduced-motion，不能用关闭动画掩盖延迟或无反馈。

## 4. 排版、控件与间距

- UI 字体：沿用 Segoe UI/system-ui，可显式补 `Microsoft YaHei UI`、`PingFang SC`、`Noto Sans CJK SC` 作为本机回退；不默认下载远程字体。
- 正文建议在既有 14px 基础上增加可选 16px 舒适字号；默认值变更先比较实际中英截图。中文正文 line-height 1.6，紧凑标签 1.35，代码 1.5，全部令牌化。
- 代码沿用 Cascadia Code/Consolas/ui-monospace；中文在代码里允许字体回退；不能省略字符。
- 间距沿用 2/4/6/8/12/16/24/32；不要再增加 7、13、19 等随机值。
- 按钮保留紧凑 28px；表单可采用统一 32px 舒适规格，先增令牌和对照截图，不强制所有按钮放大。
- 主操作一处突出：默认发送为主操作；运行中停止可见；审批“允许/拒绝”明确独立，不能默认批准。
- 聚焦 border/outline 与 selected 背景独立。只有 `:focus-visible` 显示键盘焦点，不让所有选中行看似获得键盘焦点。
- 所有按钮（含审批、弹窗、代码复制）使用共享 Button 等原语；禁止跨 CSS Modules 偷借同名 class 覆盖。
- ErrorSummary、InlineError、EmptyState、LoadingState、Dialog、Tabs、Tooltip、PaneSeparator 只负责呈现/行为原语；IPC 仍由 feature/store action 处理。

## 5. 关键流程

### 首次配置代理

空状态标题：“连接一个 ACP 代理” / “Connect an ACP agent”。正文说明代理由用户在本机安装并完成其自己的认证。主操作“添加代理”。

表单先展示显示名称、程序路径；高级展开参数列表、环境变量引用映射和工作目录等实际支持字段。显示每个参数独立行，不能用空白拆分整串参数。

测试状态：未测试、测试中、可连接、测试失败。成功显示协商能力；无法探测不假称全部支持。编辑任何相关字段使先前结果失效。测试只证明那一份配置，不保证日后网络/提供商始终可用。

保存错误就地反馈并保留非敏感字段；输入普通字符串到 Secret Ref 时拒绝且解释，不能生成不存在的引用。提供商登录与计费留给外部代理；不新增 Altior 账号。

### 发送与连续性

组合输入期间 Enter 只确认输入法；Shift+Enter 换行。若以后增加 Ctrl+Enter 发消息偏好，需要 locale-independent 快捷键契约与测试。

发送前捕获 thread/config/文本快照；不能因用户立即切换会话而发往另一个目标。同线程已有活动轮次时，只显示真正协商支持的操作；不假装 steering 成普通第二次 prompt。

无投递确认/不确定投递/明确拒绝分开。示例：

| 状态 | 中文 | English |
|---|---|---|
| Core 暂不可用 | 本地服务暂不可用。你的输入已保留。 | The local service is unavailable. Your input is preserved. |
| 投递不确定 | 无法确认这条请求是否已经执行。请先查看会话状态，再决定是否重新提交。 | We cannot confirm whether this request ran. Check the conversation state before submitting again. |
| 取消请求中 | 正在请求停止… | Requesting stop… |
| 取消失败 | 未能确认停止，代理可能仍在运行。 | Stop was not confirmed. The agent may still be running. |
| 历史加载失败 | 无法加载更多历史。已显示的内容仍可阅读。 | More history could not be loaded. The visible content is still available. |
| 无搜索结果 | 没有匹配的会话。试试其他关键词。 | No matching conversations. Try another search. |

不能把所有 error 都翻译成“网络错误”；Core 本地断线与外部模型离线不同。离线承诺指本地读写和已可用本地代理路径不被 relay/network gate 阻塞，不能承诺依赖云端的第三方模型无网也能生成。

### 记忆与上下文

记忆主界面用表格/列表：内容摘要、作用域、类型、来源、置信度、状态、更新时间；置信度明确是系统记录值，不表现成事实正确概率。

选中后共享 inspector 显示来源和操作：候选确认、纠错、拒绝、忘记；Off/Session/LongTerm 的作用域含义与 MEMORY.md 一致。

上下文面板按“此次发送实际选中的轮次”查询，显示身份材料、记忆、为什么选中、预算估算、被排除条目及原因。绝不把 UI 当前文本冒充过去发送的快照。

“忘记”提示仅描述实现承诺；有审计保留时说明“不再用于后续回答，历史审计记录按保留策略保存”，不能写“已永久擦除所有副本”。

## 6. 中英文系统

### 范围和文件组织

首批支持 `zh-CN` 与 `en`；默认跟随系统语言，用户可以显式覆盖，偏好设备本地。未知 locale 回退到 en，但开发/测试缺 key 应失败，不能默默用 key 名称显示。

提议 `src/i18n/` 保存 typed key、两份字典、formatters、React binding。这是候选路径，实施前查实际仓库组织。简单 typed 字典足以起步；引入第三方 i18n 框架需遵循 ADR，不为了两种语言新增不必要依赖。

必须翻译：导航、按钮、placeholder、aria-label、tooltip、空/错/等待状态、弹窗、状态栏、菜单、记忆元数据、帮助和 user-facing 错误建议。

不要翻译：Rust/TS 枚举 wire value、错误 code、ID、程序路径、参数、用户内容、提供商输出、模型标识、代码、协议名。技术错误 detail 用原文受限展示，友好 summary 从 code 本地化。不要字符串拼接英文句子再替换片段。

### 术语表

| Domain / English UI | 中文 UI | 说明 |
|---|---|---|
| Personal Vault | 个人知识库 | 设置/安全帮助首次说明 Vault 为个人所有权与加密同步边界 |
| Conversation (domain: Thread) | 会话 | 面向用户统一，不与“线程”混用；贡献者文档保留 Thread |
| Turn | 轮次 | 一次请求及其结果；必要时解释“本轮请求” |
| Agent | 代理 | 首次引导可写“AI 代理”；不与模型混为一谈 |
| Harness | 执行后端 | 只在高级设置出现，ACP 原样保留 |
| Model | 模型 | 仅协商支持且能下发才展示 |
| Project | 项目 | 本机项目关联，不暗示自动源码同步 |
| Memory | 记忆 | 不是整个会话正文 |
| Context | 上下文 | 本轮使用的身份材料和检索材料 |
| Source / Provenance | 来源 / 来源记录 | 普通界面用来源，细节可用来源记录 |
| Scope | 作用域 | 全局/当前项目/当前会话等 |
| Candidate / Confirmed | 待确认 / 已确认 | 模型推断默认待确认 |
| Superseded / Forgotten | 已替代 / 已忘记 | 不等于历史字节消失 |
| Permission request | 权限请求 | 显示确切动作与范围 |
| Allow / Deny | 允许 / 拒绝 | 同一 UI 不混用 Approve/Allow 三种叫法 |
| Stop / Stopping | 停止 / 正在停止 | 内部状态仍遵守 cancel 契约 |
| Pinned / Recent / Archived | 已置顶 / 最近 / 已归档 | 归档不等于删除 |
| Local only / Synced | 仅本机 / 已同步 | 只有实现真正确认后才显示已同步 |
| Settings / Diagnostics | 设置 / 诊断 | 工程细节放诊断 |
| System / Light / Dark | 跟随系统 / 浅色 / 深色 | 主题来源和值分开 |

### 格式化与内容

- Intl.DateTimeFormat/NumberFormat/RelativeTimeFormat；用注入时钟生成可测“刚刚/2 分钟前”。
- 工具提示显示完整本地日期时间和时区；排序仍用原始 timestamp，不能按格式化字符串排序。
- 字符截断按 grapheme，兼容 emoji、组合字符；UTF-8 bytes 上限在边界说明，不误标为“最多 N 个汉字”。
- 英文复数用完整消息模板；中文不硬套复数。
- locale 变化更新 document.documentElement.lang，不能把所有中文正文都翻译成英文。
- 不使用国旗表示语言，不用机器翻译真实私密记忆。
- README 可维护英文主文和中文入口；中英文都要明确预览状态。协议/ADR 可保留英文，中文摘要不能取代契约。

## 7. 验收矩阵

至少检查 `zh-CN/en × light/dark × 1280×800/760×800/720×480` 的关键状态；system 主题变化另测。不是每个页面无差别截图数百张，按共享布局和状态覆盖有代表性的组合。

必要画面：干净 Vault、已连接会话、streaming、权限等待、拒绝/取消失败、不确定投递、无搜索结果、配置错误、Context 有/无/错、记忆候选与忘记、设置。

必要操作：全键盘走完配置→创建→发送→审批→停止；弹窗 Escape/Tab/Shift+Tab/焦点归还；中文 IME；长标题/路径/代码；200% 缩放；高对比；reduced-motion；叠层中关键按钮可达。

自动化检查：真实 document geometry、未处理 pageerror、可访问名称、role/state、语言 key 对齐、对比度、被测状态 ready 标志。读屏至少在参考 Windows 环境人工验证一次，工具不能仅凭 aria 静态扫描宣布通过。

视觉基线：固定 OS、浏览器版本、字体、deviceScaleFactor、时钟、数据与 locale。只允许人工/独立视角审阅差异后更新。测试 timeout 是防挂上限，不能用固定 sleep 等待界面“差不多好了”。
