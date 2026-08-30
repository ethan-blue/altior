/**
 * Memoized timeline row renderer (ADR 0008 §4).
 *
 * Each row subscribes only to itself, so a streaming delta re-renders
 * exactly one row no matter how large the timeline is. The acceptance
 * test pins this with render counters.
 */
import { memo, useCallback, useEffect, useSyncExternalStore } from "react";
import type {
  PermissionDecision,
  TimelineRow as Row,
  TimelineStore,
} from "./timelineStore";
import rowStyles from "./timeline.module.css";

export interface RowViewProps {
  readonly store: TimelineStore;
  readonly rowId: string;
  /** Row index — used for stable test ids and zebra-free striping. */
  readonly index: number;
  readonly focused: boolean;
  readonly onFocus: (id: string) => void;
  readonly onPermissionDecision: (id: string, decision: PermissionDecision) => void;
}

const kindLabel: Record<Row["kind"], string> = {
  "user-message": "You",
  "assistant-message": "Assistant",
  tool: "Tool",
  permission: "Approval",
  error: "Failed",
  unknown: "Unknown",
};

export const TimelineRowView = memo(function TimelineRowView({
  store,
  rowId,
  index,
  focused,
  onFocus,
  onPermissionDecision,
}: RowViewProps) {
  const subscribe = useCallback(
    (listener: () => void) => store.subscribeRow(rowId, listener),
    [store, rowId],
  );
  const getSnapshot = useCallback(() => store.getRow(rowId), [store, rowId]);
  const row = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  // A focused row that was recycled by the window keeps focus when it
  // remounts (ADR 0008 failure modes: no focus loss on row recycle).
  useEffect(() => {
    if (focused) {
      // Focus lives on the container's roving tabindex target.
      document.getElementById(rowDomId(rowId))?.focus({ preventScroll: true });
    }
  }, [focused, rowId]);

  if (!row) return null;

  const decide = (decision: PermissionDecision) => onPermissionDecision(rowId, decision);

  return (
    <div
      id={rowDomId(rowId)}
      data-row-id={rowId}
      data-row-kind={row.kind}
      data-testid={`timeline-row-${index}`}
      className={`${rowStyles.row} ${rowStyles[row.kind]} ${
        focused ? rowStyles.focused : ""
      }`}
      tabIndex={focused ? 0 : -1}
      role="article"
      aria-label={`${kindLabel[row.kind]} entry`}
      onMouseDown={() => onFocus(rowId)}
      onKeyDown={(event) => {
        if (row.kind !== "permission" || row.permission?.decision != null) return;
        if (event.key === "y") {
          event.preventDefault();
          decide("approved");
        } else if (event.key === "d" || event.key === "n") {
          event.preventDefault();
          decide("denied");
        }
      }}
    >
      <span className={rowStyles.kindLabel}>{kindLabel[row.kind]}</span>
      <div className={rowStyles.body}>
        {row.kind === "permission" && row.permission ? (
          <PermissionBody
            row={row}
            onApprove={() => decide("approved")}
            onDeny={() => decide("denied")}
          />
        ) : (
          <>
            <span className={rowStyles.text}>
              {row.text}
              {row.streaming ? <span className={rowStyles.caret} aria-hidden="true">▌</span> : null}
            </span>
            {row.kind === "tool" && row.status ? (
              <span className={`${rowStyles.toolStatus} ${rowStyles[row.status]}`}>
                {row.status}
              </span>
            ) : null}
          </>
        )}
      </div>
    </div>
  );
});

function PermissionBody({
  row,
  onApprove,
  onDeny,
}: {
  readonly row: Row;
  readonly onApprove: () => void;
  readonly onDeny: () => void;
}) {
  const permission = row.permission!;
  if (permission.decision != null) {
    return (
      <div className={rowStyles.permissionDecided}>
        <span className={rowStyles.mono}>{permission.requestedAction}</span>
        <span className={rowStyles.scope}>{permission.scope}</span>
        <span
          className={`${rowStyles.decisionChip} ${
            permission.decision === "approved" ? rowStyles.approved : rowStyles.denied
          }`}
        >
          {permission.decision}
        </span>
      </div>
    );
  }
  return (
    <div className={rowStyles.permissionAsk}>
      <div className={rowStyles.permissionAction}>
        <span className={rowStyles.mono}>{permission.requestedAction}</span>
        <span className={rowStyles.scope}>{permission.scope}</span>
      </div>
      <div className={rowStyles.permissionControls}>
        <button type="button" onClick={onApprove} data-testid="approve">
          Approve (Y)
        </button>
        <button type="button" onClick={onDeny} data-testid="deny">
          Deny (D)
        </button>
      </div>
    </div>
  );
}

export function rowDomId(rowId: string): string {
  return `timeline-row-dom-${rowId}`;
}
