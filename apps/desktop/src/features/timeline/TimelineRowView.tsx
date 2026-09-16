/**
 * Memoized timeline row renderer (ADR 0008 §4).
 *
 * Each row subscribes only to itself, so a streaming delta re-renders
 * exactly one row no matter how large the timeline is. The acceptance
 * test pins this with render counters.
 */
import { memo, useCallback, useEffect, useSyncExternalStore } from "react";
import { useI18n } from "../../i18n";
import { SafeMarkdown, ToolBlock } from "../../components/SafeMarkdown";
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

export const TimelineRowView = memo(function TimelineRowView({
  store,
  rowId,
  index,
  focused,
  onFocus,
  onPermissionDecision,
}: RowViewProps) {
  const { t } = useI18n();
  const getKindLabel = (kind: Row["kind"]): string => {
    switch (kind) {
      case "user-message":
        return t.timeline.you;
      case "assistant-message":
        return t.timeline.assistant;
      case "tool":
        return t.timeline.tool;
      case "permission":
        return t.timeline.approval;
      case "error":
        return t.timeline.failed;
      default:
        return t.timeline.unknown;
    }
  };

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
      aria-label={t.timeline.entryAria(getKindLabel(row.kind))}
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
      <span className={rowStyles.kindLabel}>{getKindLabel(row.kind)}</span>
      <div className={rowStyles.body}>
        {row.kind === "permission" && row.permission ? (
          <PermissionBody
            row={row}
            onApprove={() => decide("approved")}
            onDeny={() => decide("denied")}
          />
        ) : row.kind === "tool" ? (
          <ToolBlock text={row.text} status={row.status} />
        ) : (
          <span className={rowStyles.text}>
            <SafeMarkdown text={row.text} />
            {row.streaming ? <span className={rowStyles.caret} aria-hidden="true">▌</span> : null}
          </span>
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
  const { t } = useI18n();
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
  const submitting = permission.submission === "submitting";
  return (
    <div className={rowStyles.permissionAsk}>
      <div className={rowStyles.permissionAction}>
        <span className={rowStyles.mono}>{permission.requestedAction}</span>
        <span className={rowStyles.scope}>{permission.scope}</span>
      </div>
      <div className={rowStyles.permissionControls}>
        <button
          type="button"
          className={rowStyles.approveButton}
          onClick={onApprove}
          disabled={submitting}
          data-testid="approve"
        >
          {submitting ? t.permission.recording : t.permission.approve}
        </button>
        <button
          type="button"
          className={rowStyles.denyButton}
          onClick={onDeny}
          disabled={submitting}
          data-testid="deny"
        >
          {submitting ? t.permission.recording : t.permission.deny}
        </button>
        {permission.submission === "failed" ? (
          <span role="alert">{t.permission.failed}</span>
        ) : null}
      </div>
    </div>
  );
}

export function rowDomId(rowId: string): string {
  return `timeline-row-dom-${rowId}`;
}



