## 硬性规则（违反任何一条都会被审查判 FAIL）

- 需求来源只有任务单。只做 In scope；Out of scope 一律不碰。不顺手重构、不改无关文件、不整文件重排格式。
- 不得运行 git add / commit / push / checkout / reset / restore / stash / clean；提交由执行脚本负责。
- 不引入新的 Cargo/npm 依赖、框架、协议或持久化格式，除非任务单明确要求且已有对应 ADR。
- 不修改已发布的迁移，只能新增迁移。不更新视觉基线（`npm run baselines`），除非任务单明确允许并且你在证据文件里描述了你看过的差异。
- 不删除、跳过、放宽已有测试或断言来"变绿"。测试不得依赖真实 sleep、公网、机器负载或随机 ID；用 fake clock、固定 ID、内存传输。
- 秘密、凭证、私钥、secret 形状的字符串不得进入代码、fixture、日志、快照、错误详情。
- Desktop 只是 Core 的客户端：不直接打开数据库、不启动 agent、不读秘密、不拼模型上下文。
- `altior-domain` 不依赖任何基础设施；DTO 只在 `altior-protocol` 定义，TS DTO 由 `cargo test -p altior-protocol --features dto-export` 生成，禁止手改 `apps/desktop/src/ipc/dto/`。
- 同步保持 `sync_enabled = false` 屏障；不实现、不修改任何密码学原语。
- UI：只使用 `apps/desktop/src/styles/tokens.css` 的语义 token（不写裸颜色/间距/圆角/阴影）；新增文案必须同时加入 `i18n/en.ts` 与 `i18n/zh-CN.ts`（typed keys），不翻译 wire enum / ID / 路径 / 用户正文；保持键盘可达与可见焦点；IME 合成中的 Enter 不得触发发送。
- 不把占位、mock 正常路径或只能编译的接缝标记为完成。无法验证的部分如实写"未验证"。
- 与 AGENTS.md 或已接受 ADR 冲突、需要外部凭证、需要产品决策时：不要硬做。停止改动，最后输出 `STATUS: BLOCKED`，写清原因与解除条件。

## 环境提示

- 你在一个隔离的 git worktree 中，路径就是当前目录。仓库在 Windows 上，shell 命令用 Bash 工具执行。
- 前端命令用 `npm --prefix apps/desktop run <script>`；Vitest 单文件：`npm --prefix apps/desktop exec vitest run <path>`。
- Rust 命令默认从仓库根运行；Tauri 壳是独立 crate：`--manifest-path apps/desktop/src-tauri/Cargo.toml`。
- 编译可能较慢，先跑与改动最相关的窄测试，最后跑任务单要求的全部验证命令。

## 证据文件（必需）

新建或更新 `{{EVIDENCE_PATH}}`，包含：

1. Work brief：Outcome / Acceptance / In scope / Out of scope / Contracts touched / Failure & security / Migration（纯 UI 写"无格式变更"）
2. 改动文件列表及每个文件的理由
3. 你实际运行过的命令与真实结果（通过数 / 失败数）；没跑过的不写
4. 未验证项与残留风险

不要编辑 `CHECKPOINT.md`（执行脚本负责记录）。

## 结束格式

最后一行必须单独输出以下之一（不加其他文字）：

STATUS: DONE
STATUS: PARTIAL
STATUS: BLOCKED
