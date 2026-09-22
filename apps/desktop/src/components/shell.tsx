/**
 * Workbench shell regions (ADR 0008 §2): activity rail, navigation pane,
 * thread header, composer, inspector, status bar. Five stable regions
 * per docs/UI_ARCHITECTURE.md; panes resize by drag or keyboard within
 * their token clamps.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { TimelineRow } from "../features/timeline/timelineStore";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import type { RuntimeDiagnosticsDto } from "../ipc/dto/RuntimeDiagnosticsDto";
import {
  parseCommandLineArgs,
  parseStringList,
  type AgentProfile,
  type EnvSecretMapping,
  type TestAgentResult,
  type ThreadStatus,
  type ThreadSummaryView,
} from "../stores/applicationStore";
import {
  INSPECTOR_MAX,
  INSPECTOR_MIN,
  NAV_MAX,
  NAV_MIN,
} from "../app/uiStore";
import { ContextPanel, type ContextPanelProps } from "./ContextPanel";
import { useI18n } from "../i18n";
import type { ThemeSource, LocaleSource } from "../app/uiStore";
import shell from "./shell.module.css";

export { ContextPanel, type ContextPanelProps };
export { MemoryPane, type MemoryPaneProps } from "./MemoryPane";

const statusLabel: Record<ThreadStatus, string> = {
  running: "running",
  "waiting-for-permission": "waiting",
  failed: "failed",
  completed: "completed",
};

const statusGlyph: Record<ThreadStatus, string> = {
  running: "◐",
  "waiting-for-permission": "?",
  failed: "×",
  completed: "✓",
};

export interface ActivityRailProps {
  readonly active: string;
  readonly onNavigate?: (destination: string) => void;
}

function RailIcon({ id }: { readonly id: string }) {
  switch (id) {
    case "threads":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      );
    case "agents":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <rect x="3" y="11" width="18" height="10" rx="2" />
          <circle cx="12" cy="5" r="2" />
          <path d="M12 7v4" />
          <line x1="8" y1="16" x2="8.01" y2="16" />
          <line x1="16" y1="16" x2="16.01" y2="16" />
        </svg>
      );
    case "projects":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
      );
    case "memory":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="M7 8h10M7 12h10M7 16h6" />
        </svg>
      );
    case "devices":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <rect x="2" y="3" width="20" height="14" rx="2" />
          <line x1="8" y1="21" x2="16" y2="21" />
          <line x1="12" y1="17" x2="12" y2="21" />
        </svg>
      );
    case "settings":
      return (
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={shell.railIcon}>
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      );
    default:
      return null;
  }
}

/** Activity rail: Threads and Agents; others arrive with subsequent phases. */
export function ActivityRail({ active, onNavigate }: ActivityRailProps) {
  const { t } = useI18n();
  const destinations: { id: string; label: string; arrives: string | null }[] = [
    { id: "threads", label: t.rail.threads, arrives: null },
    { id: "agents", label: t.rail.agents, arrives: null },
    { id: "projects", label: t.rail.projects, arrives: "P4" },
    { id: "memory", label: t.rail.memory, arrives: null },
    { id: "devices", label: t.rail.devices, arrives: "P3" },
    { id: "settings", label: t.rail.settings, arrives: null },
  ];

  const topItems = destinations.slice(0, 5);
  const bottomItems = destinations.slice(5);

  const renderItem = ({
    id,
    label,
    arrives,
  }: {
    id: string;
    label: string;
    arrives: string | null;
  }) => {
    const enabled = arrives == null;
    return (
      <button
        key={id}
        type="button"
        className={`${shell.railItem} ${active === id ? shell.railActive : ""}`}
        aria-current={active === id ? "page" : undefined}
        aria-disabled={!enabled}
        aria-label={label}
        disabled={!enabled}
        onClick={() => enabled && onNavigate?.(id)}
        title={enabled ? label : `${label} — ${t.rail.arrivesWith(arrives!)}`}
        data-testid={`rail-${id}`}
      >
        <RailIcon id={id} />
        <span className={shell.railLabel}>{label}</span>
      </button>
    );
  };

  return (
    <nav className={shell.rail} aria-label={t.rail.activityAria}>
      <div className={shell.railGroup}>
        {topItems.map(renderItem)}
      </div>
      <div className={shell.railBottomGroup}>
        {bottomItems.map(renderItem)}
      </div>
    </nav>
  );
}

export interface ThreadsPaneProps {
  readonly threads: readonly ThreadSummaryView[];
  readonly selectedThreadId: string;
  readonly onSelect: (id: string) => void;
  readonly filter: string;
  readonly onFilterChange: (value: string) => void;
  readonly onCreateThread?: () => void;
  /** Authoritative list has more pages (hidden while searching). */
  readonly hasMoreThreads?: boolean;
  readonly onLoadMore?: () => void;
  /** True while `threads` holds search results instead of the list. */
  readonly searchActive?: boolean;
  /** Debounce delay in milliseconds (default: 280ms). Set to 0 in tests for immediate dispatch. */
  readonly debounceMs?: number;
}

