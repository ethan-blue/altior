/**
 * Stream deduplication, epoch tracking, and bounded cache structures (ADR 0021, review A06).
 *
 * Guarantees that:
 * 1. Event IDs and sequence numbers do not leak memory under long-running
 *    streams (capped memory footprint over 1,000,000+ events).
 * 2. Core restarts (new `core_instance_id` / epoch) cleanly transition sequence
 *    spaces so seq=1 from a new Core is never dropped as a stale duplicate.
 * 3. In-memory timeline stores are bounded with an LRU eviction policy that
 *    strictly pins the active working set (selected thread + active turns).
 */
import type { TimelineStore } from "../features/timeline/timelineStore";

/**
 * Fixed-capacity FIFO set for deduplicating event IDs without unbounded growth.
 */
export class BoundedIdSet {
  private readonly capacity: number;
  private readonly set = new Set<string>();
  private readonly fifo: string[] = [];

  constructor(capacity: number = 4096) {
    this.capacity = capacity;
  }

  has(id: string): boolean {
    return this.set.has(id);
  }

  add(id: string): void {
    if (this.set.has(id)) return;
    if (this.fifo.length >= this.capacity) {
      const oldest = this.fifo.shift();
      if (oldest !== undefined) {
        this.set.delete(oldest);
      }
    }
    this.fifo.push(id);
    this.set.add(id);
  }

  clear(): void {
    this.set.clear();
    this.fifo.length = 0;
  }

  get size(): number {
    return this.set.size;
  }
}

/**
 * Epoch-aware, sliding-window sequence tracker.
 *
 * Deduplicates sequence numbers within `[highWater - windowSize, highWater]`.
 * Sequences older than the window are treated as expired duplicates.
 * An epoch change (Core restart) resets the sequence window safely.
 */
export class EpochSequenceTracker {
  private currentEpoch: string | null = null;
  private highWater: number = 0;
  private readonly windowSize: number;
  private readonly windowSeqs = new Set<number>();

  constructor(windowSize: number = 2048) {
    this.windowSize = windowSize;
  }

  get epoch(): string | null {
    return this.currentEpoch;
  }

  get highWaterSequence(): number {
    return this.highWater;
  }

  /**
   * Transitions to a new Core instance epoch. Clears the sequence window
   * so sequence 1 in the new epoch is processed immediately.
   */
  transitionEpoch(newEpoch: string): void {
    this.currentEpoch = newEpoch;
    this.highWater = 0;
    this.windowSeqs.clear();
  }

  /**
   * Checks if an event sequence is a duplicate.
   *
   * If `eventEpoch` is provided and does not match `currentEpoch`:
   * - If no epoch is set yet, sets it.
   * - If an epoch mismatch occurs, callers must decide whether to transition.
   */
  isDuplicate(seq: number, eventEpoch?: string | null): boolean {
    if (eventEpoch && this.currentEpoch && eventEpoch !== this.currentEpoch) {
      // Event belongs to a different epoch
      return false;
    }
    // Expired: strictly older than the sliding window
    if (this.highWater > this.windowSize && seq <= this.highWater - this.windowSize) {
      return true;
    }
    return this.windowSeqs.has(seq);
  }

  /**
   * Records a sequence number within the current epoch and slides the window.
   * Amortized O(1) for monotonic sequences and bounded jitter.
   */
  record(seq: number, eventEpoch?: string | null): void {
    if (eventEpoch && (!this.currentEpoch || eventEpoch !== this.currentEpoch)) {
      this.transitionEpoch(eventEpoch);
    }
    const currentCutoff = this.highWater > this.windowSize ? this.highWater - this.windowSize : 0;
    if (currentCutoff > 0 && seq <= currentCutoff) {
      // Sequence is strictly older than the sliding window; expired duplicate.
      return;
    }

    this.windowSeqs.add(seq);

    if (seq > this.highWater) {
      const prevHighWater = this.highWater;
      this.highWater = seq;
      const newCutoff = this.highWater - this.windowSize;
      if (newCutoff > 0) {
        const prevCutoff = prevHighWater > this.windowSize ? prevHighWater - this.windowSize : 0;
        const delta = this.highWater - prevHighWater;
        if (delta < this.windowSize) {
          // Monotonic or small step: prune only the newly evicted range in O(delta)
          const start = Math.max(1, prevCutoff + 1);
          for (let s = start; s <= newCutoff; s++) {
            this.windowSeqs.delete(s);
          }
        } else {
          // Large gap jump (delta >= windowSize): everything before the jump is strictly <= newCutoff
          this.windowSeqs.clear();
          this.windowSeqs.add(seq);
        }
      }
    }
  }

  get size(): number {
    return this.windowSeqs.size;
  }

  clear(): void {
    this.highWater = 0;
    this.windowSeqs.clear();
  }
}

/**
 * Bounded LRU cache for `TimelineStore` instances.
 *
 * Evicts idle, unpinned conversation stores when capacity is reached.
 * Pinned conversations (selected thread or threads with active turns) are
 * exempt from eviction to prevent UI state loss.
 */
export class BoundedTimelineCache {
  private readonly capacity: number;
  private readonly cache = new Map<string, TimelineStore>();
  private readonly lruOrder: string[] = [];

  constructor(capacity: number = 20) {
    this.capacity = capacity;
  }

  get(threadId: string): TimelineStore | undefined {
    const store = this.cache.get(threadId);
    if (store) {
      this.touch(threadId);
    }
    return store;
  }

  has(threadId: string): boolean {
    return this.cache.has(threadId);
  }

  getOrCreate(
    threadId: string,
    createFn: () => TimelineStore,
    isPinned: (id: string) => boolean,
  ): TimelineStore {
    let store = this.cache.get(threadId);
    if (!store) {
      this.evictIfNecessary(isPinned);
      store = createFn();
      this.cache.set(threadId, store);
    }
    this.touch(threadId);
    return store;
  }

  set(
    threadId: string,
    store: TimelineStore,
    isPinned: (id: string) => boolean,
  ): void {
    if (!this.cache.has(threadId)) {
      this.evictIfNecessary(isPinned);
    }
    this.cache.set(threadId, store);
    this.touch(threadId);
  }

  private touch(threadId: string): void {
    const idx = this.lruOrder.indexOf(threadId);
    if (idx >= 0) {
      this.lruOrder.splice(idx, 1);
    }
    this.lruOrder.push(threadId);
  }

  private evictIfNecessary(isPinned: (id: string) => boolean): void {
    while (this.cache.size >= this.capacity) {
      let evicted = false;
      for (let i = 0; i < this.lruOrder.length; i++) {
        const candidate = this.lruOrder[i]!;
        if (!isPinned(candidate)) {
          this.lruOrder.splice(i, 1);
          this.cache.delete(candidate);
          evicted = true;
          break;
        }
      }
      if (!evicted) {
        // All cached stores are pinned; cannot evict further
        break;
      }
    }
  }

  get size(): number {
    return this.cache.size;
  }

  clear(): void {
    this.cache.clear();
    this.lruOrder.length = 0;
  }
}
