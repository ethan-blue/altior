import { describe, expect, it } from "vitest";
import {
  BoundedIdSet,
  BoundedTimelineCache,
  EpochSequenceTracker,
} from "./streamDeduplicator";
import { createTimelineStore } from "../features/timeline/timelineStore";

describe("BoundedIdSet (A06)", () => {
  it("bounds memory capacity to specified limit and evicts oldest FIFO", () => {
    const set = new BoundedIdSet(5);
    for (let i = 1; i <= 5; i++) {
      set.add(`evt_${i}`);
    }
    expect(set.size).toBe(5);
    expect(set.has("evt_1")).toBe(true);

    // Adding 6th element evicts evt_1
    set.add("evt_6");
    expect(set.size).toBe(5);
    expect(set.has("evt_1")).toBe(false);
    expect(set.has("evt_6")).toBe(true);

    // Duplicates do not increase size or cause eviction
    set.add("evt_6");
    expect(set.size).toBe(5);
    expect(set.has("evt_2")).toBe(true);
  });

  it("handles 100,000 events without growing beyond capacity", () => {
    const set = new BoundedIdSet(100);
    for (let i = 0; i < 100_000; i++) {
      set.add(`evt_${i}`);
    }
    expect(set.size).toBe(100);
    expect(set.has("evt_99999")).toBe(true);
    expect(set.has("evt_1")).toBe(false);
  });
});

describe("EpochSequenceTracker (A06)", () => {
  it("deduplicates within same epoch and slides the sequence window", () => {
    const tracker = new EpochSequenceTracker(10);
    tracker.transitionEpoch("cor_epoch0000000001");

    expect(tracker.isDuplicate(1)).toBe(false);
    tracker.record(1);
    expect(tracker.isDuplicate(1)).toBe(true);

    // Record sequences up to 25
    for (let i = 2; i <= 25; i++) {
      tracker.record(i);
    }
    expect(tracker.highWaterSequence).toBe(25);
    // Sequences <= highWater - 10 (i.e. <= 15) are expired/duplicates
    expect(tracker.isDuplicate(5)).toBe(true);
    // Recent sequence is duplicate
    expect(tracker.isDuplicate(24)).toBe(true);
    // Unseen recent sequence is not duplicate
    expect(tracker.isDuplicate(26)).toBe(false);

    // Internal set size never exceeds windowSize + 1
    expect(tracker.size).toBeLessThanOrEqual(11);
  });

  it("handles Core restart: old epoch seq=100 followed by new epoch seq=1", () => {
    const tracker = new EpochSequenceTracker(100);
    tracker.transitionEpoch("cor_first_launch0001");

    for (let i = 1; i <= 100; i++) {
      tracker.record(i);
    }
    expect(tracker.highWaterSequence).toBe(100);
    expect(tracker.isDuplicate(1)).toBe(true);

    // Core daemon restarts: new instance ID / epoch
    tracker.transitionEpoch("cor_second_launch002");
    expect(tracker.epoch).toBe("cor_second_launch002");
    expect(tracker.highWaterSequence).toBe(0);

    // Sequence 1 in the new epoch MUST NOT be dropped as a duplicate!
    expect(tracker.isDuplicate(1)).toBe(false);
    tracker.record(1);
    expect(tracker.isDuplicate(1)).toBe(true);
    expect(tracker.highWaterSequence).toBe(1);
  });

  it("handles jittered and out-of-order arrival within the sliding window", () => {
    const tracker = new EpochSequenceTracker(10);
    tracker.transitionEpoch("cor_jitter0001");

    // Arrive out of order: 1, 3, 2, 5, 4
    for (const seq of [1, 3, 2, 5, 4]) {
      expect(tracker.isDuplicate(seq)).toBe(false);
      tracker.record(seq);
      expect(tracker.isDuplicate(seq)).toBe(true);
    }
    expect(tracker.highWaterSequence).toBe(5);
    expect(tracker.size).toBe(5);

    // Advance window so 1 and 2 fall out (cutoff = 12 - 10 = 2)
    tracker.record(12);
    expect(tracker.isDuplicate(1)).toBe(true); // Expired
    expect(tracker.isDuplicate(2)).toBe(true); // Expired
    expect(tracker.isDuplicate(3)).toBe(true); // Still in window
    expect(tracker.isDuplicate(4)).toBe(true); // Still in window
    expect(tracker.isDuplicate(5)).toBe(true); // Still in window
    expect(tracker.isDuplicate(6)).toBe(false); // Unseen
  });

  it("handles large sequence gap jumps without leaking unbounded state", () => {
    const tracker = new EpochSequenceTracker(2048);
    tracker.transitionEpoch("cor_gap0001");

    for (let i = 1; i <= 10; i++) {
      tracker.record(i);
    }
    expect(tracker.highWaterSequence).toBe(10);

    // Jump far ahead: delta (499,990) >> windowSize (2048)
    tracker.record(500_000);
    expect(tracker.highWaterSequence).toBe(500_000);
    expect(tracker.size).toBeLessThanOrEqual(2048);

    // Old sequence is expired duplicate
    expect(tracker.isDuplicate(10)).toBe(true);
    // Jump target is recorded duplicate
    expect(tracker.isDuplicate(500_000)).toBe(true);
    // Unseen sequence within window is not duplicate
    expect(tracker.isDuplicate(499_999)).toBe(false);
  });

  it("does not pollute internal window set when recording expired sequences", () => {
    const tracker = new EpochSequenceTracker(100);
    tracker.transitionEpoch("cor_expired0001");

    for (let i = 1; i <= 200; i++) {
      tracker.record(i);
    }
    // Highwater = 200, windowSize = 100, cutoff = 100
    const sizeBefore = tracker.size;
    expect(tracker.isDuplicate(50)).toBe(true);

    // Recording an expired sequence must not inflate the window set
    tracker.record(50);
    expect(tracker.size).toBe(sizeBefore);
  });

  it("processes 1,000,000 synthetic events with bounded O(1) memory footprint", () => {
    const tracker = new EpochSequenceTracker(2048);
    tracker.transitionEpoch("cor_benchmark000001");

    for (let i = 1; i <= 1_000_000; i++) {
      tracker.record(i);
    }

    expect(tracker.highWaterSequence).toBe(1_000_000);
    expect(tracker.size).toBeLessThanOrEqual(2049);
  });
});

