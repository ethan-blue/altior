你继续负责 Altior 任务 {{TASK_ID}}（{{TASK_TITLE}}），工作目录是同一个 git worktree。上一轮没有通过，原因如下：

{{FEEDBACK}}

## 怎么修

1. 先运行 `git status`、`git log --oneline {{START_POINT}}..HEAD` 与 `git diff {{START_POINT}}`，弄清当前状态（上一轮通过验证的改动可能已经被脚本提交）。
2. 只做修复上述问题所需的最小改动，不扩大范围。
3. 如果审查意见与任务单或 `AGENTS.md` 冲突，以任务单和 `AGENTS.md` 为准，不采纳该条，并在证据文件里写明理由。
4. 在证据文件 `{{EVIDENCE_PATH}}` 末尾追加"修复轮次"小节：问题 → 改动 → 重新运行的命令与结果。

## 任务单（任务包 {{PACK}}）

{{TASK_SPEC}}

{{RULES}}

## 验证命令

{{VERIFY}}

结束前你必须自己把这些命令全部跑通。
