import type { EventEnvelope } from "../ipc/dto/EventEnvelope";

export interface EventStreamStore {
  /** `useSyncExternalStore` subscription; returns an unsubscribe fn. */
  subscribe(listener: () => void): () => void;
  /** Immutable snapshot; the reference is stable until an append. */
  getSnapshot(): readonly EventEnvelope[];
  /** Appends one event and notifies subscribers. */
  append(event: EventEnvelope): void;
  /** Drops all events and notifies subscribers. */
  reset(): void;
}

/**
 * Minimal external store for the ordered event stream. Kept outside React
 * so the transport wiring (subscribe/unsubscribe) stays testable without
 * rendering (`docs/UI_ARCHITECTURE.md`, stores layer).
 */
export function createEventStreamStore(): EventStreamStore {
  let events: readonly EventEnvelope[] = [];
  const listeners = new Set<() => void>();

  const notify = (): void => {
    for (const listener of listeners) {
      listener();
    }
  };

  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot() {
      return events;
    },
    append(event) {
      events = [...events, event];
      notify();
    },
    reset() {
      events = [];
      notify();
    },
  };
}
