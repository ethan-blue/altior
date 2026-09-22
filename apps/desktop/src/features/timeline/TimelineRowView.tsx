/**
 * Commercial UI Phase 1: Memoized Timeline Row Card View (ADR 0008 §4).
 *
 * Each row subscribes only to its own model, ensuring high-frequency
 * streaming deltas re-render only the affected turn.
 * Implements centered, scannable card layout with distinct visual
 * hierarchies for user, assistant, tool, error, and high-trust permission
 * security approval cards.
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
  /** Row index — used for stable test ids and roving index mapping. */
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

  // Focus lives on the container's roving tabindex target;
  // recycled rows retain keyboard focus on remount.
  useEffect(() => {
    if (focused) {
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
        if (event.key === "y" || event.key === "Y") {
          event.preventDefault();
          decide("approved");
        } else if (
          event.key === "d" ||
          event.key === "D" ||
          event.key === "n" ||
          event.key === "N"
        ) {
          event.preventDefault();
          decide("denied");
        }
      }}
    >
      <div className={rowStyles.card}>
        <div className={rowStyles.cardHeader}>
          <div className={rowStyles.roleBadge}>
            <span className={rowStyles.roleIcon}>
              <RoleIcon kind={row.kind} />
            </span>
            <span className={rowStyles.kindLabel}>{getKindLabel(row.kind)}</span>
          </div>

          <div className={rowStyles.headerMeta}>
            {row.kind === "permission" ? (
              <span className={rowStyles.riskBadge}>
                <ShieldAlertIcon />
                <span>{t.permission.securityGate}</span>
              </span>
            ) : row.kind === "error" ? (
              <span className={rowStyles.errorTag}>
                {t.timeline.executionError}
              </span>
            ) : row.kind === "unknown" ? (
              <span className={rowStyles.unknownTag}>
                {t.timeline.preservedVerbatim}
              </span>
            ) : row.streaming ? (
              <span className={rowStyles.streamingBadge}>
                <span className={rowStyles.streamingDot} aria-hidden="true" />
                <span>{t.timeline.streaming}</span>
              </span>
            ) : null}
          </div>
        </div>

        <div className={rowStyles.body}>
          {row.kind === "permission" && row.permission ? (
            <PermissionBody
              row={row}
              onApprove={() => decide("approved")}
              onDeny={() => decide("denied")}
            />
          ) : row.kind === "tool" ? (
            <ToolBlock text={row.text} status={row.status} />
          ) : row.kind === "unknown" ? (
            <div className={rowStyles.unknownBody}>{row.text}</div>
          ) : (
            <div className={rowStyles.text}>
              <SafeMarkdown text={row.text} />
              {row.streaming ? (
                <span className={rowStyles.caret} aria-hidden="true">
                  ▌
                </span>
              ) : null}
            </div>
          )}
        </div>
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
  const submitting = permission.submission === "submitting";

  return (
    <div className={rowStyles.permissionCardContent}>
      {/* Risk Awareness Banner */}
      <div className={rowStyles.riskNotice}>
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
          className={rowStyles.riskNoticeIcon}
        >
          <path
            d="M8 1.5L1 14H15L8 1.5Z"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
          <path
            d="M8 6V9"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
          <circle cx="8" cy="11.5" r="0.75" fill="currentColor" />
        </svg>
        <span>{t.permission.riskNotice}</span>
      </div>

      {/* Context: Requested Action & Target Scope */}
      <div className={rowStyles.permissionDetails}>
        <div className={rowStyles.permissionField}>
          <span className={rowStyles.fieldLabel}>
            {t.permission.requestedAction}
          </span>
          <div className={rowStyles.codeBox}>
            <code className={rowStyles.mono}>{permission.requestedAction}</code>
          </div>
        </div>

        <div className={rowStyles.permissionField}>
          <span className={rowStyles.fieldLabel}>
            {t.permission.scope}
          </span>
          <div className={rowStyles.scopeBox}>
            <svg
              width="12"
              height="12"
              viewBox="0 0 16 16"
              fill="none"
              aria-hidden="true"
              className={rowStyles.scopeIcon}
            >
              <path
                d="M1.5 4C1.5 3.17 2.17 2.5 3 2.5H6.5L8 4.5H13C13.83 4.5 14.5 5.17 14.5 6V12C14.5 12.83 13.83 13.5 13 13.5H3C2.17 13.5 1.5 12.83 1.5 12V4Z"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinejoin="round"
              />
            </svg>
            <span className={rowStyles.scope}>{permission.scope}</span>
          </div>
        </div>
      </div>

      {/* Interactive Controls or Decided Record */}
      {permission.decision != null ? (
        <div className={rowStyles.permissionDecided}>
          <div className={rowStyles.decisionBanner}>
            <span
              className={`${rowStyles.decisionChip} ${
                permission.decision === "approved" ? rowStyles.approved : rowStyles.denied
              }`}
            >
              {permission.decision === "approved" ? (
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 16 16"
                  fill="none"
                  aria-hidden="true"
                >
                  <path
                    d="M3 8.5L6.5 12L13 4"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              ) : (
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 16 16"
                  fill="none"
                  aria-hidden="true"
                >
                  <path
                    d="M4 4L12 12M12 4L4 12"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                  />
                </svg>
              )}
              <span>{permission.decision}</span>
            </span>
            <span className={rowStyles.decisionAuditNote}>
              {permission.decision === "approved"
                ? t.permission.authorizedByUser
                : t.permission.deniedByUser}
            </span>
          </div>
        </div>
      ) : (
        <div className={rowStyles.permissionAsk}>
          <div className={rowStyles.permissionControls}>
            <div className={rowStyles.buttonGroup}>
              <button
                type="button"
                className={rowStyles.approveButton}
                onClick={onApprove}
                disabled={submitting}
                data-testid="approve"
                title={t.permission.approveTitle}
              >
                <span className={rowStyles.btnContent}>
                  <svg
                    width="13"
                    height="13"
                    viewBox="0 0 16 16"
                    fill="none"
                    aria-hidden="true"
                  >
                    <path
                      d="M3 8.5L6.5 12L13 4"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                  <span>{submitting ? t.permission.recording : t.permission.approve}</span>
                </span>
                {!submitting && <kbd className={rowStyles.kbd}>Y</kbd>}
              </button>

              <button
                type="button"
                className={rowStyles.denyButton}
                onClick={onDeny}
                disabled={submitting}
                data-testid="deny"
                title={t.permission.denyTitle}
              >
                <span className={rowStyles.btnContent}>
                  <svg
                    width="13"
                    height="13"
                    viewBox="0 0 16 16"
                    fill="none"
                    aria-hidden="true"
                  >
                    <path
                      d="M4 4L12 12M12 4L4 12"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                    />
                  </svg>
                  <span>{submitting ? t.permission.recording : t.permission.deny}</span>
                </span>
                {!submitting && <kbd className={rowStyles.kbd}>D</kbd>}
              </button>
            </div>

            <div className={rowStyles.shortcutHint}>
              <span className={rowStyles.shortcutText}>
                {t.permission.shortcutHint}
              </span>
            </div>

            {permission.submission === "failed" ? (
              <span role="alert" className={rowStyles.submissionError}>
                {t.permission.failed}
              </span>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}

function RoleIcon({ kind }: { readonly kind: Row["kind"] }) {
  switch (kind) {
    case "user-message":
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <circle cx="8" cy="5" r="3" stroke="currentColor" strokeWidth="1.5" />
          <path
            d="M2.5 14C2.5 11.2 5 9.5 8 9.5C11 9.5 13.5 11.2 13.5 14"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </svg>
      );
    case "assistant-message":
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path
            d="M8 1.5L9.5 6L14 8L9.5 10L8 14.5L6.5 10L2 8L6.5 6L8 1.5Z"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
        </svg>
      );
    case "tool":
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path d="M2 13L7 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          <path
            d="M6 3.5C6.5 2.5 8 2 9.5 2C11 2 12.5 2.8 13.2 4.2C13.6 5 13.5 6 12.8 6.6L10.5 8.5L7.5 5.5L9 3.5"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
        </svg>
      );
    case "permission":
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path
            d="M8 1.5L2.5 3.8V7.5C2.5 11 5 13.8 8 14.5C11 13.8 13.5 11 13.5 7.5V3.8L8 1.5Z"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinejoin="round"
          />
          <path d="M8 5V8.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          <circle cx="8" cy="11" r="0.75" fill="currentColor" />
        </svg>
      );
    case "error":
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" strokeWidth="1.5" />
          <path d="M8 5V8.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
          <circle cx="8" cy="11" r="0.75" fill="currentColor" />
        </svg>
      );
    default:
      return (
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <rect
            x="2.5"
            y="2.5"
            width="11"
            height="11"
            rx="2"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeDasharray="2 2"
          />
          <path
            d="M6.5 6.5C6.5 5.5 7.2 4.8 8 4.8C8.8 4.8 9.5 5.5 9.5 6.5C9.5 7.5 8 8 8 9"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
          <circle cx="8" cy="11.2" r="0.75" fill="currentColor" />
        </svg>
      );
  }
}

function ShieldAlertIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path
        d="M8 1.5L2.5 3.8V7.5C2.5 11 5 13.8 8 14.5C11 13.8 13.5 11 13.5 7.5V3.8L8 1.5Z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path d="M8 5V8.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="8" cy="11" r="0.75" fill="currentColor" />
    </svg>
  );
}

export function rowDomId(rowId: string): string {
  return `timeline-row-dom-${rowId}`;
}
