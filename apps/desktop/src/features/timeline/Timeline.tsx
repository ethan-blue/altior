/**
 * Virtualized streaming timeline (ADR 0008 §3–4).
 *
 * The window math is `virtualWindow.ts` (pure); this component owns
 * scroll state, follow-tail, the new-activity affordance, scroll
 * restoration, row-height measurement, and roving-tabindex keyboard
 * navigation. `viewportHeight` and `measured` are explicit injection
 * points because jsdom has no layout — DOM tests assert wiring, the
 * math tests assert geometry.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { TimelineRowView } from "./TimelineRowView";
import type { PermissionDecision, TimelineRow, TimelineStore } from "./timelineStore";
import {
  buildHeightIndex,
  isAtEnd,
  offsetOf,
  restoreScroll,
  rowAtOffset,
  windowFor,
  type HeightIndex,
} from "./virtualWindow";
import styles from "./timeline.module.css";

export interface TimelineProps {
  readonly store: TimelineStore;
  /** The keyboard-focused row (roving tabindex owner). */
  readonly focusedRowId: string | null;
  readonly onFocusChange: (id: string | null) => void;
  readonly onPermissionDecision: (id: string, decision: PermissionDecision) => void;
  /** Restore the scroll so this row is first visible (thread reopen). */
  readonly anchorRowId: string | null;
  /** Reports the first visible row so navigation state can persist it. */
  readonly onFirstVisibleChange?: (rowId: string) => void;
  /** Test injection: pixel height of the scroll viewport. */
  readonly viewportHeight?: number;
  /** Test injection: measured pixel heights by row id. */
  readonly measured?: ReadonlyMap<string, number>;
  readonly overscan?: number;
  readonly ariaLabel?: string;
}

const DEFAULT_VIEWPORT = 600;
const DEFAULT_OVERSCAN = 6;

/** Estimated row height per kind until measurement corrects it. */
function estimateRowHeight(row: TimelineRow): number {
  switch (row.kind) {
    case "permission":
      return 88;
    case "tool":
    case "unknown":
      return 34;
    case "error":
      return 44;
    default:
      return Math.max(28, 22 + row.text.length * 0.55);
  }
}

