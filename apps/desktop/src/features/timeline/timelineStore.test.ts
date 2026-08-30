import { describe, expect, it, vi } from "vitest";
import { createTimelineStore, type TimelineRow } from "./timelineStore";

function message(id: string, text: string, streaming = false): TimelineRow {
  return { id, kind: "assistant-message", text, status: null, permission: null, streaming };
}

describe("timeline store", () => {
  it("notifies only the streamed row's listeners for deltas", () => {
    const store = createTimelineStore([
      message("a", "first"),
      message("b", "second"),
      message("c", "third"),
    ]);
    const onA = vi.fn();
    const onB = vi.fn();
    const onStructure = vi.fn();
    store.subscribeRow("a", onA);
    store.subscribeRow("b", onB);
    store.subscribe(onStructure);

    store.appendDelta("b", " more");

    expect(store.getRow("b")?.text).toBe("second more");
    expect(onB).toHaveBeenCalledTimes(1);
    expect(onA).not.toHaveBeenCalled();
    expect(onStructure).not.toHaveBeenCalled();
  });

  it("bumps the structure version on append and prepend only", () => {
    const store = createTimelineStore([message("a", "a")]);
    const version0 = store.getSnapshot().structureVersion;
    store.appendDelta("a", "!");
    expect(store.getSnapshot().structureVersion).toBe(version0);
    store.appendRow(message("b", "b"));
    expect(store.getSnapshot().structureVersion).toBe(version0 + 1);
    store.prependRows([message("z", "z")]);
    expect(store.getSnapshot().structureVersion).toBe(version0 + 2);
    expect(store.getSnapshot().rows.map((row) => row.id)).toEqual(["z", "a", "b"]);
  });

  it("records a provisional permission decision on the row", () => {
    const store = createTimelineStore([
      {
        id: "p1",
        kind: "permission",
        text: "Run ripgrep across the project",
        status: null,
        permission: { requestedAction: "rg --files", scope: "project:demo", decision: null },
        streaming: false,
      },
    ]);
    expect(store.pendingPermission()?.id).toBe("p1");
    store.setPermissionDecision("p1", "approved");
    expect(store.getRow("p1")?.permission?.decision).toBe("approved");
    expect(store.pendingPermission()).toBeNull();
    expect(() => store.setPermissionDecision("p1", "denied")).not.toThrow();
    expect(() =>
      store.setPermissionDecision("missing", "denied"),
    ).toThrow(/holds no permission/);
  });

  it("scales to 100k rows and still updates a single row in place", () => {
    const rows = Array.from({ length: 100_000 }, (_, i) =>
      message(`row-${i}`, `row ${i}`),
    );
    const store = createTimelineStore(rows);
    expect(store.rowCount()).toBe(100_000);
    const onFirst = vi.fn();
    store.subscribeRow("row-0", onFirst);
    store.appendDelta("row-99999", "!");
    expect(store.getRow("row-99999")?.text).toBe("row 99999!");
    expect(onFirst).not.toHaveBeenCalled();
  });
});
