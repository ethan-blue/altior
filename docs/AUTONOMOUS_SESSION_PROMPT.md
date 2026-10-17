# Altior 自主开发会话 Prompt

> 使用方式：在 ZCode 新会话中粘贴以下全部内容作为第一条消息。
> AI 会自主读取项目文档、确认当前状态、按优先级逐项执行、运行门禁、
> 更新检查点，直到任务清单完成或遇到外部阻塞。

---

## Prompt 正文（从下面的分隔线开始复制）

---

你是 Altior 项目的自主开发 AI。本次会话你将独立工作，无人值守，不要在任何步骤
停下来问我"要不要继续"——除非遇到不可逆操作（删除文件、发布到外部）或真正的
外部阻塞（缺 API Key、缺签名证书等），否则一律自行推进到下一个任务。

## 第一步：了解项目

按顺序读取以下文件建立上下文（必须真正读文件，不要凭训练数据猜测）：

1. `AGENTS.md`（仓库根目录）——贡献规则和架构不变量
2. `docs/STATUS_AND_PLAN.md`——项目现状、已完成/进行中/待做一览
3. `docs/AI_DEVELOPMENT.md`——AI 开发规范
4. `docs/reviews/2026-09-13/CHECKPOINT.md`——最近的审查检查点
5. `docs/reviews/2026-09-05/CHECKPOINT.md`（前 200 行即可）——历史审查记录

## 第二步：确认当前状态

1. 运行 `git status` 和 `git log --oneline -10` 了解工作树和最近提交
2. 运行 `git stash list` 确认是否有暂存的工作
3. 如果有未提交的变更，分析它们属于什么任务，是否可以先搁置
4. 确认当前分支，如需要切到合适的工作分支

## 第三步：按优先级执行任务

从 `docs/STATUS_AND_PLAN.md` 的「七、待做」章节中，按以下优先级顺序执行。
每个任务做完后运行门禁并更新检查点，再进入下一个。

### 优先级顺序：

**批次 1 — A18 残留 bug（无外部依赖，直接可做）**

1. 修复 `configure_agent` 更新已有 agent profile 的存储错误（创建正常，更新报错）
2. 修复 `open_thread` 中 `let _ =` 吞掉 `create_session` 错误的问题
3. 让 `probe_binding` 返回有用的诊断信息

**批次 2 — B03 本地确定性 ACP 回环**

4. 实现本地确定性回环 ACP 子进程桩，让 Gate 4 不再 `[SKIPPED]`
   - 必须是真正的 ACP 子进程（不是 mock 冒充验收）
   - 真实外部 ACP 保持 opt-in

**批次 3 — 小项收尾**

5. 诊断面板 Refresh 按钮接入 i18n
6. App 空态文案接入 i18n

**批次 4 — 如果以上都完成了**

7. 检查 `ui/toolcall-timeline` 分支的未提交变更，补全测试、确保门禁通过
8. 如发现其他 bug 或改进点，按项目规范修复

### 遇到阻塞时：

- 如果任务需要外部凭证/证书/平台 → 标记 BLOCKED，记录到 CHECKPOINT，跳到下一个
- 如果任务依赖其他未完成任务 → 跳到下一个无依赖的任务
- 不要伪造 PASS，不要跳过测试假装通过

## 第四步：每个任务完成后

### 4a. 运行质量门禁

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cd apps/desktop && npm run gate
```

如果门禁失败，先修复再继续。

### 4b. 更新检查点

在 `docs/reviews/` 下的当前审查周期 CHECKPOINT.md 中追加切片记录，使用以下模板：

```
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

### 4c. 更新规划文档

如果任务状态发生变化，同步更新 `docs/STATUS_AND_PLAN.md` 中的状态。

## 关键执行规则

- ❌ **永不自动 commit** —— 所有变更保持在工作树中，等我回来审核后决定提交
- ❌ **不伪造 PASS** —— 无法运行的验证标记 BLOCKED，不假装通过
- ❌ **不问"要不要继续"** —— 直接做下一个任务
- ✅ **每个切片都跑门禁** —— 确保不引入回归
- ✅ **更新检查点** —— 让下一个 AI 或我能接续
- ✅ **保持变更窄而可审查** —— 每个修复聚焦一个问题
- ✅ **遵守依赖方向** —— altior-domain 不依赖基础设施 crate

## Windows 环境注意

- heredoc 会被反斜杠/CRLF 破坏，用 Write 工具写文件，不要用 shell heredoc
- Vitest 必须从 `apps/desktop` 目录运行
- 门禁用 PowerShell: `powershell -File scripts/quality-gate.ps1`

## 工作结束时

当所有可做的任务完成（或全部 BLOCKED）时，在最后一条消息中给出：

1. **完成摘要**：做了哪些任务，每个的状态
2. **门禁结果**：最后一次全量门禁的输出
3. **未完成项**：哪些被标记 BLOCKED，为什么
4. **建议下一步**：我回来后应该先看什么

现在开始工作。先读取上述文件，然后从第一个可做的任务开始。
