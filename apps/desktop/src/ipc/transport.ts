/**
 * The transport boundary every Desktop feature goes through.
 *
 * Provides typed abstraction for Core IPC: connect, handshake, command,
 * subscribe, reconnect, and close. Retains in-memory implementation for tests
 * and fixture shell, and provides TauriCoreTransport for production wiring.
 */
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";
import type { Sequence } from "./dto/Sequence";

export type TransportStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "unavailable"
  | "closed";

export interface ReconnectCursor {
  /** The last processed sequence number received by the client. */
  readonly last_sequence?: Sequence;
  /** Optional cursor identifier or snapshot token. */
  readonly cursor_id?: string;
}

export interface CoreTransport {
  /** Unique ID or discriminator for this transport instance. */
  readonly id?: string;

  /** Current connection lifecycle state. */
  status(): TransportStatus;

  /** Initializes the connection and performs handshake negotiation. */
  connect(): Promise<NegotiatedHandshake>;

  /** Performs the version handshake and returns the negotiated result. */
  handshake(): Promise<NegotiatedHandshake>;

  /** Sends a command envelope and returns the typed response from Core. */
  command<T = unknown>(envelope: CommandEnvelope): Promise<T>;

  /** Backwards-compatible send for one command envelope. */
  send(command: CommandEnvelope): Promise<void>;

  /** Subscribes to ordered event envelopes; returns an unsubscribe fn. */
  subscribe(onEvent: (event: EventEnvelope) => void): () => void;

  /**
   * Re-establishes connection starting from the given sequence or cursor,
   * asking Core to replay missed events through the retained window.
   */
  reconnect(cursor?: ReconnectCursor): Promise<NegotiatedHandshake>;

  /** Gracefully shuts down the transport and cancels active subscriptions. */
  close(): Promise<void>;
}

/** Backwards-compatible alias for CoreTransport. */
export type DesktopTransport = CoreTransport;
