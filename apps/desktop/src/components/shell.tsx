/**
 * Workbench shell regions (ADR 0008 §2): activity rail, navigation pane,
 * thread header, composer, inspector, status bar. Five stable regions
 * per docs/UI_ARCHITECTURE.md; panes resize by drag or keyboard within
 * their token clamps.
 */
import { useCallback, useRef } from "react";
import type { TimelineRow } from "../features/timeline/timelineStore";
import type { ThreadFixture, ThreadStatus } from "../fixtures/timeline";
import {
  INSPECTOR_MAX,
  INSPECTOR_MIN,
  NAV_MAX,
  NAV_MIN,
} from "../app/uiStore";
import shell from "./shell.module.css";

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

/** Activity rail: Threads now; the rest are explicitly unavailable. */
export function ActivityRail({ active }: { readonly active: string }) {
  const destinations: { id: string; label: string; arrives: string | null }[] = [
    { id: "threads", label: "Threads", arrives: null },
    { id: "projects", label: "Projects", arrives: "P1" },
    { id: "memory", label: "Memory", arrives: "P2" },
    { id: "agents", label: "Agents", arrives: "P1" },
    { id: "devices", label: "Devices", arrives: "P3" },
    { id: "settings", label: "Settings", arrives: "P1" },
  ];
  return (
    <nav className={shell.rail} aria-label="Activity">
      {destinations.map(({ id, label, arrives }) => {
        const enabled = arrives == null;
        return (
          <button
            key={id}
            type="button"
            className={`${shell.railItem} ${active === id ? shell.railActive : ""}`}
            aria-current={active === id ? "page" : undefined}
            aria-disabled={!enabled}
            disabled={!enabled}
            title={enabled ? label : `${label} — arrives with ${arrives}`}
          >
            <span aria-hidden="true">{label.slice(0, 2)}</span>
            <span className={shell.railLabel}>{label}</span>
          </button>
        );
      })}
    </nav>
  );
}

export interface ThreadsPaneProps {
  readonly threads: readonly ThreadFixture[];
  readonly selectedThreadId: string;
  readonly onSelect: (id: string) => void;
  readonly filter: string;
  readonly onFilterChange: (value: string) => void;
}

/** Navigation pane: pinned and recent threads with status indicators. */
export function ThreadsPane({
  threads,
  selectedThreadId,
  onSelect,
  filter,
  onFilterChange,
}: ThreadsPaneProps) {
  const matches = threads.filter((thread) =>
    thread.title.toLowerCase().includes(filter.toLowerCase()),
  );
  const pinned = matches.filter((thread) => thread.pinned);
  const recent = matches.filter((thread) => !thread.pinned);
  return (
    <section className={shell.threadsPane} aria-label="Threads">
      <input
        type="search"
        className={shell.search}
        placeholder="Filter threads"
        value={filter}
        onChange={(event) => onFilterChange(event.target.value)}
        aria-label="Filter threads"
        data-testid="thread-filter"
      />
      {pinned.length > 0 ? (
        <ThreadSection title="Pinned" threads={pinned} selectedThreadId={selectedThreadId} onSelect={onSelect} />
      ) : null}
      <ThreadSection title="Recent" threads={recent} selectedThreadId={selectedThreadId} onSelect={onSelect} />
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
  readonly threads: readonly ThreadFixture[];
  readonly selectedThreadId: string;
  readonly onSelect: (id: string) => void;
}) {
  return (
    <section className={shell.threadSection} aria-label={title}>
      <h2 className={shell.sectionTitle}>{title}</h2>
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
    </section>
  );
}

export interface ThreadHeaderProps {
  readonly title: string;
  readonly agent: string;
  readonly theme: "light" | "dark";
  readonly onToggleTheme: () => void;
  readonly inspectorOpen: boolean;
  readonly onToggleInspector: () => void;
}

