import { useEffect, useState, useSyncExternalStore } from "react";
import type { NegotiatedHandshake } from "../ipc/dto/NegotiatedHandshake";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import type { DesktopTransport } from "../ipc/transport";
import { createEventStreamStore } from "../stores/eventStreamStore";
import styles from "./App.module.css";

export interface AppProps {
  /** Transport to run against; defaults to the in-memory fixture shell. */
  readonly transport?: DesktopTransport;
}

function capabilityList(negotiated: NegotiatedHandshake): string[] {
  return Object.entries(negotiated.negotiated_capabilities).map(
    ([id, support]) => `${id}: ${String(support)}`,
  );
}

/**
 * Fixture shell for P0.1: proves the generated DTOs, the transport
 * boundary, and the Rust-owned fixtures render end to end. No thread
 * state, composer, or Tauri wiring — those arrive in later phases.
 */
export function App({ transport = new InMemoryTransport() }: AppProps) {
  const [store] = useState(createEventStreamStore);
  const [negotiated, setNegotiated] = useState<NegotiatedHandshake | null>(null);
  const events = useSyncExternalStore(store.subscribe, store.getSnapshot);

  useEffect(() => {
    let active = true;
    const unsubscribe = transport.subscribe((event) => store.append(event));
    void transport.handshake().then((result) => {
      if (active) {
        setNegotiated(result);
      }
    });
    return () => {
      active = false;
      unsubscribe();
    };
  }, [transport, store]);

  return (
    <div className={styles.shell}>
      <header className={styles.header}>
        <h1 className={styles.title}>Altior</h1>
        <p className={styles.subtitle}>Fixture shell · in-memory IPC</p>
      </header>

      <div
        className={styles.statusBar}
        role="status"
        aria-label="IPC connection status"
      >
        {negotiated ? (
          <>
            <span data-testid="ipc-version">IPC v{negotiated.selected_version}</span>
            <ul className={styles.capabilities} aria-label="Negotiated capabilities">
              {capabilityList(negotiated).map((capability) => (
                <li key={capability} className={styles.capability}>
                  {capability}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <span className={styles.pending}>Negotiating…</span>
        )}
      </div>

      <main className={styles.timeline} aria-label="Event timeline">
        <ul>
          {events.map((event) => (
            <li key={event.event_id} className={styles.eventRow}>
              <span className={styles.sequence}>#{event.sequence}</span>
              <span className={styles.eventKind}>{event.body.kind}</span>
              {"diagnostic" in event.body ? (
                <code className={styles.diagnostic}>{event.body.diagnostic}</code>
              ) : null}
            </li>
          ))}
        </ul>
      </main>

      <footer className={styles.composer}>
        <input
          className={styles.composerInput}
          type="text"
          disabled
          placeholder="Composer arrives with the first turn input"
          aria-label="Message composer (disabled in fixture shell)"
          readOnly
        />
        <button type="button" className={styles.composerSend} disabled>
          Send
        </button>
      </footer>
    </div>
  );
}
