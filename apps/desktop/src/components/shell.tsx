/**
 * Workbench shell regions (ADR 0008 §2): activity rail, navigation pane,
 * thread header, composer, inspector, status bar. Five stable regions
 * per docs/UI_ARCHITECTURE.md; panes resize by drag or keyboard within
 * their token clamps.
 */
import { useCallback, useRef, useState } from "react";
import type { TimelineRow } from "../features/timeline/timelineStore";
import type { ThreadFixture, ThreadStatus } from "../fixtures/timeline";
import type { AgentProfile } from "../stores/applicationStore";
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

export interface ActivityRailProps {
  readonly active: string;
  readonly onNavigate?: (destination: string) => void;
}

/** Activity rail: Threads and Agents; others arrive with subsequent phases. */
export function ActivityRail({ active, onNavigate }: ActivityRailProps) {
  const destinations: { id: string; label: string; arrives: string | null }[] = [
    { id: "threads", label: "Threads", arrives: null },
    { id: "agents", label: "Agents", arrives: null },
    { id: "projects", label: "Projects", arrives: "P1" },
    { id: "memory", label: "Memory", arrives: "P2" },
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
            onClick={() => enabled && onNavigate?.(id)}
            title={enabled ? label : `${label} — arrives with ${arrives}`}
            data-testid={`rail-${id}`}
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
  readonly onCreateThread?: () => void;
}

/** Navigation pane: pinned and recent threads with search and thread creation. */
export function ThreadsPane({
  threads,
  selectedThreadId,
  onSelect,
  filter,
  onFilterChange,
  onCreateThread,
}: ThreadsPaneProps) {
  const matches = threads.filter(
    (thread) =>
      thread.title.toLowerCase().includes(filter.toLowerCase()) ||
      thread.agent.toLowerCase().includes(filter.toLowerCase()),
  );
  const pinned = matches.filter((thread) => thread.pinned);
  const recent = matches.filter((thread) => !thread.pinned);

  return (
    <section className={shell.threadsPane} aria-label="Threads">
      <div className={shell.threadsHeader}>
        <input
          type="search"
          className={shell.search}
          placeholder="Filter threads"
          value={filter}
          onChange={(event) => onFilterChange(event.target.value)}
          aria-label="Filter threads"
          data-testid="thread-filter"
        />
        {onCreateThread ? (
          <button
            type="button"
            className={shell.newThreadBtn}
            onClick={onCreateThread}
            data-testid="new-thread"
            title="Create new thread"
          >
            + New
          </button>
        ) : null}
      </div>
      {pinned.length > 0 ? (
        <ThreadSection
          title="Pinned"
          threads={pinned}
          selectedThreadId={selectedThreadId}
          onSelect={onSelect}
        />
      ) : null}
      <ThreadSection
        title="Recent"
        threads={recent}
        selectedThreadId={selectedThreadId}
        onSelect={onSelect}
      />
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
  return (
    <header className={shell.threadHeader}>
      <h1 className={shell.threadTitleMain}>{title}</h1>

      {agents && onSelectAgent ? (
        <div style={{ display: "flex", gap: "var(--spacing-8)", alignItems: "center" }}>
          <select
            className={shell.agentSelect}
            value={
              selectedAgentId ??
              agents.find((a) => a.name === agent || a.id === agent)?.id ??
              agents[0]?.id
            }
            onChange={(e) => onSelectAgent(e.target.value)}
            aria-label="Select agent"
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
              title="Add agent"
            >
              + Agent
            </button>
          ) : null}
        </div>
      ) : (
        <span className={shell.threadAgent}>{agent}</span>
      )}

      <div className={shell.headerControls}>
        {isStreaming && onCancel ? (
          <button
            type="button"
            onClick={onCancel}
            className={shell.cancelBtn}
            data-testid="header-cancel-turn"
          >
            Cancel turn
          </button>
        ) : null}
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
  readonly onCancel?: () => void;
  readonly isStreaming?: boolean;
  readonly disabledReason: string | null;
}

/** Composer: one draft per thread; Enter sends, Shift+Enter breaks lines. */
export function Composer({
  draft,
  onDraftChange,
  onSend,
  onCancel,
  isStreaming,
  disabledReason,
}: ComposerProps) {
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
      {isStreaming && onCancel ? (
        <button
          type="button"
          className={shell.cancelBtn}
          onClick={onCancel}
          data-testid="cancel-turn"
        >
          Cancel
        </button>
      ) : null}
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
  readonly activeAgent?: AgentProfile | null;
}

/**
 * Inspector: one contextual pane for turn details, tool output, and
 * provenance. The resize handle is a slider: drag or arrow keys, clamped
 * to the token range.
 */
export function Inspector({
  width,
  onWidthChange,
  onClose,
  focusedRow,
  activeAgent,
}: InspectorProps) {
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
        <InspectorDetails row={focusedRow} activeAgent={activeAgent} />
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
  if (!row) {
    return (
      <div>
        <p className={shell.inspectorEmpty}>Select a timeline row to inspect it.</p>
        {activeAgent ? (
          <dl className={shell.inspectorFields} style={{ marginTop: "1rem" }}>
            <dt>Agent</dt>
            <dd>{activeAgent.name}</dd>
            <dt>Model</dt>
            <dd className={shell.mono}>{activeAgent.model}</dd>
            <dt>Provider</dt>
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
            <dt>Secret Ref</dt>
            <dd className={shell.mono}>
              {activeAgent.secretRef ? activeAgent.secretRef : "none"}
            </dd>
          </dl>
        ) : null}
      </div>
    );
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
  streamState,
  onReconnect,
}: {
  readonly coreState: string;
  readonly threadStatus: string;
  readonly streamState?: string;
  readonly onReconnect?: () => void;
}) {
  const isDisconnected = coreState.includes("disconnected") || coreState.includes("unavailable");
  return (
    <footer className={shell.statusBar} data-testid="status-bar">
      <span>Core · {coreState}</span>
      <span>Thread · {threadStatus}</span>
      {streamState ? <span>Stream · {streamState}</span> : null}
      <span>Local · no sync (P3)</span>
      {isDisconnected && onReconnect ? (
        <button
          type="button"
          onClick={onReconnect}
          className={shell.reconnectBtn}
          data-testid="reconnect-button"
        >
          Reconnect
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
    model: string;
    program?: string;
    args?: string[] | string;
    envKeys?: string[] | string;
    secretRef?: string;
    label?: string;
  }) => Promise<void>;
  readonly onTest: (data: {
    provider?: string;
    model?: string;
    program?: string;
    args?: string[] | string;
    envKeys?: string[] | string;
    secretRef?: string;
    label?: string;
  }) => Promise<{ success: boolean; latencyMs?: number; error?: string }>;
  readonly isTesting: boolean;
  readonly testResult: { success: boolean; latencyMs?: number; error?: string } | null;
}

/** Minimal Agent Onboarding modal with opaque secret reference handling. */
export function AgentOnboardingModal({
  isOpen,
  onClose,
  onSave,
  onTest,
  isTesting,
  testResult,
}: AgentOnboardingModalProps) {
  const [name, setName] = useState("");
  const [provider, setProvider] = useState("acp");
  const [model, setModel] = useState("claude-3-7-sonnet");
  const [program, setProgram] = useState("");
  const [args, setArgs] = useState("");
  const [envKeys, setEnvKeys] = useState("");
  const [secretRef, setSecretRef] = useState("");
  const [label, setLabel] = useState("");
  const [submitting, setSubmitting] = useState(false);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    setSubmitting(true);
    try {
      await onSave({
        name: name.trim(),
        provider: provider.trim(),
        model: model.trim(),
        program: program.trim() || provider.trim(),
        args: args.trim() ? args.trim().split(/\s+/) : [],
        envKeys: envKeys.trim() ? envKeys.trim().split(/[,\s]+/) : [],
        secretRef: secretRef.trim() || undefined,
        label: label.trim() || name.trim() || undefined,
      });
      onClose();
    } finally {
      setSubmitting(false);
    }
  };

  const handleTest = async () => {
    await onTest({
      provider: provider.trim(),
      model: model.trim(),
      program: program.trim() || provider.trim(),
      args: args.trim() ? args.trim().split(/\s+/) : [],
      envKeys: envKeys.trim() ? envKeys.trim().split(/[,\s]+/) : [],
      secretRef: secretRef.trim() || undefined,
      label: label.trim() || name.trim() || undefined,
    });
  };

  return (
    <div className={shell.modalOverlay} role="dialog" aria-modal="true" aria-label="Agent Onboarding">
      <div className={shell.modalCard}>
        <div className={shell.modalHeader}>
          <h2>Agent Onboarding</h2>
          <button type="button" onClick={onClose} data-testid="onboarding-close">
            ×
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className={shell.formGrid}>
            <label htmlFor="agent-name">Name</label>
            <input
              id="agent-name"
              className={shell.formInput}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Gamma Agent"
              required
              data-testid="agent-name-input"
            />

            <label htmlFor="agent-provider">Provider</label>
            <input
              id="agent-provider"
              className={shell.formInput}
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              placeholder="acp / terminal / native"
              required
              data-testid="agent-provider-input"
            />

            <label htmlFor="agent-model">Model</label>
            <input
              id="agent-model"
              className={shell.formInput}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="claude-3-7-sonnet"
              required
              data-testid="agent-model-input"
            />

            <label htmlFor="agent-program">Program</label>
            <input
              id="agent-program"
              className={shell.formInput}
              value={program}
              onChange={(e) => setProgram(e.target.value)}
              placeholder="/usr/local/bin/acp-agent or command"
              data-testid="agent-program-input"
            />

            <label htmlFor="agent-args">Args</label>
            <input
              id="agent-args"
              className={shell.formInput}
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder="--mode server --verbose"
              data-testid="agent-args-input"
            />

            <label htmlFor="agent-env-keys">Env Keys</label>
            <input
              id="agent-env-keys"
              className={shell.formInput}
              value={envKeys}
              onChange={(e) => setEnvKeys(e.target.value)}
              placeholder="ANTHROPIC_API_KEY, DEBUG"
              data-testid="agent-env-keys-input"
            />

            <label htmlFor="agent-label">Label</label>
            <input
              id="agent-label"
              className={shell.formInput}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="Primary ACP Binding"
              data-testid="agent-label-input"
            />

            <label htmlFor="agent-secret">Secret Ref</label>
            <input
              id="agent-secret"
              className={shell.formInput}
              value={secretRef}
              onChange={(e) => setSecretRef(e.target.value)}
              placeholder="vault://key-id or env:VAR"
              data-testid="agent-secret-ref"
            />

            <p className={shell.secretNotice}>
              🔒 Plaintext keys are never stored. Only opaque reference pointers (e.g. vault://..., env:...) are accepted.
            </p>
          </div>

          <div style={{ marginTop: "0.5rem" }}>
            {testResult ? (
              testResult.success ? (
                <span className={shell.testResultOk}>
                  ✓ Connection verified ({testResult.latencyMs}ms)
                </span>
              ) : (
                <span className={shell.testResultErr}>
                  × Test failed: {testResult.error}
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
              {isTesting ? "Testing…" : "Test Connection"}
            </button>
            <button
              type="submit"
              disabled={submitting || !name.trim()}
              data-testid="agent-save-button"
            >
              {submitting ? "Saving…" : "Save Agent"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