/** Navigation pane: pinned and recent threads with search and thread creation. */
export function ThreadsPane({
  threads,
  selectedThreadId,
  onSelect,
  filter,
  onFilterChange,
  onCreateThread,
  hasMoreThreads = false,
  onLoadMore,
  searchActive = false,
  debounceMs = 280,
}: ThreadsPaneProps) {
  const { t } = useI18n();
  const [localFilter, setLocalFilter] = useState(filter);
  const isComposingRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setLocalFilter(filter);
  }, [filter]);

  useEffect(() => {
    return () => {
      if (timerRef.current != null) {
        clearTimeout(timerRef.current);
      }
    };
  }, []);

  const dispatchDebounced = useCallback(
    (value: string) => {
      if (timerRef.current != null) {
        clearTimeout(timerRef.current);
      }
      if (debounceMs <= 0) {
        onFilterChange(value);
      } else {
        timerRef.current = setTimeout(() => {
          onFilterChange(value);
        }, debounceMs);
      }
    },
    [debounceMs, onFilterChange],
  );

  const handleInputChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const nextValue = event.target.value;
    setLocalFilter(nextValue);
    if (!isComposingRef.current) {
      dispatchDebounced(nextValue);
    }
  };

  const handleCompositionStart = () => {
    isComposingRef.current = true;
  };

  const handleCompositionEnd = (event: React.CompositionEvent<HTMLInputElement>) => {
    isComposingRef.current = false;
    dispatchDebounced(event.currentTarget.value);
  };

  // The visible list is already the authority — the Core list/search
  // responses. No client-side second filtering that could drop legitimate
  // backend hits (review A03, F06).
  const pinned = threads.filter((thread) => thread.pinned);
  const recent = threads.filter((thread) => !thread.pinned);

  return (
    <section className={shell.threadsPane} aria-label={t.nav.paneAria}>
      <div className={shell.threadsHeader}>
        <div className={shell.searchWrapper}>
          <svg
            className={shell.searchIcon}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            type="search"
            className={shell.search}
            placeholder={t.nav.filterPlaceholder}
            value={localFilter}
            onChange={handleInputChange}
            onCompositionStart={handleCompositionStart}
            onCompositionEnd={handleCompositionEnd}
            aria-label={t.nav.filterPlaceholder}
            data-testid="thread-filter"
          />
        </div>
        {onCreateThread ? (
          <button
            type="button"
            className={shell.newThreadBtn}
            onClick={onCreateThread}
            data-testid="new-thread"
            title={t.inspector.createThreadTitle}
          >
            {t.nav.newThread}
          </button>
        ) : null}
      </div>
      {threads.length === 0 ? (
        <p className={shell.threadsEmpty} role="status">
          {searchActive ? t.nav.noMatches : t.nav.empty}
        </p>
      ) : null}
      {pinned.length > 0 ? (
        <ThreadSection
          title={t.nav.pinned}
          threads={pinned}
          selectedThreadId={selectedThreadId}
          onSelect={onSelect}
        />
      ) : null}
      <ThreadSection
        title={t.nav.recent}
        threads={recent}
        selectedThreadId={selectedThreadId}
        onSelect={onSelect}
      />
      {hasMoreThreads && onLoadMore ? (
        <button
          type="button"
          className={shell.loadMoreBtn}
          onClick={onLoadMore}
          data-testid="load-more-threads"
        >
          {t.nav.loadMore}
        </button>
      ) : null}
    </section>
  );
}

