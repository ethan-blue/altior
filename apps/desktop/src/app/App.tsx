/**
 * Workbench shell composition (P1.3, ADR 0008).
 *
 * The five stable regions from docs/UI_ARCHITECTURE.md compose here:
 * title bar, activity rail, threads pane, workbench (header, timeline,
 * composer), inspector, status bar. Renderer-owned state lives in
 * `uiStore`; Core-facing state is transport-driven through `applicationStore`.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  ActivityRail,
  AgentOnboardingModal,
  Composer,
  Inspector,
  NavResizeHandle,
  SettingsModal,
  MemoryPane,
  StatusBar,
  ThreadHeader,
  ThreadsPane,
} from "../components/shell";
import { I18nProvider, getDictionary } from "../i18n";
import { Timeline } from "../features/timeline/Timeline";
import type { TimelineRow } from "../features/timeline/timelineStore";
import type { PermissionDecision } from "../features/timeline/timelineStore";
import { createDefaultTransport } from "../ipc/tauriTransport";
import type { CoreTransport } from "../ipc/transport";
import { createApplicationStore } from "../stores/applicationStore";
import styles from "./App.module.css";
import { createUiStore, useUiState } from "./uiStore";

export interface AppProps {
  /** Transport to run against; defaults to the environment default transport factory. */
  readonly transport?: CoreTransport;
  /**
   * Fixture-world only (review A03): seeds curated timeline rows for the
   * synthetic shell. Production entry points never pass this.
   */
  readonly fixtureTimelineRows?: readonly {
    readonly id: string;
    readonly rows: readonly TimelineRow[];
  }[];
  /** Test injection for the timeline viewport (jsdom has no layout). */
  readonly timelineViewportHeight?: number;
}