describe("BoundedTimelineCache (A06)", () => {
  it("evicts unpinned entries when capacity is exceeded", () => {
    const cache = new BoundedTimelineCache(3);
    const isPinned = () => false;

    cache.set("thr_1", createTimelineStore(), isPinned);
    cache.set("thr_2", createTimelineStore(), isPinned);
    cache.set("thr_3", createTimelineStore(), isPinned);
    expect(cache.size).toBe(3);

    // Adding thr_4 evicts the oldest untouched entry (thr_1)
    cache.set("thr_4", createTimelineStore(), isPinned);
    expect(cache.size).toBe(3);
    expect(cache.has("thr_1")).toBe(false);
    expect(cache.has("thr_2")).toBe(true);
    expect(cache.has("thr_3")).toBe(true);
    expect(cache.has("thr_4")).toBe(true);
  });

  it("never evicts pinned working set (selected thread or active turns)", () => {
    const cache = new BoundedTimelineCache(3);
    const pinnedIds = new Set(["thr_pinned_active"]);
    const isPinned = (id: string) => pinnedIds.has(id);

    cache.set("thr_pinned_active", createTimelineStore(), isPinned);
    cache.set("thr_idle_1", createTimelineStore(), isPinned);
    cache.set("thr_idle_2", createTimelineStore(), isPinned);
    expect(cache.size).toBe(3);

    // Adding thr_idle_3 must evict thr_idle_1, NOT thr_pinned_active
    cache.set("thr_idle_3", createTimelineStore(), isPinned);
    expect(cache.size).toBe(3);
    expect(cache.has("thr_pinned_active")).toBe(true);
    expect(cache.has("thr_idle_1")).toBe(false);
    expect(cache.has("thr_idle_2")).toBe(true);
    expect(cache.has("thr_idle_3")).toBe(true);
  });
});
