/**
 * Journal-to-Timeline reducer (ADR 0020, review A05).
 *
 * Normalizes journal-projected `HistoryEntryDto` records into `TimelineRow`
 * models for the virtualized timeline. Live stream events and journal
 * history pages produce the same row structures, ensuring consistent
 * rendering and interaction across app restarts and pagination.
 */
import type { HistoryEntryDto } from "../../ipc/dto/HistoryEntryDto";
import type { TimelineRow } from "./timelineStore";

/**
 * Reduces an ordered list of journal history entries (oldest-first) into
 * displayable timeline rows.
 *
 * - `user_message`: produces a `"user-message"` row with the user prompt.
 * - `assistant_delta`: coalesces deltas within the same turn into a single
 *   `"assistant-message"` row preserving text continuity.
 * - `permission`: produces a `"permission"` row holding the action and scope.
 * - `permission_decision`: updates the corresponding permission request row
 *   with the authoritative decision (`"approved"` or `"denied"`).
 * - `turn_state`: finalizes assistant streaming state or records turn failures.
 * - `unknown`: preserves forward-compatible/unknown provider facts as inspectable
 *   `"unknown"` rows instead of dropping them or failing the page.
 */
export function reduceHistoryEntries(
  entries: readonly HistoryEntryDto[],
): TimelineRow[] {
  const rows: TimelineRow[] = [];
  const assistantRowsByTurn = new Map<string, { index: number; row: TimelineRow }>();
  const permissionRowsById = new Map<string, { index: number; row: TimelineRow }>();

  for (const entry of entries) {
    switch (entry.entry_kind) {
      case "user_message": {
        const row: TimelineRow = {
          id: entry.event_id,
          kind: "user-message",
          text: entry.text,
          status: null,
          permission: null,
          streaming: false,
        };
        rows.push(row);
        break;
      }

      case "assistant_delta": {
        const existing = assistantRowsByTurn.get(entry.turn_id);
        if (existing) {
          const updatedRow: TimelineRow = {
            ...existing.row,
            text: existing.row.text + entry.text,
          };
          rows[existing.index] = updatedRow;
          assistantRowsByTurn.set(entry.turn_id, {
            index: existing.index,
            row: updatedRow,
          });
        } else {
          // Stable row identity derived from the turn identifier
          const rowId = entry.turn_id;
          const newRow: TimelineRow = {
            id: rowId,
            kind: "assistant-message",
            text: entry.text,
            status: null,
            permission: null,
            streaming: false,
          };
          const index = rows.length;
          rows.push(newRow);
          assistantRowsByTurn.set(entry.turn_id, { index, row: newRow });
        }
        break;
      }

      case "permission": {
        const row: TimelineRow = {
          id: entry.event_id,
          kind: "permission",
          text: entry.description,
          status: null,
          permission: {
            requestedAction: entry.description,
            scope: entry.permission_kind,
            decision:
              entry.decision === "approved" || entry.decision === "denied"
                ? entry.decision
                : null,
            submission: null,
          },
          streaming: false,
        };
        const index = rows.length;
        rows.push(row);
        permissionRowsById.set(entry.event_id, { index, row });
        break;
      }

      case "permission_decision": {
        const targetId = entry.permission_event_id;
        const existing = permissionRowsById.get(targetId);
        if (existing && existing.row.permission) {
          const updatedRow: TimelineRow = {
            ...existing.row,
            permission: {
              ...existing.row.permission,
              decision:
                entry.decision === "approved" || entry.decision === "denied"
                  ? entry.decision
                  : null,
              submission: null,
            },
          };
          rows[existing.index] = updatedRow;
          permissionRowsById.set(targetId, {
            index: existing.index,
            row: updatedRow,
          });
        }
        break;
      }

      case "turn_state": {
        const existing = assistantRowsByTurn.get(entry.turn_id);
        if (existing && existing.row.streaming) {
          const updatedRow: TimelineRow = {
            ...existing.row,
            streaming: false,
          };
          rows[existing.index] = updatedRow;
          assistantRowsByTurn.set(entry.turn_id, {
            index: existing.index,
            row: updatedRow,
          });
        }
        if (entry.state === "failed") {
          rows.push({
            id: `fail-${entry.event_id}`,
            kind: "error",
            text: entry.reason ?? "Turn failed",
            status: null,
            permission: null,
            streaming: false,
          });
        }
        break;
      }

      case "unknown": {
        rows.push({
          id: entry.event_id,
          kind: "unknown",
          text: `${entry.kind}: ${entry.diagnostic}`,
          status: null,
          permission: null,
          streaming: false,
        });
        break;
      }
    }
  }

  return rows;
}
