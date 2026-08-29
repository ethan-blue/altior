import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";
import type { DesktopTransport } from "./transport";
import { eventFixtures, negotiatedFixture } from "./fixtures";

export interface InMemoryTransportOptions {
  /** Handshake result to return; defaults to the negotiated fixture. */
  readonly negotiated?: NegotiatedHandshake;
  /** Events to replay; defaults to the fixture stream. */
  readonly events?: readonly EventEnvelope[];
}

/**
 * In-memory `DesktopTransport` for tests and the fixture shell.
 *
 * Deterministic by construction: no timers, no randomness, no network.
 * Fixture events are replayed synchronously in stream-sequence order when
 * a subscriber attaches; sent commands are recorded for assertions.
 */
export class InMemoryTransport implements DesktopTransport {
  readonly #negotiated: NegotiatedHandshake;
  readonly #events: readonly EventEnvelope[];
  readonly #listeners = new Set<(event: EventEnvelope) => void>();
  readonly #sent: CommandEnvelope[] = [];

  constructor(options: InMemoryTransportOptions = {}) {
    this.#negotiated = options.negotiated ?? negotiatedFixture;
    this.#events = [...(options.events ?? eventFixtures)].sort(
      (a, b) => a.sequence - b.sequence,
    );
  }

  /** Commands sent through `send`, in send order. */
  get sentCommands(): readonly CommandEnvelope[] {
    return this.#sent;
  }

  handshake(): Promise<NegotiatedHandshake> {
    return Promise.resolve(structuredClone(this.#negotiated));
  }

  subscribe(onEvent: (event: EventEnvelope) => void): () => void {
    this.#listeners.add(onEvent);
    for (const event of this.#events) {
      onEvent(structuredClone(event));
    }
    return () => {
      this.#listeners.delete(onEvent);
    };
  }

  send(command: CommandEnvelope): Promise<void> {
    this.#sent.push(structuredClone(command));
    return Promise.resolve();
  }
}