export function Timeline({
  store,
  focusedRowId,
  onFocusChange,
  onPermissionDecision,
  anchorRowId,
  onFirstVisibleChange,
  viewportHeight,
  measured,
  overscan = DEFAULT_OVERSCAN,
  ariaLabel = "Conversation timeline",
}: TimelineProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const { structureVersion } = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  const [scrollTop, setScrollTop] = useState(0);
  const [liveViewport, setLiveViewport] = useState(0);
  const [stickToTail, setStickToTail] = useState(true);
  const [missedRows, setMissedRows] = useState(0);
  const [announcement, setAnnouncement] = useState("");
  /** Heights measured from real layout (empty in jsdom). */
  const [browserMeasured, setBrowserMeasured] = useState<ReadonlyMap<string, number>>(
    () => new Map(),
  );

  const effectiveMeasured = useMemo(() => {
    if (browserMeasured.size === 0) return measured;
    const merged = new Map(measured);
    for (const [id, height] of browserMeasured) merged.set(id, height);
    return merged;
  }, [measured, browserMeasured]);

  const count = store.rowCount();
  const heightIndex: HeightIndex = useMemo(
    () =>
      buildHeightIndex(count, (index) => {
        const row = store.getRowByIndex(index);
        if (!row) return 32;
        const height = effectiveMeasured?.get(row.id);
        return height && height > 0 ? height : estimateRowHeight(row);
      }),
    // Rebuilt only when the row structure or a height correction lands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [structureVersion, count, store, effectiveMeasured],
  );

  const viewHeight = viewportHeight ?? (liveViewport > 0 ? liveViewport : DEFAULT_VIEWPORT);
  const view = windowFor(heightIndex, scrollTop, viewHeight, overscan);
  const atEnd = isAtEnd(heightIndex, scrollTop, viewHeight);

  // Measure the viewport in real browsers; tests inject `viewportHeight`
  // (jsdom has neither layout nor ResizeObserver, and the estimate path
  // already covers us there).
  useLayoutEffect(() => {
    if (viewportHeight != null || !containerRef.current) return;
    const element = containerRef.current;
    const update = () => setLiveViewport(element.clientHeight || 0);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [viewportHeight]);

  // Measure rendered rows after every paint; converges (no update when
  // heights match) and stays silent in jsdom where layout is 0.
  useLayoutEffect(() => {
    const scroller = containerRef.current;
    if (!scroller) return;
    const updates = new Map<string, number>();
    for (const element of scroller.querySelectorAll<HTMLElement>("[data-row-id]")) {
      const height = element.getBoundingClientRect().height;
      if (height <= 0) continue;
      const id = element.dataset.rowId;
      if (!id) continue;
      const known = effectiveMeasured?.get(id);
      if (known == null || Math.abs(known - height) > 1) {
        updates.set(id, height);
      }
    }
    if (updates.size > 0) {
      setBrowserMeasured((previous) => {
        const next = new Map(previous);
        for (const [id, height] of updates) next.set(id, height);
        return next;
      });
    }
  });

  const applyScroll = useCallback(
    (next: number) => {
      // Real browsers clamp scrollTop to `scrollHeight - clientHeight`;
      // jsdom does not, so the engine clamps explicitly.
      const maximum = Math.max(0, heightIndex.total - viewHeight);
      const clamped = Math.max(0, Math.min(next, maximum));
      if (containerRef.current) containerRef.current.scrollTop = clamped;
      setScrollTop(clamped);
    },
    [heightIndex, viewHeight],
  );

  // Structural changes: prepended history keeps the viewport anchored on
  // the first visible row (prepend stability); appends follow the tail
  // while pinned; otherwise they count toward the new-activity affordance.
  // Prepending is detected by the first row *changing*, never by the
  // anchor's index alone (the anchor is mid-list in the common case).
  const firstVisibleRef = useRef<string | null>(null);
  const firstRowIdRef = useRef<string | null>(null);
  useEffect(() => {
    const rows = store.getSnapshot().rows;
    const currentFirst = rows[0]?.id ?? null;
    const anchorId = firstVisibleRef.current;
    if (
      firstRowIdRef.current != null &&
      currentFirst !== firstRowIdRef.current &&
      anchorId != null
    ) {
      const index = rows.findIndex((row) => row.id === anchorId);
      if (index > 0) {
        // Older history landed above the viewport: shift by the exact
        // height of the prepended block so the anchor stays first visible.
        applyScroll(offsetOf(heightIndex, index));
        firstRowIdRef.current = currentFirst;
        return;
      }
    }
    firstRowIdRef.current = currentFirst;
    if (stickToTail) {
      applyScroll(heightIndex.total);
    } else {
      setMissedRows((seen) => seen + 1);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [structureVersion, store]);

  const onScroll = (event: React.UIEvent<HTMLDivElement>) => {
    const next = event.currentTarget.scrollTop;
    setScrollTop(next);
    if (isAtEnd(heightIndex, next, viewHeight)) {
      setStickToTail(true);
      setMissedRows(0);
    } else {
      setStickToTail(false);
    }
    const firstVisible = store.getRowByIndex(rowAtOffset(heightIndex, next));
    if (firstVisible) {
      firstVisibleRef.current = firstVisible.id;
      onFirstVisibleChange?.(firstVisible.id);
    }
  };

  // Restore the anchor row when switching threads. Tail-following must
  // re-evaluate for the restored position (jsdom fires no scroll event
  // for programmatic scrollTop).
  useEffect(() => {
    if (!anchorRowId) return;
    const index = store.getSnapshot().rows.findIndex((row) => row.id === anchorRowId);
    if (index >= 0) {
      const restored = restoreScroll(heightIndex, index);
      applyScroll(restored);
      firstVisibleRef.current = anchorRowId;
      setStickToTail(isAtEnd(heightIndex, restored, viewHeight));
      setMissedRows(0);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [anchorRowId]);

  const rowsInRange = useMemo(() => {
    const rendered: { row: TimelineRow; index: number; top: number }[] = [];
    for (let index = view.startIndex; index <= view.endIndex; index += 1) {
      const row = store.getRowByIndex(index);
      if (!row) continue;
      rendered.push({ row, index, top: offsetOf(heightIndex, index) });
    }
    return rendered;
  }, [view, heightIndex, store]);

  const focusRowByIndex = (index: number) => {
    const clamped = Math.max(0, Math.min(index, count - 1));
    const row = store.getRowByIndex(clamped);
    if (!row) return;
    onFocusChange(row.id);
    // Keep the focused row fully visible (bottom-align when it is
    // taller than the viewport).
    const top = offsetOf(heightIndex, clamped);
    const rowBottom = heightIndex.offsets[clamped + 1] ?? top;
    if (top < scrollTop) {
      applyScroll(top);
    } else if (rowBottom > scrollTop + viewHeight) {
      applyScroll(rowBottom - viewHeight);
    }
  };

  const focusIndex = () =>
    focusedRowId == null
      ? -1
      : store.getSnapshot().rows.findIndex((row) => row.id === focusedRowId);

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = focusIndex();
    const firstVisible = rowAtOffset(heightIndex, scrollTop);
    const page = Math.max(
      1,
      rowAtOffset(heightIndex, scrollTop + viewHeight) - firstVisible,
    );
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusRowByIndex(current < 0 ? 0 : current + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusRowByIndex(current < 0 ? 0 : Math.max(0, current - 1));
        break;
      case "PageDown":
        event.preventDefault();
        focusRowByIndex((current < 0 ? firstVisible : current) + page);
        break;
      case "PageUp":
        event.preventDefault();
        focusRowByIndex(Math.max(0, (current < 0 ? firstVisible : current) - page));
        break;
      case "Home":
        event.preventDefault();
        focusRowByIndex(0);
        break;
      case "End":
        event.preventDefault();
        focusRowByIndex(count - 1);
        break;
      default:
        break;
    }
  };

  return (
    <div className={styles.timelineShell}>
      <div
        ref={containerRef}
        className={styles.scroller}
        role="log"
        aria-label={ariaLabel}
        tabIndex={focusedRowId == null ? 0 : -1}
        onKeyDown={onKeyDown}
        onScroll={onScroll}
        data-testid="timeline-scroller"
        data-row-count={count}
        data-window-size={view.endIndex - view.startIndex + 1}
      >
        <div className={styles.spacer} style={{ height: heightIndex.total }}>
          {rowsInRange.map(({ row, index, top }) => (
            <div key={row.id} className={styles.slot} style={{ top }}>
              <TimelineRowView
                store={store}
                rowId={row.id}
                index={index}
                focused={focusedRowId === row.id}
                onFocus={onFocusChange}
                onPermissionDecision={(id, decision) => {
                  onPermissionDecision(id, decision);
                  setAnnouncement(
                    decision === "approved" ? "Permission approved" : "Permission denied",
                  );
                }}
              />
            </div>
          ))}
        </div>
      </div>
      {!atEnd && missedRows > 0 ? (
        <button
          type="button"
          className={styles.newActivity}
          data-testid="new-activity"
          onClick={() => {
            setStickToTail(true);
            setMissedRows(0);
            applyScroll(heightIndex.total);
          }}
        >
          {missedRows} new {missedRows === 1 ? "row" : "rows"} ↓
        </button>
      ) : null}
      <div className={styles.visuallyHidden} aria-live="polite" role="status">
        {announcement}
      </div>
    </div>
  );
}
