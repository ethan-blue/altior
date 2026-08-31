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
  StatusBar,
  ThreadHeader,
  ThreadsPane,
} from "../components/shell";
import { Timeline } from "../features/timeline/Timeline";
import type { PermissionDecision } from "../features/timeline/timelineStore";
import type { NegotiatedHandshake } from "../ipc/dto/NegotiatedHandshake";
import { createDefaultTransport } from "../ipc/tauriTransport";
import type { CoreTransport } from "../ipc/transport";
import { createApplicationStore } from "../stores/applicationStore";
import styles from "./App.module.css";
import { createUiStore, useUiState } from "./uiStore";

export interface AppProps {
  /** Transport to run against; defaults to the environment default transport factory. */
  readonly transport?: CoreTransport;
  /** Include the 100,000-row acceptance thread (acceptance runs only). */
  readonly includeHugeThread?: boolean;
  /** Test injection for the timeline viewport (jsdom has no layout). */
  readonly timelineViewportHeight?: number;
}

function capabilityList(negotiated: NegotiatedHandshake): string[] {
  return Object.entries(negotiated.negotiated_capabilities).map(
    ([id, support]) => `${id}: ${String(support)}`,
  );
}

export function App({
  transport,
  includeHugeThread = false,
  timelineViewportHeight,
}: AppProps) {
  const [resolvedTransport] = useState(() => transport ?? createDefaultTransport());
  const [appStore] = useState(() =>
    createApplicationStore(resolvedTransport, { includeHugeThread }),
  );
  const appState = useSyncExternalStore(
    appStore.subscribe,
    appStore.getState,
    appStore.getState,
  );

  const [uiStore] = useState(() =>
    createUiStore(appState.threads[0]?.id ?? "fixture/standard"),
  );
  const ui = useUiState(uiStore);

  const [focusedRowId, setFocusedRowId] = useState<string | null>(null);
  const [narrow, setNarrow] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Initialize transport connection on mount
  useEffect(() => {
    void appStore.init();
  }, [appStore]);

  const currentThread =
    appState.threads.find((t) => t.id === appState.selectedThreadId) ??
    appState.threads[0]!;
  const store = appStore.getTimelineStore(currentThread.id);
  const focusedRow = focusedRowId == null ? null : store.getRow(focusedRowId);
  const activeAgent =
    appState.agents.find((a) => a.id === appState.selectedAgentId) ??
    appState.agents[0];

  // Narrow-width detection for the inspector overlay drawer.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || typeof ResizeObserver === "undefined") return;
    const update = () => setNarrow(root.clientWidth > 0 && root.clientWidth < 900);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(root);
    return () => observer.disconnect();
  }, []);

  const onSend = useCallback(async () => {
    const text = (ui.drafts[currentThread.id] ?? "").trim();
    if (!text) return;
    uiStore.setDraft(currentThread.id, "");

    await appStore.sendPrompt(text);
  }, [appStore, currentThread.id, ui.drafts, uiStore]);

  const onCancelTurn = useCallback(async () => {
    await appStore.cancelActiveTurn();
  }, [appStore]);

  const onPermissionDecision = useCallback(
    async (id: string, decision: PermissionDecision) => {
      await appStore.decidePermission(id, decision);
    },
    [appStore],
  );

  const onFirstVisibleChange = useCallback(
    (rowId: string) => uiStore.setAnchor(currentThread.id, rowId),
    [currentThread.id, uiStore],
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
    const newThread = await appStore.createThread(
      `Thread ${appState.threads.length + 1}`,
      activeAgent?.id,
    );
    uiStore.selectThread(newThread.id);
    setFocusedRowId(null);
  }, [activeAgent?.id, appState.threads.length, appStore, uiStore]);

  const onActivityNavigate = useCallback(
    (dest: string) => {
      if (dest === "agents") {
        appStore.openOnboarding(true);
      }
    },
    [appStore],
  );

  const isCurrentThreadStreaming =
    appState.activeTurn != null &&
    appState.activeTurn.threadId === currentThread.id &&
    appState.activeTurn.isStreaming;

  return (
    <div
      ref={rootRef}
      className={styles.shell}
      data-theme={ui.theme}
      data-narrow={narrow}
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

      <ActivityRail active="threads" onNavigate={onActivityNavigate} />

      <div style={{ display: "flex", minWidth: 0 }}>
        <div style={{ width: ui.navWidth }}>
          <ThreadsPane
            threads={appState.threads}
            selectedThreadId={currentThread.id}
            onSelect={onSelectThread}
            filter={appState.threadFilter}
            onFilterChange={(f) => {
              void appStore.setThreadFilter(f);
            }}
            onCreateThread={onCreateThread}
          />
        </div>
        <NavResizeHandle
          width={ui.navWidth}
          onWidthChange={uiStore.setNavWidth}
        />
      </div>

      <main className={styles.workbench}>
        <ThreadHeader
          title={currentThread.title}
          agent={currentThread.agent}
          agents={appState.agents}
          onSelectAgent={(agentId) => appStore.selectAgent(agentId)}
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
          ariaLabel={`${currentThread.title} timeline`}
        />
        <Composer
          draft={ui.drafts[currentThread.id] ?? ""}
          onDraftChange={(text) => uiStore.setDraft(currentThread.id, text)}
          onSend={onSend}
          onCancel={onCancelTurn}
          isStreaming={isCurrentThreadStreaming}
          disabledReason={
            appState.connectionStatus === "disconnected"
              ? "Core disconnected"
              : null
          }
        />
      </main>

      {ui.inspectorOpen ? (
        <Inspector
          width={ui.inspectorWidth}
          onWidthChange={uiStore.setInspectorWidth}
          onClose={() => uiStore.setInspectorOpen(false)}
          focusedRow={focusedRow}
          activeAgent={activeAgent}
        />
      ) : null}

      <StatusBar
        coreState={
          appState.negotiated
            ? `connected (IPC v${appState.negotiated.selected_version})`
            : appState.connectionStatus
        }
        threadStatus={currentThread.status}
        streamState={appState.streamState !== "idle" ? appState.streamState : undefined}
        onReconnect={() => void appStore.reconnect()}
      />

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

      {/* Protocol diagnostics: the P0.1 evidence surface lives on. */}
      <div className={styles.protocolDiagnostics} data-testid="protocol-diagnostics">
        <details>
          <summary>Protocol stream ({appState.streamLog.length} events)</summary>
          {appState.negotiated ? (
            <ul>
              {capabilityList(appState.negotiated).map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          ) : null}
          <ol>
            {appState.streamLog.map((entry) => (
              <li key={`${entry.sequence}-${entry.event_id}`}>
                #{entry.sequence} {entry.label}
                {entry.diagnostic ? (
                  <>
                    {" "}
                    <code>{entry.diagnostic}</code>
                  </>
                ) : null}
              </li>
            ))}
          </ol>
        </details>
      </div>
    </div>
  );
}
