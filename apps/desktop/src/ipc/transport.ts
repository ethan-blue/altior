/**
 * The transport boundary every Desktop feature goes through.
 *
 * Production transports arrive with the P0.2 IPC decision; tests and the
 * fixture shell run against an in-memory implementation that replays the
 * same Rust-owned fixtures. Feature code never talks to Tauri, sockets, or
 * the filesystem directly (`docs/UI_ARCHITECTURE.md`, ADR 0005).
 */
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";

export interface DesktopTransport {
  /** Performs the version handshake and returns the negotiated result. */
  handshake(): Promise<NegotiatedHandshake>;
  /** Subscribes to ordered event envelopes; returns an unsubscribe fn. */
  subscribe(onEvent: (event: EventEnvelope) => void): () => void;
  /** Sends one command envelope. */
  send(command: CommandEnvelope): Promise<void>;
}