function ThreadSection({
  title,
  threads,
  selectedThreadId,
  onSelect,
}: {
  readonly title: string;
  readonly threads: readonly ThreadSummaryView[];
  readonly selectedThreadId: string;
  readonly onSelect: (id: string) => void;
}) {
  return (
    <section className={shell.threadSection} aria-label={title}>
      <h2 className={shell.sectionTitle}>{title}</h2>
      <div className={shell.threadList}>
        {threads.map((thread) => (
          <button
            key={thread.id}
            type="button"
            className={`${shell.threadRow} ${
              thread.id === selectedThreadId ? shell.threadSelected : ""
            }`}
            aria-current={thread.id === selectedThreadId ? "true" : undefined}
            onClick={() => onSelect(thread.id)}
            data-testid={`thread-${thread.id}`}
          >
            <span aria-hidden="true" data-status={thread.status} className={shell.statusGlyph}>
              {statusGlyph[thread.status]}
            </span>
            <span className={shell.threadTitle}>{thread.title}</span>
            <span className={shell.threadStatus}>{statusLabel[thread.status]}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

export interface ThreadHeaderProps {
  readonly title: string;
  readonly agent: string;
  readonly agents?: readonly AgentProfile[];
  readonly selectedAgentId?: string;
  readonly onSelectAgent?: (agentId: string) => void;
  readonly onAddAgent?: () => void;
  readonly theme: "light" | "dark";
  readonly onToggleTheme: () => void;
  readonly inspectorOpen: boolean;
  readonly onToggleInspector: () => void;
  readonly isStreaming?: boolean;
  readonly onCancel?: () => void;
}

export function ThreadHeader({
  title,
  agent,
  agents,
  selectedAgentId,
  onSelectAgent,
  onAddAgent,
  theme,
  onToggleTheme,
  inspectorOpen,
  onToggleInspector,
  isStreaming,
  onCancel,
}: ThreadHeaderProps) {
  const { t } = useI18n();
  return (
    <header className={shell.threadHeader}>
      <div className={shell.headerTitleGroup}>
        <h1 className={shell.threadTitleMain}>{title}</h1>

        {agents && onSelectAgent ? (
          <div className={shell.agentSelectorWrapper}>
            <select
              className={shell.agentSelect}
              value={
                selectedAgentId ??
                agents.find((a) => a.name === agent || a.id === agent)?.id ??
                agents[0]?.id
              }
              onChange={(e) => onSelectAgent(e.target.value)}
              aria-label={t.workbench.selectAgent}
              data-testid="agent-selector"
            >
              {agents.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name} ({a.model})
                </option>
              ))}
            </select>
            {onAddAgent ? (
              <button
                type="button"
                className={shell.reconnectBtn}
                onClick={onAddAgent}
                data-testid="add-agent-btn"
                title={t.workbench.addAgent}
              >
                {t.workbench.addAgent}
              </button>
            ) : null}
          </div>
        ) : (
          <span className={shell.threadAgent}>{agent}</span>
        )}
      </div>

      <div className={shell.headerControls}>
        {isStreaming && onCancel ? (
          <button
            type="button"
            onClick={onCancel}
            className={shell.cancelBtn}
            data-testid="header-cancel-turn"
          >
            {t.workbench.cancelTurn}
          </button>
        ) : null}
        <button type="button" onClick={onToggleTheme} data-testid="theme-toggle">
          {t.workbench.themeToggle(theme)}
        </button>
        <button type="button" onClick={onToggleInspector} data-testid="inspector-toggle">
          {inspectorOpen ? t.workbench.hideInspector : t.workbench.showInspector}
        </button>
      </div>
    </header>
  );
}

export interface ComposerProps {
  readonly draft: string;
  readonly onDraftChange: (text: string) => void;
  readonly onSend: () => void;
  readonly onCancel?: () => void;
  readonly isStreaming?: boolean;
  /** True while a cancel_turn is in flight (button reads "Cancelling…"). */
  readonly cancelPending?: boolean;
  readonly disabledReason: string | null;
}

/** Composer: one draft per thread; Enter sends, Shift+Enter breaks lines. */
export function Composer({
  draft,
  onDraftChange,
  onSend,
  onCancel,
  isStreaming,
  cancelPending = false,
  disabledReason,
}: ComposerProps) {
  const { t } = useI18n();
  const [isComposing, setIsComposing] = useState(false);
  const [isFocused, setIsFocused] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const handleCardClick = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button, select, a")) return;
    textareaRef.current?.focus();
  };

  return (
    <div className={shell.composer}>
      <div
        className={`${shell.composerCard} ${isFocused ? shell.composerCardFocused : ""} ${
          disabledReason != null ? shell.composerCardDisabled : ""
        }`}
        onClick={handleCardClick}
      >
        <textarea
          ref={textareaRef}
          className={shell.composerInput}
          placeholder={disabledReason ?? t.composer.placeholder}
          value={draft}
          disabled={disabledReason != null}
          onFocus={() => setIsFocused(true)}
          onBlur={() => setIsFocused(false)}
          onChange={(event) => onDraftChange(event.target.value)}
          onCompositionStart={() => setIsComposing(true)}
          onCompositionEnd={() => setIsComposing(false)}
          onKeyDown={(event) => {
            // Chinese IME / composition protection (A09 / F16):
            // Never send on Enter when confirming IME candidates (isComposing or Windows keyCode 229)
            if (
              isComposing ||
              event.nativeEvent.isComposing ||
              event.keyCode === 229
            ) {
              return;
            }
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              if (disabledReason == null) onSend();
            }
          }}
          aria-label={t.composer.ariaLabel}
          data-testid="composer"
          rows={2}
        />

        <div className={shell.composerToolbar}>
          <div className={shell.composerToolsLeft}>
            <span className={shell.composerChip}>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <circle cx="12" cy="12" r="10" />
                <polygon points="12 8 8 12 12 16 12 8" />
              </svg>
              <span>{t.composer.smartTools}</span>
            </span>

            <div className={shell.composerShortcuts}>
              <kbd className={shell.composerKbd}>{t.composer.shortcutHintSend}</kbd>
              <span className={shell.composerKbdDivider}>•</span>
              <kbd className={shell.composerKbd}>{t.composer.shortcutHintNewline}</kbd>
            </div>
          </div>

          <div className={shell.composerToolsRight}>
            {isStreaming && onCancel ? (
              <button
                type="button"
                className={shell.cancelBtn}
                onClick={onCancel}
                disabled={cancelPending}
                data-testid="cancel-turn"
              >
                <span className={shell.cancelIndicator} aria-hidden="true">■</span>
                {cancelPending ? t.composer.requestingStop : t.composer.stop}
              </button>
            ) : null}
            <button
              type="button"
              className={shell.send}
              onClick={onSend}
              disabled={disabledReason != null || draft.trim().length === 0}
              data-testid="send"
            >
              <span>{t.composer.send}</span>
              <svg
                width="13"
                height="13"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <line x1="22" y1="2" x2="11" y2="13" />
                <polygon points="22 2 15 22 11 13 2 9 22 2" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export interface RuntimeDiagnosticsViewProps {
  readonly diagnostics?: RuntimeDiagnosticsDto | null;
  readonly status?: "idle" | "loading" | "loaded" | "error";
  readonly error?: string | null;
  readonly onRefresh?: () => void;
}

export function RuntimeDiagnosticsView({
  diagnostics,
  status = "idle",
  error,
  onRefresh,
}: RuntimeDiagnosticsViewProps) {
  const { t } = useI18n();

  if (status === "loading") {
    return (
      <div data-testid="diagnostics-loading">
        <p className={shell.inspectorEmpty} role="status">
          {t.inspector.loadingDiagnostics}
        </p>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div data-testid="diagnostics-error">
        <p className={shell.inspectorEmpty} role="alert" style={{ color: "var(--color-danger, #b3362b)" }}>
          {error || t.inspector.diagnosticsFailed}
        </p>
      </div>
    );
  }

  if (!diagnostics) {
    return (
      <div data-testid="diagnostics-empty">
        <p className={shell.inspectorEmpty} role="status">
          {t.inspector.noDiagnostics}
        </p>
        {onRefresh ? (
          <button type="button" className={shell.btnAction} onClick={onRefresh} style={{ display: "block", margin: "8px auto" }}>
            {t.common.refresh}
          </button>
        ) : null}
      </div>
    );
  }

  return (
    <div data-testid="diagnostics-panel">
      <dl className={shell.inspectorFields}>
        <dt>{t.inspector.instanceId}</dt>
        <dd className={shell.mono} data-testid="diag-instance-id">{diagnostics.instance_id}</dd>

        <dt>{t.inspector.status}</dt>
        <dd data-testid="diag-status">
          <span className={shell.memoryKindBadge}>{diagnostics.status}</span>
        </dd>

        <dt>{t.inspector.activeThreads}</dt>
        <dd className={shell.mono} data-testid="diag-active-threads">{diagnostics.active_threads}</dd>

        <dt>{t.inspector.activeTurns}</dt>
        <dd className={shell.mono} data-testid="diag-active-turns">{diagnostics.active_turns}</dd>

        <dt>{t.inspector.diagnosticsSummary}</dt>
        <dd className={shell.mono} data-testid="diag-summary">{diagnostics.summary ?? t.inspector.none}</dd>
      </dl>
      <p style={{ fontSize: "0.75rem", color: "var(--color-muted)", marginTop: "12px", borderTop: "1px solid var(--color-border)", paddingTop: "8px" }} data-testid="diag-redacted-notice">
        {t.inspector.redactedNotice}
      </p>
      {onRefresh ? (
        <button type="button" className={shell.btnAction} onClick={onRefresh} style={{ marginTop: "8px" }} data-testid="diag-refresh-btn">
          {t.common.refresh}
        </button>
      ) : null}
    </div>
  );
}

export interface InspectorProps {
  readonly width: number;
  readonly onWidthChange: (width: number) => void;
  readonly onClose: () => void;
  readonly focusedRow: TimelineRow | null;
  readonly activeAgent?: AgentProfile | null;
  readonly contextSnapshot?: ContextSnapshotDto | null;
  readonly contextSnapshotStatus?: "idle" | "loading" | "loaded" | "not_found" | "error";
  readonly contextSnapshotError?: string | null;
  readonly diagnostics?: RuntimeDiagnosticsDto | null;
  readonly diagnosticsStatus?: "idle" | "loading" | "loaded" | "error";
  readonly diagnosticsError?: string | null;
  readonly onRefreshDiagnostics?: () => void;
  readonly initialTab?: "details" | "context" | "diagnostics";
}

/**
 * Inspector: contextual pane for turn details, tool output, provenance,
 * and context assembly snapshots (ADR 0018).
 */
export function Inspector({
  width,
  onWidthChange,
  onClose,
  focusedRow,
  activeAgent,
  contextSnapshot,
  contextSnapshotStatus,
  contextSnapshotError,
  diagnostics,
  diagnosticsStatus,
  diagnosticsError,
  onRefreshDiagnostics,
  initialTab = "details",
}: InspectorProps) {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<"details" | "context" | "diagnostics">(initialTab);
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    dragState.current = { startX: event.clientX, startWidth: width };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const state = dragState.current;
    if (!state) return;
    onWidthChange(state.startWidth - (event.clientX - state.startX));
  };
  const onPointerUp = () => {
    dragState.current = null;
  };
  const step = useCallback(
    (delta: number) => onWidthChange(width + delta),
    [onWidthChange, width],
  );

  return (
    <aside className={shell.inspector} style={{ width }} aria-label={t.inspector.ariaLabel}>
      <div
        className={shell.resizeHandle}
        role="slider"
        tabIndex={0}
        aria-label={t.inspector.widthAria}
        aria-valuemin={INSPECTOR_MIN}
        aria-valuemax={INSPECTOR_MAX}
        aria-valuenow={width}
        aria-orientation="vertical"
        data-testid="inspector-resize"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            step(-16);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            step(16);
          }
        }}
      />
      <div className={shell.inspectorBody}>
        <div className={shell.inspectorHeader}>
          <div className={shell.inspectorTabs} role="tablist" aria-label={t.inspector.viewsAria}>
            <button
              type="button"
              role="tab"
              id="inspector-tab-details"
              aria-controls="inspector-panel-details"
              aria-selected={activeTab === "details"}
              tabIndex={activeTab === "details" ? 0 : -1}
              className={`${shell.inspectorTab} ${activeTab === "details" ? shell.inspectorTabActive : ""}`}
              onClick={() => setActiveTab("details")}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight") {
                  e.preventDefault();
                  setActiveTab("context");
                } else if (e.key === "ArrowLeft") {
                  e.preventDefault();
                  setActiveTab("diagnostics");
                }
              }}
              data-testid="inspector-tab-details"
            >
              {t.inspector.turnDetails}
            </button>
            <button
              type="button"
              role="tab"
              id="inspector-tab-context"
              aria-controls="inspector-panel-context"
              aria-selected={activeTab === "context"}
              tabIndex={activeTab === "context" ? 0 : -1}
              className={`${shell.inspectorTab} ${activeTab === "context" ? shell.inspectorTabActive : ""}`}
              onClick={() => setActiveTab("context")}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight") {
                  e.preventDefault();
                  setActiveTab("diagnostics");
                } else if (e.key === "ArrowLeft") {
                  e.preventDefault();
                  setActiveTab("details");
                }
              }}
              data-testid="inspector-tab-context"
            >
              {t.inspector.context}
            </button>
            <button
              type="button"
              role="tab"
              id="inspector-tab-diagnostics"
              aria-controls="inspector-panel-diagnostics"
              aria-selected={activeTab === "diagnostics"}
              tabIndex={activeTab === "diagnostics" ? 0 : -1}
              className={`${shell.inspectorTab} ${activeTab === "diagnostics" ? shell.inspectorTabActive : ""}`}
              onClick={() => {
                setActiveTab("diagnostics");
                onRefreshDiagnostics?.();
              }}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight") {
                  e.preventDefault();
                  setActiveTab("details");
                } else if (e.key === "ArrowLeft") {
                  e.preventDefault();
                  setActiveTab("context");
                }
              }}
              data-testid="inspector-tab-diagnostics"
            >
              {t.inspector.diagnostics}
            </button>
          </div>
          <button type="button" onClick={onClose} aria-label={t.common.close} data-testid="inspector-close">
            {t.common.close}
          </button>
        </div>
        <div
          role="tabpanel"
          id={`inspector-panel-${activeTab}`}
          aria-labelledby={`inspector-tab-${activeTab}`}
          tabIndex={0}
        >
          {activeTab === "details" ? (
            <InspectorDetails row={focusedRow} activeAgent={activeAgent} />
          ) : activeTab === "context" ? (
            <ContextPanel snapshot={contextSnapshot} status={contextSnapshotStatus} error={contextSnapshotError} />
          ) : (
            <RuntimeDiagnosticsView
              diagnostics={diagnostics}
              status={diagnosticsStatus}
              error={diagnosticsError}
              onRefresh={onRefreshDiagnostics}
            />
          )}
        </div>
      </div>
    </aside>
  );
}

