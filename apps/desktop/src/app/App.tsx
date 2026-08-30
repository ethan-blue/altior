/**
 * Workbench shell composition (P0.4, ADR 0008).
 *
 * The five stable regions from docs/UI_ARCHITECTURE.md compose here:
 * title bar, activity rail, threads pane, workbench (header, timeline,
 * composer), inspector, status bar. Renderer-owned state lives in
 * `uiStore`; Core-facing state flows through the transport as in P0.1.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ActivityRail,
  Composer,
  Inspector,
  NavResizeHandle,
  StatusBar,
  ThreadHeader,
  ThreadsPane,
} from "../components/shell";
import {
  allThreads,
  streamingReplyChunks,
  type ThreadFixture,
} from "../fixtures/timeline";
import { Timeline } from "../features/timeline/Timeline";
import {
  createTimelineStore,
  type PermissionDecision,
  type TimelineStore,
} from "../features/timeline/timelineStore";
import type { NegotiatedHandshake } from "../ipc/dto/NegotiatedHandshake";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import type { DesktopTransport } from "../ipc/transport";
import styles from "./App.module.css";
import { createUiStore, useUiState } from "./uiStore";

export interface AppProps {
  /** Transport to run against; defaults to the in-memory fixture shell. */
  readonly transport?: DesktopTransport;
  /** Include the 100,000-row acceptance thread (acceptance runs only). */
  readonly includeHugeThread?: boolean;
  /** Test injection for the timeline viewport (jsdom has no layout). */
  readonly timelineViewportHeight?: number;
}

/** Bounded view of the raw protocol stream for the inspector. */
const MAX_STREAM_LOG = 50;

function capabilityList(negotiated: NegotiatedHandshake): string[] {
  return Object.entries(negotiated.negotiated_capabilities).map(
    ([id, support]) => `${id}: ${String(support)}`,
  );
}

export function App({
  transport = new InMemoryTransport(),
  includeHugeThread = false,
  timelineViewportHeight,
}: AppProps) {
  const [uiStore] = useState(() => createUiStore(allThreads(false)[0]!.id));
  const ui = useUiState(uiStore);
  const threads = useMemo(() => allThreads(includeHugeThread), [includeHugeThread]);
  const [threadFilter, setThreadFilter] = useState("");
  const [focusedRowId, setFocusedRowId] = useState<string | null>(null);
  const [negotiated, setNegotiated] = useState<NegotiatedHandshake | null>(null);
  const [streamLog, setStreamLog] = useState<
    { sequence: number; label: string; diagnostic: string | null }[]
  >([]);
  const [narrow, setNarrow] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Per-thread timeline stores survive thread navigation (P0.1-style
  // fixture core: thread state is per-thread, not per-mount).
  const storesRef = useRef(new Map<string, TimelineStore>());
  const storeFor = useCallback((thread: ThreadFixture): TimelineStore => {
    let store = storesRef.current.get(thread.id);
    if (!store) {
      store = createTimelineStore(thread.rows);
      storesRef.current.set(thread.id, store);
    }
    return store;
  }, []);

  const currentThread =
    threads.find((thread) => thread.id === ui.selectedThreadId) ?? threads[0]!;
  const store = storeFor(currentThread);
  const focusedRow = focusedRowId == null ? null : store.getRow(focusedRowId);

  // Handshake + subscription, exactly as in the P0.1 shell.
  useEffect(() => {
    let active = true;
    const unsubscribe = transport.subscribe((event) => {
      setStreamLog((previous) => {
        const body = event.body;
        const diagnostic = "diagnostic" in body ? body.diagnostic : null;
        const next = [
          ...previous,
          { sequence: event.sequence, label: body.kind, diagnostic },
        ];
        return next.length > MAX_STREAM_LOG ? next.slice(-MAX_STREAM_LOG) : next;
      });
    });
    void transport.handshake().then((result) => {
      if (active) setNegotiated(result);
    });
    return () => {
      active = false;
      unsubscribe();
    };
  }, [transport]);

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

  const sendSeq = useRef(0);
  const onSend = useCallback(() => {
    const text = (ui.drafts[currentThread.id] ?? "").trim();
    if (!text) return;
    sendSeq.current += 1;
    const turn = sendSeq.current;
    uiStore.setDraft(currentThread.id, "");
    store.appendRow({
      id: `send-${turn}`,
      kind: "user-message",
      text,
      status: null,
      permission: null,
      streaming: false,
    });
    const replyId = `send-${turn}-reply`;
    store.appendRow({
      id: replyId,
      kind: "assistant-message",
      text: "",
      status: null,
      permission: null,
      streaming: true,
    });
    // The fixture stream is deterministic and synchronous; the real
    // streaming runtime arrives with P1.2.
    for (const chunk of streamingReplyChunks) store.appendDelta(replyId, chunk);
    store.finishStreaming(replyId);
  }, [currentThread.id, store, ui.drafts, uiStore]);

  const onPermissionDecision = useCallback(
    (id: string, decision: PermissionDecision) => {
      store.setPermissionDecision(id, decision);
    },
    [store],
  );

  const onFirstVisibleChange = useCallback(
    (rowId: string) => uiStore.setAnchor(currentThread.id, rowId),
    [currentThread.id, uiStore],
  );

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
          {negotiated ? `IPC v${negotiated.selected_version}` : "IPC connecting…"}
        </span>
      </header>

      <ActivityRail active="threads" />

      <div style={{ display: "flex", minWidth: 0 }}>
        <div style={{ width: ui.navWidth }}>
          <ThreadsPane
            threads={threads}
            selectedThreadId={currentThread.id}
            onSelect={(id) => {
              uiStore.selectThread(id);
              setFocusedRowId(null);
            }}
            filter={threadFilter}
            onFilterChange={setThreadFilter}
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
          theme={ui.theme}
          onToggleTheme={uiStore.toggleTheme}
          inspectorOpen={ui.inspectorOpen}
          onToggleInspector={() => uiStore.setInspectorOpen(!ui.inspectorOpen)}
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
          disabledReason={null}
        />
      </main>

      {ui.inspectorOpen ? (
        <Inspector
          width={ui.inspectorWidth}
          onWidthChange={uiStore.setInspectorWidth}
          onClose={() => uiStore.setInspectorOpen(false)}
          focusedRow={focusedRow}
        />
      ) : null}

      <StatusBar
        coreState={
          negotiated ? `connected (IPC v${negotiated.selected_version})` : "connecting"
        }
        threadStatus={currentThread.status}
      />

      {/* Protocol diagnostics: the P0.1 evidence surface lives on. */}
      <div className={styles.protocolDiagnostics} data-testid="protocol-diagnostics">
        <details>
          <summary>Protocol stream ({streamLog.length} events)</summary>
          {negotiated ? (
            <ul>
              {capabilityList(negotiated).map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          ) : null}
          <ol>
            {streamLog.map((entry) => (
              <li key={entry.sequence}>
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
