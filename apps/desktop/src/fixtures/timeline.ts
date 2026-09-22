/**
 * Synthetic timeline fixtures (ADR 0008 §1).
 *
 * The row model is provisional P0.4 UI shape shaped to match the P0.3
 * normalized agent events and the preserved-unknown rule; P1 replaces it
 * with the frozen event taxonomy. Content is index-derived — no random,
 * no real conversations, no secrets.
 *
 * Identities that reach the wire (thread ids on commands, message-row ids
 * replayed as turn ids, permission-row ids replayed as event ids) follow
 * the `altior-domain` `<prefix>_<16..64 of [0-9a-z]>` rules (A01, ADR 0019)
 * so the fixture transport validates them exactly like real Core. Row kinds
 * that never leave the UI (tool, error, unknown) keep local display ids.
 */
import type { TimelineRow, ToolStatus } from "../features/timeline/timelineStore";

export type ThreadStatus = "running" | "waiting-for-permission" | "failed" | "completed";

export interface ThreadFixture {
  readonly id: string;
  readonly title: string;
  readonly agent: string;
  readonly status: ThreadStatus;
  readonly pinned: boolean;
  readonly rows: readonly TimelineRow[];
}

function user(id: string, text: string): TimelineRow {
  return { id, kind: "user-message", text, status: null, permission: null, streaming: false };
}

function assistant(id: string, text: string, streaming = false): TimelineRow {
  return {
    id,
    kind: "assistant-message",
    text,
    status: null,
    permission: null,
    streaming,
  };
}

function tool(id: string, summary: string, status: ToolStatus): TimelineRow {
  return { id, kind: "tool", text: summary, status, permission: null, streaming: false };
}

function permission(
  id: string,
  requestedAction: string,
  scope: string,
): TimelineRow {
  return {
    id,
    kind: "permission",
    text: requestedAction,
    status: null,
    permission: { requestedAction, scope, decision: null },
    streaming: false,
  };
}

function error(id: string, diagnostic: string): TimelineRow {
  return { id, kind: "error", text: diagnostic, status: null, permission: null, streaming: false };
}

function unknownEvent(id: string, providerKind: string): TimelineRow {
  return {
    id,
    kind: "unknown",
    text: `${providerKind}: 协议 v1 未识别；原文保留`,
    status: null,
    permission: null,
    streaming: false,
  };
}

/** A normal completed thread: prompt, tool, answered permission, reply. */
export const standardThread: ThreadFixture = {
  id: "thr_fixture000000001",
  title: "契约夹具演练",
  agent: "alpha (ACP)",
  status: "completed",
  pinned: true,
  rows: [
    user("trn_fixture000000101", "用三条要点概括 P0.2 IPC 契约。"),
    tool("std-2", "rg --files crates/altior-ipc", "completed"),
    permission("evt_fixture000000101", "read crates/altior-ipc/src", "project:altior"),
    user("trn_fixture000000103", "[已批准]"),
    assistant(
      "trn_fixture000000105",
      "帧为 4 字节长度前缀，上限 256 KiB；会话共享单次启动的事件日志；重载是在同一日志上的新连接。",
    ),
    unknownEvent("std-6", "acp.update.plan"),
  ],
};

/** A thread parked on an unanswered permission request. */
export const approvalThread: ThreadFixture = {
  id: "thr_fixture000000002",
  title: "依赖审计（需审批）",
  agent: "alpha (ACP)",
  status: "waiting-for-permission",
  pinned: false,
  rows: [
    user("trn_fixture000000111", "审计工作区依赖并标出风险项。"),
    tool("apr-2", "cargo tree --workspace", "completed"),
    permission("evt_fixture000000111", "cargo tree --workspace --edges all", "project:altior"),
    assistant("trn_fixture000000113", "在读取完整依赖图之前，等待你的决定。", true),
  ],
};

/** A failed turn: error diagnostics and an indeterminate delivery note. */
export const failureThread: ThreadFixture = {
  id: "thr_fixture000000003",
  title: "中断的尖峰试跑",
  agent: "beta (ACP)",
  status: "failed",
  pinned: false,
  rows: [
    user("trn_fixture000000121", "起草中继尖峰试跑大纲。"),
    assistant("trn_fixture000000123", "中继需要信封格式、确认语义，以及…", true),
    error("fai-3", "轮次已停止：拒绝 — 代理婉拒了此请求"),
    error("fai-4", "投递：不确定（进程在轮次中途退出）；不重发"),
  ],
};

/**
 * The acceptance-size thread: exactly 100,000 deterministic rows mixing
 * messages, tools, and the occasional preserved unknown.
 */
export function hundredThousandRowThread(): ThreadFixture {
  const rows: TimelineRow[] = new Array<TimelineRow>(100_000);
  for (let index = 0; index < 100_000; index += 1) {
    const id = `trn_big${String(index).padStart(13, "0")}`;
    if (index % 97 === 96) {
      rows[index] = unknownEvent(`big-${index}`, "acp.update.usage");
    } else if (index % 11 === 10) {
      rows[index] = tool(`big-tool-${index}`, `scan batch ${index}`, "completed");
    } else if (index % 2 === 0) {
      rows[index] = user(id, `deterministic question ${index}`);
    } else {
      rows[index] = assistant(id, `deterministic answer ${index} about contracts.`);
    }
  }
  return {
    id: "thr_fixture000000100",
    title: "10万行历史（验收规模）",
    agent: "alpha (ACP)",
    status: "completed",
    pinned: false,
    rows,
  };
}

/** Older history that can be prepended (prepend-stability evidence). */
export function olderHistory(count: number): TimelineRow[] {
  const rows: TimelineRow[] = new Array<TimelineRow>(count);
  for (let index = 0; index < count; index += 1) {
    const seq = count - index;
    const id = `trn_old${String(seq).padStart(13, "0")}`;
    rows[index] =
      index % 2 === 0
        ? user(id, `older question ${seq}`)
        : assistant(id, `older answer ${seq}.`);
  }
  return rows;
}

/**
 * The deterministic streaming script a send triggers in the fixture
 * shell: chunked deltas in the P0.3 trace vocabulary, then completion.
 */
export const streamingReplyChunks: readonly string[] = [
  "Frames are ",
  "length-prefixed; ",
  "sessions replay ",
  "through a retained window; ",
  "reload never stops a turn.",
];

export function allThreads(includeHuge: boolean): ThreadFixture[] {
  const threads = [standardThread, approvalThread, failureThread];
  if (includeHuge) {
    threads.push(hundredThousandRowThread());
  }
  return threads;
}