function InspectorDetails({
  row,
  activeAgent,
}: {
  readonly row: TimelineRow | null;
  readonly activeAgent?: AgentProfile | null;
}) {
  const { t } = useI18n();
  if (!row) {
    return (
      <div>
        <p className={shell.inspectorEmpty}>{t.inspector.selectRowToInspect}</p>
        {activeAgent ? (
          <dl className={shell.inspectorFields} style={{ marginTop: "1rem" }}>
            <dt>{t.inspector.agent}</dt>
            <dd>{activeAgent.name}</dd>
            <dt>{t.inspector.model}</dt>
            <dd className={shell.mono}>{activeAgent.model}</dd>
            <dt>{t.inspector.provider}</dt>
            <dd>{activeAgent.provider}</dd>
            {activeAgent.program ? (
              <>
                <dt>Program</dt>
                <dd className={shell.mono}>{activeAgent.program}</dd>
              </>
            ) : null}
            {activeAgent.label ? (
              <>
                <dt>Binding Label</dt>
                <dd>{activeAgent.label}</dd>
              </>
            ) : null}
            {activeAgent.bindingId ? (
              <>
                <dt>Binding ID</dt>
                <dd className={shell.mono}>{activeAgent.bindingId}</dd>
              </>
            ) : null}
            <dt>{t.inspector.secretRef}</dt>
            <dd className={shell.mono}>
              {activeAgent.secretRef ? activeAgent.secretRef : t.inspector.none}
            </dd>
          </dl>
        ) : null}
      </div>
    );
  }
  return (
    <dl className={shell.inspectorFields}>
      <dt>{t.inspector.kind}</dt>
      <dd>{row.kind}</dd>
      <dt>{t.inspector.rowId}</dt>
      <dd className={shell.mono}>{row.id}</dd>
      {row.status ? (
        <>
          <dt>{t.inspector.toolStatus}</dt>
          <dd>{row.status}</dd>
        </>
      ) : null}
      {row.permission ? (
        <>
          <dt>{t.inspector.requestedAction}</dt>
          <dd className={shell.mono}>{row.permission.requestedAction}</dd>
          <dt>{t.inspector.scope}</dt>
          <dd className={shell.mono}>{row.permission.scope}</dd>
          <dt>{t.inspector.decision}</dt>
          <dd>{row.permission.decision ?? "pending"}</dd>
          <dt>{t.inspector.decisionAuthority}</dt>
          <dd>{t.inspector.decisionAuthorityDesc}</dd>
        </>
      ) : null}
      <dt>{t.inspector.text}</dt>
      <dd>{row.text}</dd>
    </dl>
  );
}

