import { describe, expect, it } from "vitest";
import type { HistoryEntryDto } from "../../ipc/dto/HistoryEntryDto";
import { reduceHistoryEntries } from "./historyReducer";

describe("reduceHistoryEntries (A05 / ADR 0020)", () => {
  it("reduces user messages and coalesces streaming deltas for the same turn", () => {
    const entries: HistoryEntryDto[] = [
      {
        entry_kind: "user_message",
        event_id: "evt_0000000000000001",
        turn_id: "trn_0000000000000001",
        seq: 1,
        text: "请帮我总结这份文档的核心架构",
        occurred_at: 1700000000000,
      },
      {
        entry_kind: "assistant_delta",
        event_id: "evt_0000000000000002",
        turn_id: "trn_0000000000000001",
        seq: 2,
        text: "这份文档介绍了",
        occurred_at: 1700000001000,
      },
      {
        entry_kind: "assistant_delta",
        event_id: "evt_0000000000000003",
        turn_id: "trn_0000000000000001",
        seq: 3,
        text: "本地优先架构的三层设计。",
        occurred_at: 1700000002000,
      },
      {
        entry_kind: "turn_state",
        event_id: "evt_0000000000000004",
        turn_id: "trn_0000000000000001",
        seq: 4,
        state: "completed",
        reason: null,
        delivery: null,
        occurred_at: 1700000003000,
      },
    ];

    const rows = reduceHistoryEntries(entries);
    expect(rows).toHaveLength(2);

    expect(rows[0]).toEqual({
      id: "evt_0000000000000001",
      kind: "user-message",
      text: "请帮我总结这份文档的核心架构",
      status: null,
      permission: null,
      streaming: false,
    });

    expect(rows[1]).toEqual({
      id: "trn_0000000000000001",
      kind: "assistant-message",
      text: "这份文档介绍了本地优先架构的三层设计。",
      status: null,
      permission: null,
      streaming: false,
    });
  });

  it("applies permission decisions to corresponding permission requests", () => {
    const entries: HistoryEntryDto[] = [
      {
        entry_kind: "permission",
        event_id: "evt_perm000000000001",
        turn_id: "trn_0000000000000002",
        seq: 1,
        permission_kind: "execute",
        description: "cargo test --workspace",
        decision: "pending",
        occurred_at: 1700000000000,
      },
      {
        entry_kind: "permission_decision",
        event_id: "evt_dec0000000000001",
        permission_event_id: "evt_perm000000000001",
        turn_id: "trn_0000000000000002",
        seq: 2,
        decision: "approved",
        occurred_at: 1700000001000,
      },
    ];

    const rows = reduceHistoryEntries(entries);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.kind).toBe("permission");
    expect(rows[0]?.permission?.decision).toBe("approved");
    expect(rows[0]?.permission?.requestedAction).toBe("cargo test --workspace");
  });

  it("preserves unknown future provider events verbatim", () => {
    const entries: HistoryEntryDto[] = [
      {
        entry_kind: "unknown",
        event_id: "evt_unknown000000001",
        seq: 1,
        kind: "acp.custom.metric",
        diagnostic: '{"tokens": 42}',
      },
    ];

    const rows = reduceHistoryEntries(entries);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.kind).toBe("unknown");
    expect(rows[0]?.text).toBe('acp.custom.metric: {"tokens": 42}');
  });
});
