/**
 * Timeline store: rows, coalesced streaming deltas, and provisional
 * permission decisions (ADR 0008 §4–5).
 *
 * The row model is the P0.4 provisional fixture shape; P1 swaps it for
 * the frozen event taxonomy behind this same interface. Structural
 * changes (append/prepend) bump `structureVersion` for the windowing
 * layer; delta text changes notify only the affected row's listeners, so
 * a streaming turn never re-renders the rest of the timeline.
 */

export type RowKind =
  | "user-message"
  | "assistant-message"
  | "tool"
  | "permission"
  | "error"
  | "unknown";

export type ToolStatus = "running" | "completed" | "failed";

/** A provisional UI decision; the P1 runtime owns the real command. */
export type PermissionDecision = "approved" | "denied";

export interface PermissionRequest {
  /** The exact action the agent asked to perform. */
  readonly requestedAction: string;
  /** The scope it would run in (path, command set, …). */
  readonly scope: string;
  readonly decision: PermissionDecision | null;
}

export interface TimelineRow {
  readonly id: string;
  readonly kind: RowKind;
  /** Message text, tool summary, or diagnostic. */
  readonly text: string;
  readonly status: ToolStatus | null;
  readonly permission: PermissionRequest | null;
  /** True while deltas are still appending to this row. */
  readonly streaming: boolean;
}

export interface TimelineSnapshot {
  readonly rows: readonly TimelineRow[];
  readonly structureVersion: number;
}

export interface TimelineStore {
  /** Snapshot for external-store consumers (structural changes only). */
  getSnapshot(): TimelineSnapshot;
  subscribe(listener: () => void): () => void;
  /** Per-row subscription used by memoized row components. */
  subscribeRow(id: string, listener: () => void): () => void;
  getRow(id: string): TimelineRow | null;
  getRowByIndex(index: number): TimelineRow | null;
  rowCount(): number;
  appendRow(row: TimelineRow): void;
  prependRows(rows: readonly TimelineRow[]): void;
  /** Appends delta text to a streaming row; notifies that row only. */
  appendDelta(id: string, text: string): void;
  finishStreaming(id: string): void;
  /** Records a provisional approval decision (ADR 0008 §5). */
  setPermissionDecision(id: string, decision: PermissionDecision): void;
  /** First unanswered permission row, for keyboard focus and a11y. */
  pendingPermission(): TimelineRow | null;
}

interface RowSlot {
  row: TimelineRow;
  listeners: Set<() => void>;
}

export function createTimelineStore(
  initial: readonly TimelineRow[] = [],
): TimelineStore {
  const slots: RowSlot[] = initial.map((row) => ({ row, listeners: new Set() }));
  const byId = new Map<string, RowSlot>();
  for (const slot of slots) byId.set(slot.row.id, slot);
  const structureListeners = new Set<() => void>();
  let structureVersion = 0;

  let snapshot: TimelineSnapshot = {
    rows: slots.map((slot) => slot.row),
    structureVersion,
  };
  const notifyStructure = () => {
    structureVersion += 1;
    snapshot = { rows: slots.map((slot) => slot.row), structureVersion };
    for (const listener of structureListeners) listener();
  };

  const slotOf = (id: string): RowSlot | undefined => byId.get(id);

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      structureListeners.add(listener);
      return () => structureListeners.delete(listener);
    },
    subscribeRow(id, listener) {
      const slot = slotOf(id);
      if (!slot) return () => undefined;
      slot.listeners.add(listener);
      return () => slot.listeners.delete(listener);
    },
    getRow: (id) => slotOf(id)?.row ?? null,
    getRowByIndex: (index) => slots[index]?.row ?? null,
    rowCount: () => slots.length,
    appendRow(row) {
      const slot: RowSlot = { row, listeners: new Set<() => void>() };
      slots.push(slot);
      byId.set(row.id, slot);
      notifyStructure();
    },
    prependRows(rows) {
      const newSlots = rows.map((row) => ({
        row,
        listeners: new Set<() => void>(),
      }));
      slots.unshift(...newSlots);
      for (const slot of newSlots) byId.set(slot.row.id, slot);
      notifyStructure();
    },
    appendDelta(id, text) {
      const slot = slotOf(id);
      if (!slot) throw new Error(`appendDelta for unknown row ${id}`);
      slot.row = { ...slot.row, text: slot.row.text + text };
      for (const listener of slot.listeners) listener();
    },
    finishStreaming(id) {
      const slot = slotOf(id);
      if (!slot) return;
      slot.row = { ...slot.row, streaming: false };
      for (const listener of slot.listeners) listener();
    },
    setPermissionDecision(id, decision) {
      const slot = slotOf(id);
      if (!slot?.row.permission) throw new Error(`row ${id} holds no permission request`);
      slot.row = {
        ...slot.row,
        permission: { ...slot.row.permission, decision },
      };
      for (const listener of slot.listeners) listener();
    },
    pendingPermission() {
      for (const slot of slots) {
        if (slot.row.kind === "permission" && slot.row.permission?.decision == null) {
          return slot.row;
        }
      }
      return null;
    },
  };
}