/** Navigation-pane resize handle (same slider contract as the inspector). */
export function NavResizeHandle({
  width,
  onWidthChange,
}: {
  readonly width: number;
  readonly onWidthChange: (width: number) => void;
}) {
  const { t } = useI18n();
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null);
  return (
    <div
      className={shell.navResize}
      role="slider"
      tabIndex={0}
      aria-label={t.nav.widthAria}
      aria-valuemin={NAV_MIN}
      aria-valuemax={NAV_MAX}
      aria-valuenow={width}
      aria-orientation="vertical"
      data-testid="nav-resize"
      onPointerDown={(event) => {
        dragState.current = { startX: event.clientX, startWidth: width };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        const state = dragState.current;
        if (!state) return;
        onWidthChange(state.startWidth + (event.clientX - state.startX));
      }}
      onPointerUp={() => {
        dragState.current = null;
      }}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft") {
          event.preventDefault();
          onWidthChange(width - 16);
        } else if (event.key === "ArrowRight") {
          event.preventDefault();
          onWidthChange(width + 16);
        }
      }}
    />
  );
}

export function StatusBar({
  coreState,
  threadStatus,
  streamState,
  onReconnect,
  ipcVersion,
}: {
  readonly coreState: string;
  readonly threadStatus: string;
  readonly streamState?: string;
  readonly onReconnect?: () => void;
  readonly ipcVersion?: number | string;
}) {
  const { t } = useI18n();
  const isDisconnected = coreState === "disconnected" || coreState === "unavailable";

  const coreDetail = (() => {
    if (ipcVersion !== undefined && ipcVersion !== null && coreState === "connected") {
      return t.statusBar.coreConnected(ipcVersion);
    }
    switch (coreState) {
      case "connected":
        return t.statusBar.coreConnected("?");
      case "disconnected":
        return t.statusBar.coreDisconnected;
      case "connecting":
        return t.statusBar.coreConnecting;
      case "reconnecting":
        return t.statusBar.coreReconnecting;
      case "unavailable":
        return t.statusBar.coreUnavailable;
      default:
        return coreState;
    }
  })();

  const threadDetail = (() => {
    switch (threadStatus) {
      case "completed":
        return t.statusBar.threadCompleted;
      case "running":
        return t.statusBar.threadRunning;
      case "failed":
        return t.statusBar.threadFailed;
      case "waiting-for-permission":
        return t.statusBar.threadWaitingPermission;
      default:
        return "Thread · " + threadStatus;
    }
  })();

  const streamDetail = (() => {
    if (!streamState) return null;
    switch (streamState) {
      case "live":
        return t.statusBar.streamLive;
      case "interrupted":
        return t.statusBar.streamInterrupted;
      case "replaying":
        return t.statusBar.streamReplaying;
      case "ready":
        return t.statusBar.streamReady;
      default:
        return "Stream · " + streamState;
    }
  })();

  return (
    <footer className={shell.statusBar} data-testid="status-bar">
      <span>{t.statusBar.coreLine(coreDetail)}</span>
      <span>{threadDetail}</span>
      {streamDetail ? <span>{streamDetail}</span> : null}
      <span>{t.statusBar.localNoSync}</span>
      {isDisconnected && onReconnect ? (
        <button
          type="button"
          onClick={onReconnect}
          className={shell.reconnectBtn}
          data-testid="reconnect-button"
        >
          {t.statusBar.reconnect}
        </button>
      ) : null}
    </footer>
  );
}