export function App({
  transport,
  fixtureTimelineRows,
  timelineViewportHeight,
}: AppProps) {
  const [resolvedTransport] = useState(() => transport ?? createDefaultTransport());
  const [appStore] = useState(() =>
    createApplicationStore(resolvedTransport, { fixtureTimelineRows }),
  );
  const appState = useSyncExternalStore(
    appStore.subscribe,
    appStore.getState,
    appStore.getState,
  );

  const [uiStore] = useState(() =>
    createUiStore(appState.threads[0]?.id ?? ""),
  );
  const ui = useUiState(uiStore);

  const [focusedRowId, setFocusedRowId] = useState<string | null>(null);
  const [narrow, setNarrow] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Initialize transport connection on mount.
  // Cleanup releases the renderer's event listener only; it never closes
  // the Core process or tears down the negotiated connection, and init is
  // idempotent so StrictMode's mount→cleanup→mount cycle neither duplicates
  // bootstrap commands nor drops events (review A02).
  useEffect(() => {
    void appStore.init();
    return () => appStore.release();
  }, [appStore]);

  // The conversation being read is independent of the visible list: an
  // empty search result never unmounts it (review A03, F06).
  const currentThread = appState.selectedThread;
  const store = currentThread ? appStore.getTimelineStore(currentThread.id) : null;
  const focusedRow = focusedRowId == null || !store ? null : store.getRow(focusedRowId);
  const activeAgent =
    appState.agents.find((a) => currentThread && (a.name === currentThread.agent || a.id === currentThread.agent)) ??
    appState.agents.find((a) => a.id === appState.selectedAgentId) ??
    appState.agents[0];

  // Narrow-width detection based on content budget (A08, DESIGN_I18N §2).
  // Side-by-side layout requires rail (48) + navWidth + workbench min (480) + inspector (360) + dividers (24).
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const update = () => {
      const w = root.clientWidth;
      if (w <= 0) return;
      const required = 48 + ui.navWidth + 480 + (ui.inspectorOpen ? ui.inspectorWidth : 0) + 24;
      setNarrow(w < required);
    };
    update();
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(update);
      observer.observe(root);
      return () => observer.disconnect();
    }
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [ui.navWidth, ui.inspectorOpen, ui.inspectorWidth]);

  // Resolve context snapshot when focused turn/row changes (A14)
  useEffect(() => {
    if (!currentThread) return;
    if (!focusedRowId) {
      void appStore.getContextSnapshot(currentThread.id, null);
      return;
    }
    const turnId = focusedRowId.startsWith("trn_")
      ? focusedRowId
      : (appState.activeTurns.find(
          (t) => t.userRowId === focusedRowId || t.replyRowId === focusedRowId,
        )?.turnId ?? null);
    void appStore.getContextSnapshot(currentThread.id, turnId);
  }, [focusedRowId, currentThread?.id, appStore, appState.activeTurns]);

  // Dismiss inspector drawer on Escape in narrow mode (A08 / F14)
  useEffect(() => {
    if (!narrow || !ui.inspectorOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        uiStore.setInspectorOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [narrow, ui.inspectorOpen, uiStore]);

  const onSend = useCallback(async () => {
    if (!currentThread) return;
    const text = (ui.drafts[currentThread.id] ?? "").trim();
    if (!text) return;

    // The draft is kept until the dispatch settles (A04): an unconfirmed
    // send never silently destroys the user's text.
    const result = await appStore.sendPrompt(text, currentThread.id);
    if (result.status === "admitted") {
      uiStore.setDraft(currentThread.id, "");
    } else if (result.status === "rejected") {
      // Not delivered — the text stays in the composer so the user can
      // correct and resend deliberately.
      uiStore.setDraft(currentThread.id, text);
    }
    // Indeterminate: the text stays in the composer read-only territory of
    // the user; the timeline notice tells them to verify before resending.
  }, [appStore, currentThread, ui.drafts, uiStore]);

  const onCancelTurn = useCallback(async () => {
    if (!currentThread) return;
    await appStore.cancelActiveTurn(currentThread.id);
  }, [appStore, currentThread]);

  const onPermissionDecision = useCallback(
    async (id: string, decision: PermissionDecision) => {
      await appStore.decidePermission(id, decision);
    },
    [appStore],
  );

  const onFirstVisibleChange = useCallback(
    (rowId: string) => {
      if (currentThread) uiStore.setAnchor(currentThread.id, rowId);
    },
    [currentThread, uiStore],
  );

  const onSelectThread = useCallback(
    (id: string) => {
      void appStore.selectThread(id);
      uiStore.selectThread(id);
      setFocusedRowId(null);
    },
    [appStore, uiStore],
  );

  const onCreateThread = useCallback(async () => {
    const targetAgentId = appState.selectedAgentId || activeAgent?.id;
    try {
      const newThread = await appStore.createThread(
        `Thread ${appState.threads.length + 1}`,
        targetAgentId,
      );
      uiStore.selectThread(newThread.id);
    } catch {
      // The store surfaces the failure reason in its error state; the
      // composer/list stay usable (A03: no fabricated created thread).
    }
    setFocusedRowId(null);
  }, [activeAgent?.id, appState.selectedAgentId, appState.threads.length, appStore, uiStore]);

  const onActivityNavigate = useCallback(
    (dest: string) => {
      if (dest === "threads" || dest === "memory") {
        uiStore.setActiveRail(dest);
      } else if (dest === "agents") {
        appStore.openOnboarding(true);
      } else if (dest === "settings") {
        uiStore.setSettingsOpen(true);
      }
    },
    [appStore, uiStore],
  );

  const currentThreadActiveTurn =
    currentThread != null
      ? appState.activeTurns.find((t) => t.threadId === currentThread.id) ?? null
      : null;
  const isCurrentThreadStreaming = currentThreadActiveTurn?.isStreaming === true;

  return (
    <I18nProvider
      localeSource={ui.localeSource}
      onLocaleSourceChange={uiStore.setLocaleSource}
    >
      <div
        ref={rootRef}
        className={styles.shell}
        data-theme={ui.theme}
        data-narrow={narrow}
        data-connection-status={appState.connectionStatus}
        data-fixture-ready={appState.connectionStatus === "connected" && appState.threads.length > 0}
      >
      <header className={styles.titleBar}>
        <strong>Altior</strong>
        <span data-testid="ipc-version">
          {appState.negotiated
            ? `IPC v${appState.negotiated.selected_version}`
            : appState.connectionStatus === "connecting"
              ? "IPC connecting…"
              : `IPC (${appState.connectionStatus})`}
        </span>
      </header>

      <div className={styles.railArea}>
        <ActivityRail active={ui.activeRail} onNavigate={onActivityNavigate} />
      </div>

      <div className={styles.navArea}>
        <div style={{ width: ui.navWidth }}>
          <ThreadsPane
            threads={appState.threads}
            selectedThreadId={appState.selectedThreadId}
            onSelect={onSelectThread}
            filter={appState.threadFilter}
            onFilterChange={(f) => {
              void appStore.setThreadFilter(f);
            }}
            onCreateThread={onCreateThread}
            hasMoreThreads={appState.hasMoreThreads && !appState.searchActive}
            onLoadMore={() => {
              void appStore.loadMoreThreads();
            }}
            searchActive={appState.searchActive}
          />
        </div>
        <NavResizeHandle
          width={ui.navWidth}
          onWidthChange={uiStore.setNavWidth}
        />
      </div>

      {ui.activeRail === "memory" ? (
        <main className={styles.workbench} data-testid="workbench-memory">
          <MemoryPane
            memories={appState.memories}
            isLoading={appState.isLoadingMemories}
            activeAgent={activeAgent}
            onSetAgentMemoryMode={appStore.setAgentMemoryMode}
            onProposeMemory={appStore.proposeMemory}
            onConfirmMemory={appStore.confirmMemory}
            onRejectMemory={appStore.rejectMemory}
            onCorrectMemory={appStore.correctMemory}
            onForgetMemory={appStore.forgetMemory}
          />
        </main>
      ) : currentThread && store ? (
        <main className={styles.workbench}>
          <ThreadHeader
            title={currentThread.title}
            agent={currentThread.agent}
            agents={appState.agents}
            selectedAgentId={
              appState.agents.find((a) => a.name === currentThread.agent || a.id === currentThread.agent)?.id ??
              appState.selectedAgentId
            }
            onSelectAgent={(agentId) => appStore.selectAgent(agentId)}
            onAddAgent={() => appStore.openOnboarding(true)}
            theme={ui.theme}
            onToggleTheme={uiStore.toggleTheme}
            inspectorOpen={ui.inspectorOpen}
            onToggleInspector={() => uiStore.setInspectorOpen(!ui.inspectorOpen)}
            isStreaming={isCurrentThreadStreaming}
            onCancel={onCancelTurn}
          />
          <Timeline
            store={store}
            focusedRowId={focusedRowId}
            onFocusChange={setFocusedRowId}
            onPermissionDecision={onPermissionDecision}
            anchorRowId={ui.anchors[currentThread.id] ?? null}
            onFirstVisibleChange={onFirstVisibleChange}
            viewportHeight={timelineViewportHeight}
            ariaLabel={getDictionary(ui.locale).timeline.threadTimelineAria(currentThread.title)}
          />
          {currentThreadActiveTurn?.notice ? (
            <p
              className={styles.emptyState}
              role="status"
              data-testid="turn-notice"
              style={{ margin: "0 auto", padding: "4px 16px" }}
            >
              {currentThreadActiveTurn.notice}
            </p>
          ) : null}
          <Composer
            draft={ui.drafts[currentThread.id] ?? ""}
            onDraftChange={(text) => uiStore.setDraft(currentThread.id, text)}
            onSend={onSend}
            onCancel={onCancelTurn}
            isStreaming={isCurrentThreadStreaming}
            cancelPending={currentThreadActiveTurn?.cancelState === "requested"}
            disabledReason={
              appState.connectionStatus === "disconnected"
                ? getDictionary(ui.locale).workbenchEmpty.coreDisconnected
                : null
            }
          />
        </main>
      ) : (
        <main className={styles.workbench} data-testid="thread-empty">
          {appState.connectionStatus === "connecting" ? (
            <p className={styles.emptyState} role="status">
              {getDictionary(ui.locale).workbenchEmpty.connecting}
            </p>
          ) : appState.error ? (
            <p className={styles.emptyState} role="alert" data-testid="empty-state-error">
              {appState.error}
            </p>
          ) : appState.threads.length === 0 ? (
            <p className={styles.emptyState} role="status" data-testid="empty-state-none">
              {getDictionary(ui.locale).workbenchEmpty.noConversations}
            </p>
          ) : (
            <p className={styles.emptyState} role="status">
              {getDictionary(ui.locale).workbenchEmpty.selectConversation}
            </p>
          )}
        </main>
      )}

      {currentThread && ui.inspectorOpen ? (
        <>
          {narrow && (
            <div
              className={styles.drawerOverlay}
              onClick={() => uiStore.setInspectorOpen(false)}
              data-testid="inspector-backdrop"
            />
          )}
          <div className={styles.inspectorArea}>
            <Inspector
              width={ui.inspectorWidth}
              onWidthChange={uiStore.setInspectorWidth}
              onClose={() => uiStore.setInspectorOpen(false)}
              focusedRow={focusedRow}
              activeAgent={activeAgent}
              contextSnapshot={appState.contextSnapshot}
              contextSnapshotStatus={appState.contextSnapshotStatus}
              contextSnapshotError={appState.contextSnapshotError}
              diagnostics={appState.runtimeDiagnostics}
              onRefreshDiagnostics={() => void appStore.getDiagnostics()}
            />
          </div>
        </>
      ) : null}

      <div className={styles.statusBarArea}>
        <StatusBar
          coreState={appState.negotiated ? "connected" : appState.connectionStatus}
          ipcVersion={appState.negotiated?.selected_version}
          threadStatus={currentThread?.status ?? "completed"}
          streamState={appState.streamState !== "idle" ? appState.streamState : undefined}
          onReconnect={() => void appStore.reconnect()}
        />
      </div>

      {/* Agent Onboarding Modal */}
      <AgentOnboardingModal
        isOpen={appState.isOnboardingOpen}
        onClose={() => appStore.openOnboarding(false)}
        onSave={async (data) => {
          await appStore.onboardAgent(data);
        }}
        onTest={async (data) => {
          return await appStore.testAgent(data);
        }}
        isTesting={appState.onboardingStatus.isTesting}
        testResult={appState.onboardingStatus.testResult}
      />

      <SettingsModal
        isOpen={ui.settingsOpen}
        onClose={() => uiStore.setSettingsOpen(false)}
        themeSource={ui.themeSource}
        onThemeSourceChange={uiStore.setThemeSource}
        localeSource={ui.localeSource}
        onLocaleSourceChange={uiStore.setLocaleSource}
        diagnostics={appState.runtimeDiagnostics}
        onRefreshDiagnostics={() => void appStore.getDiagnostics()}
      />

      {/* Headless protocol stream evidence container (removed from visual grid layout per ADR 0008) */}
      <div data-testid="protocol-diagnostics" style={{ display: "none" }} aria-hidden="true">
        {appState.negotiated ? (
          <ul>
            {Object.entries(appState.negotiated.negotiated_capabilities).map(([id, support]) => (
              <li key={id}>{id}: {String(support)}</li>
            ))}
          </ul>
        ) : null}
        <ol>
          {appState.streamLog.map((entry) => (
            <li key={`${entry.sequence}-${entry.event_id}`}>
              #{entry.sequence} {entry.label} {entry.diagnostic ? <code>{entry.diagnostic}</code> : null}
            </li>
          ))}
        </ol>
      </div>
    </div>
    </I18nProvider>
  );
}
