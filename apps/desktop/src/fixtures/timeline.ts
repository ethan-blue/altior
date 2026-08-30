/**
 * Synthetic timeline fixtures (ADR 0008 §1).
 *
 * The row model is provisional P0.4 UI shape shaped to match the P0.3
 * normalized agent events and the preserved-unknown rule; P1 replaces it
 * with the frozen event taxonomy. Content is index-derived — no random,
 * no real conversations, no secrets.
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
    text: `${providerKind}: unrecognized by protocol v1; preserved verbatim`,
    status: null,
    permission: null,
    streaming: false,
  };
}

/** A normal completed thread: prompt, tool, answered permission, reply. */
export const standardThread: ThreadFixture = {
  id: "fixture/standard",
  title: "Contract fixture walkthrough",
  agent: "alpha (ACP)",
  status: "completed",
  pinned: true,
  rows: [
    user("std-1", "Summarize the P0.2 IPC contract in three bullets."),
    tool("std-2", "rg --files crates/altior-ipc", "completed"),
    permission("std-3", "read crates/altior-ipc/src", "project:altior"),
    user("std-4", "[approved]"),
    assistant(
      "std-5",
      "Frames are 4-byte length-prefixed and capped at 256 KiB; sessions share one per-launch event log; reload is a new connection over the same log.",
    ),
    unknownEvent("std-6", "acp.update.plan"),
  ],
};

/** A thread parked on an unanswered permission request. */
export const approvalThread: ThreadFixture = {
  id: "fixture/approval",
  title: "Dependency audit with approvals",
  agent: "alpha (ACP)",
  status: "waiting-for-permission",
  pinned: false,
  rows: [
    user("apr-1", "Audit the workspace dependencies and flag anything risky."),
    tool("apr-2", "cargo tree --workspace", "completed"),
    permission("apr-3", "cargo tree --workspace --edges all", "project:altior"),
    assistant("apr-4", "Waiting for your decision before reading the full graph.", true),
  ],
};

/** A failed turn: error diagnostics and an indeterminate delivery note. */
export const failureThread: ThreadFixture = {
  id: "fixture/failure",
  title: "Interrupted spike run",
  agent: "beta (ACP)",
  status: "failed",
  pinned: false,
  rows: [
    user("fai-1", "Draft the relay spike outline."),
    assistant("fai-2", "The relay needs an envelope format, ack semantics, and…", true),
    error("fai-3", "turn stopped: refusal — the agent declined this request"),
    error("fai-4", "delivery: indeterminate (process exited mid-turn); no resend"),
  ],
};

/**
 * The acceptance-size thread: exactly 100,000 deterministic rows mixing
 * messages, tools, and the occasional preserved unknown.
 */
export function hundredThousandRowThread(): ThreadFixture {
  const rows: TimelineRow[] = new Array<TimelineRow>(100_000);
  for (let index = 0; index < 100_000; index += 1) {
    const id = `big-${index}`;
    if (index % 97 === 96) {
      rows[index] = unknownEvent(id, "acp.update.usage");
    } else if (index % 11 === 10) {
      rows[index] = tool(id, `scan batch ${index}`, "completed");
    } else if (index % 2 === 0) {
      rows[index] = user(id, `deterministic question ${index}`);
    } else {
      rows[index] = assistant(id, `deterministic answer ${index} about contracts.`);
    }
  }
  return {
    id: "fixture/hundred-thousand",
    title: "100,000-row history (acceptance size)",
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
    rows[index] =
      index % 2 === 0
        ? user(`old-${seq}`, `older question ${seq}`)
        : assistant(`old-${seq}`, `older answer ${seq}.`);
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
