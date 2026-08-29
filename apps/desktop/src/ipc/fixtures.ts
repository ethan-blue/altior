/**
 * Rust-owned protocol fixtures, imported directly from the protocol crate.
 *
 * The JSON files are validated and re-encoded byte-for-byte by the Rust
 * fixture suite (`crates/altior-protocol/tests/fixtures.rs`); the casts
 * below are therefore backed by a checked-in contract, not trust in this
 * file. Fixtures contain only synthetic data (ADR 0005).
 */
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";

import negotiatedRaw from "../../../../crates/altior-protocol/fixtures/handshake-negotiated-v1.json";
import eventTurnStartedRaw from "../../../../crates/altior-protocol/fixtures/event-turn-started-v1.json";
import eventUnknownPreservedRaw from "../../../../crates/altior-protocol/fixtures/event-unknown-preserved-v1.json";

export const negotiatedFixture = negotiatedRaw as unknown as NegotiatedHandshake;

/** Fixture events in stream sequence order. */
export const eventFixtures: EventEnvelope[] = [
  eventTurnStartedRaw as unknown as EventEnvelope,
  eventUnknownPreservedRaw as unknown as EventEnvelope,
];
