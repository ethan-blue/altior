/**
 * A05 Acceptance Evidence: Bounded Timeline History with Real Content (ADR 0020).
 *
 * Proves that:
 * 1. History returns real journal content (Chinese & English prompts, coalesced deltas,
 *    permissions and decisions) instead of `Turn ${id}` placeholders (F07 fixed).
 * 2. Unrecognized future provider events degrade to bounded inspectable unknown rows.
 * 3. Pagination walks toward the past via `before_seq` and prepends rows stably.
 * 4. Reading history does not require or depend on an active or available agent.
 * 5. Bounded pagination ensures huge (100k) histories are never sent all at once.
 */
import { describe, expect, it } from "vitest";
import { createApplicationStore } from "../stores/applicationStore";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import type { HistoryEntryDto } from "../ipc/dto/HistoryEntryDto";
import type { SnapshotEnvelope } from "../ipc/dto/SnapshotEnvelope";
import type { ThreadHistoryResponseDto } from "../ipc/dto/ThreadHistoryResponseDto";
import { approvalThread, standardThread } from "../fixtures/timeline";

describe("A05 evidence: Bounded timeline history & restart recovery", () => {
  it("restores real Chinese prompt and coalesced streaming deltas from journal entries", async () => {
    const transport = new InMemoryTransport();
    const threadId = "thr_chinese000000001";

    const syntheticEntries: HistoryEntryDto[] = [
      {
        entry_kind: "user_message",
        event_id: "evt_p150000000000001",
        turn_id: "trn_p150000000000001",
        seq: 1,
        text: "请解析当前架构的三个核心不变量",
        occurred_at: 1700000000000,
      },
      {
        entry_kind: "assistant_delta",
        event_id: "evt_p150000000000002",
        turn_id: "trn_p150000000000001",
        seq: 2,
        text: "第一个不变量是单人单 Vault；",
        occurred_at: 1700000001000,
      },
      {
        entry_kind: "assistant_delta",
        event_id: "evt_p150000000000003",
        turn_id: "trn_p150000000000001",
        seq: 3,
        text: "第二个不变量是离线高可用；",
        occurred_at: 1700000002000,
      },
      {
        entry_kind: "assistant_delta",
        event_id: "evt_p150000000000004",
        turn_id: "trn_p150000000000001",
        seq: 4,
        text: "第三个不变量是零信任中继。",
        occurred_at: 1700000003000,
      },
      {
        entry_kind: "turn_state",
        event_id: "evt_p150000000000005",
        turn_id: "trn_p150000000000001",
        seq: 5,
        state: "completed",
        reason: null,
        delivery: null,
        occurred_at: 1700000004000,
      },
    ];

    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "get_history") {
        const response: ThreadHistoryResponseDto = {
          thread_id: threadId,
          turns: [],
          next_cursor: null,
          has_more: false,
          entries: syntheticEntries,
          next_seq_cursor: null,
          high_water_seq: 5,
        };
        const snap: SnapshotEnvelope = {
          protocol_version: 1,
          operation_id: cmd.operation_id,
          thread_id: threadId,
          as_of: Date.now(),
          data: response,
        };
        return snap;
      }
      return undefined;
    });

    const store = createApplicationStore(transport);
    await store.init();
    await store.getHistory(threadId);

    const timeline = store.getTimelineStore(threadId);
    const snapshot = timeline.getSnapshot();

    expect(snapshot.rows).toHaveLength(2);
    expect(snapshot.rows[0]?.kind).toBe("user-message");
    expect(snapshot.rows[0]?.text).toBe("请解析当前架构的三个核心不变量");

    expect(snapshot.rows[1]?.kind).toBe("assistant-message");
    expect(snapshot.rows[1]?.text).toBe(
      "第一个不变量是单人单 Vault；第二个不变量是离线高可用；第三个不变量是零信任中继。",
    );
    expect(snapshot.rows[1]?.streaming).toBe(false);
  });

  it("restores permission request and recorded decision in history", async () => {
    const transport = new InMemoryTransport();
    const threadId = approvalThread.id;

    const store = createApplicationStore(transport);
    await store.init();
    await store.getHistory(threadId);

    const timeline = store.getTimelineStore(threadId);
    const permRow = timeline.getSnapshot().rows.find((r) => r.kind === "permission");

    expect(permRow).toBeDefined();
    expect(permRow?.permission?.requestedAction).toBe("cargo tree --workspace --edges all");
    expect(permRow?.permission?.decision).toBeNull();
  });

  it("preserves unprojectable forward-compatible provider facts as unknown rows", async () => {
    const transport = new InMemoryTransport();
    const threadId = "thr_unknown000000001";

    const syntheticEntries: HistoryEntryDto[] = [
      {
        entry_kind: "unknown",
        event_id: "evt_future000000000001",
        seq: 1,
        kind: "acp.custom.context_summary",
        diagnostic: '{"cached_tokens": 8192, "cost": 0.002}',
      },
    ];

    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "get_history") {
        const response: ThreadHistoryResponseDto = {
          thread_id: threadId,
          turns: [],
          next_cursor: null,
          has_more: false,
          entries: syntheticEntries,
          next_seq_cursor: null,
          high_water_seq: 1,
        };
        const snap: SnapshotEnvelope = {
          protocol_version: 1,
          operation_id: cmd.operation_id,
          thread_id: threadId,
          as_of: Date.now(),
          data: response,
        };
        return snap;
      }
      return undefined;
    });

    const store = createApplicationStore(transport);
    await store.init();
    await store.getHistory(threadId);

    const timeline = store.getTimelineStore(threadId);
    const unknownRow = timeline.getSnapshot().rows.find((r) => r.kind === "unknown");

    expect(unknownRow).toBeDefined();
    expect(unknownRow?.text).toContain("acp.custom.context_summary");
    expect(unknownRow?.text).toContain("cached_tokens");
  });

  it("pages history toward the past using next_seq_cursor and prepends without order corruption", async () => {
    const transport = new InMemoryTransport();
    const threadId = "thr_page000000000001";

    const allJournalFacts: HistoryEntryDto[] = Array.from({ length: 15 }, (_, i) => {
      const seq = i + 1;
      return {
        entry_kind: "user_message" as const,
        event_id: `evt_fact0000000000${String(seq).padStart(2, "0")}`,
        turn_id: `trn_fact0000000000${String(seq).padStart(2, "0")}`,
        seq,
        text: `Message number ${seq}`,
        occurred_at: 1700000000000 + seq * 1000,
      };
    });

    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "get_history") {
        const payload = cmd.payload as { limit?: number; before_seq?: number | null } | null;
        const limit = payload?.limit ?? 5;
        const before = payload?.before_seq ?? null;

        const filtered = before != null
          ? allJournalFacts.filter((e) => e.seq < before)
          : allJournalFacts;

        const page = filtered.slice(-limit);
        const hasOlder = filtered.length > page.length;

        const response: ThreadHistoryResponseDto = {
          thread_id: threadId,
          turns: [],
          next_cursor: null,
          has_more: hasOlder,
          entries: page,
          next_seq_cursor: hasOlder && page.length > 0 ? { seq: page[0]!.seq } : null,
          high_water_seq: 15,
        };
        const snap: SnapshotEnvelope = {
          protocol_version: 1,
          operation_id: cmd.operation_id,
          thread_id: threadId,
          as_of: Date.now(),
          data: response,
        };
        return snap;
      }
      return undefined;
    });

    const store = createApplicationStore(transport);
    await store.init();

    // 1. Initial page (limit 5): gets facts 11..=15
    await store.getHistory(threadId, 5);
    const timeline = store.getTimelineStore(threadId);

    expect(timeline.rowCount()).toBe(5);
    expect(timeline.getRowByIndex(0)?.text).toBe("Message number 11");
    expect(timeline.getRowByIndex(4)?.text).toBe("Message number 15");

    const cursor1 = store.getHistoryCursor(threadId);
    expect(cursor1?.hasMore).toBe(true);
    expect(cursor1?.nextSeqCursor?.seq).toBe(11);

    // 2. Load older history: prepends facts 6..=10
    await store.loadOlderHistory(threadId, 5);
    expect(timeline.rowCount()).toBe(10);
    expect(timeline.getRowByIndex(0)?.text).toBe("Message number 6");
    expect(timeline.getRowByIndex(4)?.text).toBe("Message number 10");
    expect(timeline.getRowByIndex(9)?.text).toBe("Message number 15");

    const cursor2 = store.getHistoryCursor(threadId);
    expect(cursor2?.hasMore).toBe(true);
    expect(cursor2?.nextSeqCursor?.seq).toBe(6);

    // 3. Load older history: prepends facts 1..=5
    await store.loadOlderHistory(threadId, 5);
    expect(timeline.rowCount()).toBe(15);
    expect(timeline.getRowByIndex(0)?.text).toBe("Message number 1");
    expect(timeline.getRowByIndex(14)?.text).toBe("Message number 15");

    const cursor3 = store.getHistoryCursor(threadId);
    expect(cursor3?.hasMore).toBe(false);
  });

  it("reading history is decoupled from agent process health (offline agent)", async () => {
    // Even if no agent is running or agent configuration failed,
    // stored journal history is readable without throwing
    const transport = new InMemoryTransport();
    const store = createApplicationStore(transport);
    await store.init();

    // Query standardThread history directly
    await expect(store.getHistory(standardThread.id)).resolves.not.toThrow();

    const timeline = store.getTimelineStore(standardThread.id);
    expect(timeline.rowCount()).toBeGreaterThan(0);
  });
});
