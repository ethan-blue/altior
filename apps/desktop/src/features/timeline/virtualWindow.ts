/**
 * Pure virtualization math for the timeline (ADR 0008).
 *
 * This module has no DOM imports: the React layer feeds it heights and
 * scroll state, and jsdom tests assert the math directly because jsdom
 * has no layout. Row heights come from measurement when a row has been
 * rendered and from an estimate otherwise; the prefix-sum index is
 * rebuilt only when the row structure changes, not for scroll events.
 */

/** A prefix-sum index over row heights, in pixels. */
export interface HeightIndex {
  /** `offsets[i]` is the top of row `i`; `offsets[count]` is the total. */
  readonly offsets: readonly number[];
  readonly count: number;
  readonly total: number;
}

/** The rows a viewport should render, plus the padding around them. */
export interface VirtualWindow {
  /** Index of the first rendered row (inclusive). */
  readonly startIndex: number;
  /** Index of the last rendered row (inclusive). */
  readonly endIndex: number;
  /** Height of the spacer above `startIndex`. */
  readonly padTop: number;
  /** Height of the spacer below `endIndex`. */
  readonly padBottom: number;
}

/**
 * Builds the prefix-sum index. `heightOf` receives each row index and
 * returns a positive pixel height (measured or estimated).
 */
export function buildHeightIndex(
  count: number,
  heightOf: (index: number) => number,
): HeightIndex {
  const offsets = new Array<number>(count + 1);
  offsets[0] = 0;
  for (let index = 0; index < count; index += 1) {
    const height = heightOf(index);
    if (!(height > 0)) {
      throw new Error(`row ${index} has a non-positive height: ${String(height)}`);
    }
    offsets[index + 1] = (offsets[index] ?? 0) + height;
  }
  return { offsets, count, total: offsets[count] ?? 0 };
}

/**
 * Appends a new row's height to an existing HeightIndex in O(1).
 */
export function appendRowHeight(index: HeightIndex, height: number): HeightIndex {
  if (!(height > 0)) {
    throw new Error(`row has a non-positive height: ${String(height)}`);
  }
  const offsets = index.offsets.slice();
  offsets.push(index.total + height);
  return {
    offsets,
    count: index.count + 1,
    total: offsets[offsets.length - 1] ?? index.total + height,
  };
}

/**
 * Adjusts an existing row's height by a delta in O(count - rowIndex).
 */
export function adjustRowHeight(
  index: HeightIndex,
  rowIndex: number,
  delta: number,
): HeightIndex {
  if (delta === 0 || rowIndex < 0 || rowIndex >= index.count) {
    return index;
  }
  const offsets = index.offsets.slice();
  for (let i = rowIndex + 1; i <= index.count; i += 1) {
    offsets[i] = (offsets[i] ?? 0) + delta;
  }
  return {
    offsets,
    count: index.count,
    total: offsets[index.count] ?? (index.total + delta),
  };
}

/** The top offset of a row. */
export function offsetOf(index: HeightIndex, row: number): number {
  if (row < 0 || row > index.count) {
    throw new Error(`row ${row} is outside 0..${index.count}`);
  }
  return index.offsets[row] ?? 0;
}

/**
 * The row containing a pixel offset (the first row when `offset` sits in
 * the padding above row 0).
 */
export function rowAtOffset(index: HeightIndex, offset: number): number {
  if (index.count === 0) {
    return 0;
  }
  const clamped = Math.max(0, Math.min(offset, index.total));
  // Binary search for the last offset that is <= clamped.
  let low = 0;
  let high = index.count;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if ((index.offsets[mid] ?? 0) <= clamped) {
      low = mid;
    } else {
      high = mid - 1;
    }
  }
  return Math.min(low, index.count - 1);
}

/**
 * Computes the render window for a viewport. `overscan` rows are added
 * on both sides so fast scrolling does not flash spacers.
 */
export function windowFor(
  index: HeightIndex,
  scrollTop: number,
  viewportHeight: number,
  overscan: number,
): VirtualWindow {
  if (index.count === 0) {
    return { startIndex: 0, endIndex: -1, padTop: 0, padBottom: 0 };
  }
  if (!(viewportHeight > 0)) {
    throw new Error(`viewport height must be positive: ${String(viewportHeight)}`);
  }
  const clampedTop = Math.max(0, Math.min(scrollTop, index.total));
  const firstVisible = rowAtOffset(index, clampedTop);
  const lastVisible = rowAtOffset(index, clampedTop + viewportHeight - 1);
  const startIndex = Math.max(0, firstVisible - overscan);
  const endIndex = Math.min(index.count - 1, lastVisible + overscan);
  return {
    startIndex,
    endIndex,
    padTop: index.offsets[startIndex] ?? 0,
    padBottom: index.total - (index.offsets[endIndex + 1] ?? index.total),
  };
}

/**
 * The scroll-top adjustment that keeps the viewport anchored after rows
 * are prepended: the exact measured height of the prepended block. This
 * is what makes history loading prepend-stable (ADR 0008 §3).
 */
export function prependShift(prependHeights: readonly number[]): number {
  return prependHeights.reduce((sum, height) => sum + height, 0);
}

/**
 * Whether the viewport is pinned to the end of the list within
 * `threshold` pixels (the follow-tail condition).
 */
export function isAtEnd(
  index: HeightIndex,
  scrollTop: number,
  viewportHeight: number,
  threshold = 24,
): boolean {
  return scrollTop + viewportHeight >= index.total - threshold;
}

/**
 * Restores the scroll position so that `row` is the first fully visible
 * row — the scroll-anchoring half of reopen-restores-scroll
 * (`docs/UI_ARCHITECTURE.md` Threads).
 */
export function restoreScroll(index: HeightIndex, row: number): number {
  return offsetOf(index, row);
}