export interface AgentOnboardingModalProps {
  readonly isOpen: boolean;
  readonly onClose: () => void;
  readonly onSave: (data: {
    name: string;
    provider: string;
    model?: string;
    program?: string;
    args?: string[] | string;
    envKeys?: string[] | string;
    secretRef?: string;
    secretRefs?: string[] | string;
    envMappings?: readonly EnvSecretMapping[];
    label?: string;
  }) => Promise<void>;
  readonly onTest: (data: {
    provider?: string;
    model?: string;
    program?: string;
    args?: string[] | string;
    envKeys?: string[] | string;
    secretRef?: string;
    secretRefs?: string[] | string;
    envMappings?: readonly EnvSecretMapping[];
    label?: string;
  }) => Promise<TestAgentResult>;
  readonly isTesting: boolean;
  readonly testResult: TestAgentResult | null;
  readonly notice?: React.ReactNode;
}

/** Minimal Agent Onboarding modal with opaque secret reference handling. */
export function AgentOnboardingModal({
  isOpen,
  onClose,
  onSave,
  onTest,
  isTesting,
  testResult,
  notice,
}: AgentOnboardingModalProps) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [provider, setProvider] = useState("acp");
  const [model, setModel] = useState("");
  const [program, setProgram] = useState("");
  const [args, setArgs] = useState("");
  const [envKeys, setEnvKeys] = useState("");
  const [secretRef, setSecretRef] = useState("");
  const [extraMappings, setExtraMappings] = useState<{ id: string; envKey: string; secretRef: string }[]>([]);
  const [label, setLabel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const modalRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<Element | null>(null);

  useEffect(() => {
    if (isOpen) {
      triggerRef.current = document.activeElement;
      const firstInput = modalRef.current?.querySelector<HTMLElement>(
        "input, button, select, textarea",
      );
      firstInput?.focus();
    } else if (triggerRef.current && "focus" in triggerRef.current) {
      (triggerRef.current as HTMLElement).focus();
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isOpen, onClose]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Tab") {
      const focusables = modalRef.current?.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      if (!focusables || focusables.length === 0) return;
      const first = focusables[0]!;
      const last = focusables[focusables.length - 1]!;
      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    }
  };

  if (!isOpen) return null;

  const gatherMappings = () => {
    const list: EnvSecretMapping[] = [];
    const keys: string[] = [];
    const refs: string[] = [];

    const pk = envKeys.trim();
    const pr = secretRef.trim();
    if (pk || pr) {
      const pKeys = parseStringList(pk);
      if (pKeys.length > 1) {
        for (const k of pKeys) {
          keys.push(k);
        }
        if (pr) {
          refs.push(pr);
        }
      } else if (pk) {
        keys.push(pk);
        if (pr) refs.push(pr);
        if (pk && pr) list.push({ envKey: pk, secretRef: pr });
      }
    }

    for (const extra of extraMappings) {
      const ek = extra.envKey.trim();
      const er = extra.secretRef.trim();
      if (ek || er) {
        if (ek) keys.push(ek);
        if (er) refs.push(er);
        if (ek && er) list.push({ envKey: ek, secretRef: er });
      }
    }

    return {
      envKeys: keys,
      secretRefs: refs,
      envMappings: list,
    };
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    setSubmitting(true);
    try {
      const m = gatherMappings();
      await onSave({
        name: name.trim(),
        provider: provider.trim(),
        model: model.trim() || undefined,
        program: program.trim() || provider.trim(),
        args: parseCommandLineArgs(args),
        envKeys: m.envKeys,
        secretRef: m.secretRefs[0] || undefined,
        secretRefs: m.secretRefs,
        envMappings: m.envMappings,
        label: label.trim() || name.trim() || undefined,
      });
      onClose();
    } finally {
      setSubmitting(false);
    }
  };

  const handleTest = async () => {
    const m = gatherMappings();
    await onTest({
      provider: provider.trim(),
      model: model.trim() || undefined,
      program: program.trim() || provider.trim(),
      args: parseCommandLineArgs(args),
      envKeys: m.envKeys,
      secretRef: m.secretRefs[0] || undefined,
      secretRefs: m.secretRefs,
      envMappings: m.envMappings,
      label: label.trim() || name.trim() || undefined,
    });
  };

  return (
    <div
      className={shell.modalOverlay}
      role="dialog"
      aria-modal="true"
      aria-labelledby="onboarding-modal-title"
      ref={modalRef}
      onKeyDown={handleKeyDown}
    >
      <div className={shell.modalCard}>
        <div className={shell.modalHeader}>
          <h2 id="onboarding-modal-title">{t.onboarding.title}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label={t.onboarding.closeAria}
            title={t.common.close}
            data-testid="onboarding-close"
          >
            ×
          </button>
        </div>

        {notice ? (
          <div
            className={shell.modalNotice}
            role="status"
            data-testid="onboarding-notice"
          >
            ⚠️ {notice}
          </div>
        ) : null}

        <form onSubmit={handleSubmit}>
          <div className={shell.formGrid}>
            <label htmlFor="agent-name">{t.onboarding.nameLabel}</label>
            <input
              id="agent-name"
              className={shell.formInput}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t.onboarding.namePlaceholder}
              required
              data-testid="agent-name-input"
            />

            <label htmlFor="agent-provider">{t.onboarding.providerLabel}</label>
            <div>
              <input
                id="agent-provider"
                className={shell.formInput}
                value={provider}
                onChange={(e) => setProvider(e.target.value)}
                placeholder={t.onboarding.providerPlaceholder}
                required
                data-testid="agent-provider-input"
              />
              <span style={{ fontSize: "0.75rem", color: "var(--color-muted)", display: "block", marginTop: "2px" }}>
                {t.agents.acpHarnessNote}
              </span>
            </div>

            <label htmlFor="agent-model">{t.onboarding.modelLabel}</label>
            <div>
              <input
                id="agent-model"
                className={shell.formInput}
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={t.onboarding.modelPlaceholder}
                data-testid="agent-model-input"
              />
              <span style={{ fontSize: "0.75rem", color: "var(--color-muted)", display: "block", marginTop: "2px" }}>
                {t.agents.modelOptionalNote}
              </span>
            </div>

            <label htmlFor="agent-program">{t.onboarding.programPathLabel}</label>
            <input
              id="agent-program"
              className={shell.formInput}
              value={program}
              onChange={(e) => setProgram(e.target.value)}
              placeholder={t.onboarding.programPathPlaceholder}
              data-testid="agent-program-input"
            />

            <label htmlFor="agent-args">{t.onboarding.argsLabel}</label>
            <input
              id="agent-args"
              className={shell.formInput}
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder={t.onboarding.argsPlaceholder}
              data-testid="agent-args-input"
            />

            <label htmlFor="agent-env-keys">{t.onboarding.envSecretsLabel}</label>
            <div>
              <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                <input
                  id="agent-env-keys"
                  className={shell.formInput}
                  value={envKeys}
                  onChange={(e) => setEnvKeys(e.target.value)}
                  placeholder={t.onboarding.envKeysPlaceholder}
                  data-testid="agent-env-keys-input"
                  style={{ flex: 1 }}
                />
                <input
                  id="agent-secret"
                  className={shell.formInput}
                  value={secretRef}
                  onChange={(e) => setSecretRef(e.target.value)}
                  placeholder={t.onboarding.secretPointerPlaceholder}
                  data-testid="agent-secret-ref"
                  style={{ flex: 1 }}
                />
                <button
                  type="button"
                  onClick={() => setExtraMappings((prev) => [...prev, { id: `extra-${Date.now()}-${Math.random()}`, envKey: "", secretRef: "" }])}
                  data-testid="add-env-mapping-btn"
                  style={{ padding: "0 var(--spacing-8)", height: "var(--control-height)", cursor: "pointer" }}
                >
                  {t.onboarding.addMapping}
                </button>
              </div>

              {extraMappings.map((row, idx) => (
                <div key={row.id} style={{ display: "flex", gap: "8px", marginTop: "6px", alignItems: "center" }} data-testid={`env-mapping-row-${idx}`}>
                  <input
                    className={shell.formInput}
                    value={row.envKey}
                    onChange={(e) => {
                      const v = e.target.value;
                      setExtraMappings((prev) => prev.map((r) => r.id === row.id ? { ...r, envKey: v } : r));
                    }}
                    placeholder={t.onboarding.variableKeyPlaceholder}
                    data-testid={`env-key-input-${idx}`}
                    style={{ flex: 1 }}
                  />
                  <input
                    className={shell.formInput}
                    value={row.secretRef}
                    onChange={(e) => {
                      const v = e.target.value;
                      setExtraMappings((prev) => prev.map((r) => r.id === row.id ? { ...r, secretRef: v } : r));
                    }}
                    placeholder={t.onboarding.secretRefPlaceholder}
                    data-testid={`secret-ref-input-${idx}`}
                    style={{ flex: 1 }}
                  />
                  <button
                    type="button"
                    onClick={() => setExtraMappings((prev) => prev.filter((r) => r.id !== row.id))}
                    data-testid={`remove-mapping-btn-${idx}`}
                    style={{ background: "transparent", border: "none", color: "var(--color-danger, #b3362b)", cursor: "pointer", fontSize: "1.2rem", padding: "0 4px" }}
                    title={t.onboarding.removeMappingAria}
                    aria-label={t.onboarding.removeMappingAria}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>

            <label htmlFor="agent-label">{t.onboarding.labelLabel}</label>
            <input
              id="agent-label"
              className={shell.formInput}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder={t.onboarding.primaryBindingPlaceholder}
              data-testid="agent-label-input"
            />

            <p className={shell.secretNotice}>
              🔒 {t.onboarding.secretNotice}
            </p>
            <p style={{ fontSize: "0.75rem", color: "var(--color-muted)", marginTop: "4px" }} data-testid="agent-deferred-notice">
              ℹ️ {t.agents.deferredNotice}
            </p>
          </div>

          <div style={{ marginTop: "0.5rem" }}>
            {testResult ? (
              testResult.success ? (
                <div>
                  <span className={shell.testResultOk}>
                    ✓ {t.onboarding.verifiedWithLatency(testResult.latencyMs ?? 0)}
                  </span>
                  {testResult.capabilities && Object.keys(testResult.capabilities).length > 0 ? (
                    <div style={{ fontSize: "0.75rem", color: "var(--color-muted)", marginTop: "4px" }} data-testid="tested-capabilities">
                      {t.onboarding.capabilitiesLabel}: {Object.entries(testResult.capabilities).map(([k, v]) => `${k}: ${v}`).join(", ")}
                    </div>
                  ) : null}
                </div>
              ) : (
                <span className={shell.testResultErr}>
                  × {t.onboarding.testFailed(testResult.error ?? "")}
                </span>
              )
            ) : null}
          </div>

          <div className={shell.modalActions}>
            <button
              type="button"
              onClick={handleTest}
              disabled={isTesting}
              data-testid="agent-test-button"
            >
              {isTesting ? t.onboarding.testing : t.onboarding.testConnection}
            </button>
            <button
              type="submit"
              disabled={submitting || !name.trim()}
              data-testid="agent-save-button"
            >
              {submitting ? t.common.save : t.onboarding.saveAgent}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

export interface SettingsModalProps {
  readonly isOpen: boolean;
  readonly onClose: () => void;
  readonly themeSource: ThemeSource;
  readonly onThemeSourceChange: (theme: ThemeSource) => void;
  readonly localeSource: LocaleSource;
  readonly onLocaleSourceChange: (locale: LocaleSource) => void;
  readonly diagnostics?: RuntimeDiagnosticsDto | null;
  readonly diagnosticsStatus?: "idle" | "loading" | "loaded" | "error";
  readonly onRefreshDiagnostics?: () => void;
}

export function SettingsModal({
  isOpen,
  onClose,
  themeSource,
  onThemeSourceChange,
  localeSource,
  onLocaleSourceChange,
  diagnostics,
  diagnosticsStatus,
  onRefreshDiagnostics,
}: SettingsModalProps) {
  const { t } = useI18n();
  const modalRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<Element | null>(null);

  useEffect(() => {
    if (isOpen) {
      triggerRef.current = document.activeElement;
      const firstInput = modalRef.current?.querySelector<HTMLElement>(
        "input, button, select, textarea",
      );
      firstInput?.focus();
    } else if (triggerRef.current && "focus" in triggerRef.current) {
      (triggerRef.current as HTMLElement).focus();
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      } else if (e.key === "Tab") {
        if (!modalRef.current) return;
        const focusable = modalRef.current.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (e.shiftKey) {
          if (document.activeElement === first) {
            e.preventDefault();
            last?.focus();
          }
        } else {
          if (document.activeElement === last) {
            e.preventDefault();
            first?.focus();
          }
        }
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <div
      className={shell.modalOverlay}
      role="dialog"
      aria-modal="true"
      aria-label={t.settings.title}
      ref={modalRef}
      data-testid="settings-modal"
    >
      <div className={shell.modalCard}>
        <div className={shell.modalHeader}>
          <h2>{t.settings.title}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label={t.common.close}
            data-testid="settings-close-btn"
          >
            ×
          </button>
        </div>

        <div className={shell.formGrid}>
          <label htmlFor="settings-theme">{t.settings.theme}</label>
          <select
            id="settings-theme"
            className={shell.formInput}
            value={themeSource}
            onChange={(e) => onThemeSourceChange(e.target.value as ThemeSource)}
            data-testid="settings-theme-select"
          >
            <option value="system">{t.settings.themeSystem}</option>
            <option value="light">{t.settings.themeLight}</option>
            <option value="dark">{t.settings.themeDark}</option>
          </select>

          <label htmlFor="settings-language">{t.settings.language}</label>
          <select
            id="settings-language"
            className={shell.formInput}
            value={localeSource}
            onChange={(e) => onLocaleSourceChange(e.target.value as LocaleSource)}
            data-testid="settings-locale-select"
          >
            <option value="system">{t.settings.langSystem}</option>
            <option value="en">{t.settings.langEn}</option>
            <option value="zh-CN">{t.settings.langZh}</option>
          </select>

          <div className={shell.secretNotice} style={{ marginTop: "var(--spacing-8)" }}>
            {t.settings.deviceNotice}
          </div>
        </div>

        <div style={{ marginTop: "16px", borderTop: "1px solid var(--color-border)", paddingTop: "12px" }} data-testid="settings-diagnostics-section">
          <h3 style={{ fontSize: "0.875rem", marginBottom: "8px" }}>{t.inspector.diagnostics}</h3>
          <RuntimeDiagnosticsView
            diagnostics={diagnostics}
            status={diagnosticsStatus}
            onRefresh={onRefreshDiagnostics}
          />
        </div>

        <div className={shell.modalActions}>
          <button
            type="button"
            onClick={onClose}
            data-testid="settings-done-button"
          >
            {t.common.close}
          </button>
        </div>
      </div>
    </div>
  );
}