export function ThreadHeader({
  title,
  agent,
  theme,
  onToggleTheme,
  inspectorOpen,
  onToggleInspector,
}: ThreadHeaderProps) {
  return (
    <header className={shell.threadHeader}>
      <h1 className={shell.threadTitleMain}>{title}</h1>
      <span className={shell.threadAgent}>{agent}</span>
      <div className={shell.headerControls}>
        <button type="button" onClick={onToggleTheme} data-testid="theme-toggle">
          {theme === "light" ? "Dark theme" : "Light theme"}
        </button>
        <button type="button" onClick={onToggleInspector} data-testid="inspector-toggle">
          {inspectorOpen ? "Hide inspector" : "Show inspector"}
        </button>
      </div>
    </header>
  );
}

export interface ComposerProps {
  readonly draft: string;
  readonly onDraftChange: (text: string) => void;
  readonly onSend: () => void;
  readonly disabledReason: string | null;
}

/** Composer: one draft per thread; Enter sends, Shift+Enter breaks lines. */
export function Composer({ draft, onDraftChange, onSend, disabledReason }: ComposerProps) {
  return (
    <div className={shell.composer}>
      <textarea
        className={shell.composerInput}
        placeholder={disabledReason ?? "Message the agent… (Enter to send)"}
        value={draft}
        disabled={disabledReason != null}
        onChange={(event) => onDraftChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            if (disabledReason == null) onSend();
          }
        }}
        aria-label="Composer"
        data-testid="composer"
      />
      <button
        type="button"
        className={shell.send}
        onClick={onSend}
        disabled={disabledReason != null || draft.trim().length === 0}
        data-testid="send"
      >
        Send
      </button>
    </div>
  );
}

export interface InspectorProps {
  readonly width: number;
  readonly onWidthChange: (width: number) => void;
  readonly onClose: () => void;
  readonly focusedRow: TimelineRow | null;
}

/**
 * Inspector: one contextual pane for turn details, tool output, and
 * provenance. The resize handle is a slider: drag or arrow keys, clamped
 * to the token range.
 */
export function Inspector({ width, onWidthChange, onClose, focusedRow }: InspectorProps) {
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
    <aside className={shell.inspector} style={{ width }} aria-label="Inspector">
      <div
        className={shell.resizeHandle}
        role="slider"
        tabIndex={0}
        aria-label="Inspector width"
        aria-valuemin={INSPECTOR_MIN}
        aria-valuemax={INSPECTOR_MAX}
        aria-valuenow={width}
        aria-orientation="vertical"
        data-testid="inspector-resize"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onKeyDown={(event) => {
          // Standard slider semantics: the value is the pane width.
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
          <h2 className={shell.sectionTitle}>Turn details</h2>
          <button type="button" onClick={onClose} data-testid="inspector-close">
            Close
          </button>
        </div>
        <InspectorDetails row={focusedRow} />
      </div>
    </aside>
  );
}

function InspectorDetails({ row }: { readonly row: TimelineRow | null }) {
  if (!row) {
    return <p className={shell.inspectorEmpty}>Select a timeline row to inspect it.</p>;
  }
  return (
    <dl className={shell.inspectorFields}>
      <dt>Kind</dt>
      <dd>{row.kind}</dd>
      <dt>Row id</dt>
      <dd className={shell.mono}>{row.id}</dd>
      {row.status ? (
        <>
          <dt>Tool status</dt>
          <dd>{row.status}</dd>
        </>
      ) : null}
      {row.permission ? (
        <>
          <dt>Requested action</dt>
          <dd className={shell.mono}>{row.permission.requestedAction}</dd>
          <dt>Scope</dt>
          <dd className={shell.mono}>{row.permission.scope}</dd>
          <dt>Decision</dt>
          <dd>{row.permission.decision ?? "pending"}</dd>
          <dt>Decision authority</dt>
          <dd>Provisional UI decision; the P1 runtime owns the real command.</dd>
        </>
      ) : null}
      <dt>Text</dt>
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
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null);
  return (
    <div
      className={shell.navResize}
      role="slider"
      tabIndex={0}
      aria-label="Threads pane width"
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
}: {
  readonly coreState: string;
  readonly threadStatus: string;
}) {
  return (
    <footer className={shell.statusBar} data-testid="status-bar">
      <span>Core · {coreState}</span>
      <span>Thread · {threadStatus}</span>
      <span>Local · no sync (P3)</span>
    </footer>
  );
}
