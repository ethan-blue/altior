import { describe, expect, it } from "vitest";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { Sequence } from "./dto/Sequence";
import { ConnectionClosedError } from "./errors";
import pingRaw from "../../../../crates/altior-protocol/fixtures/command-ping-v1.json";
import { negotiatedFixture } from "./fixtures";
import { InMemoryTransport } from "./inMemoryTransport";

const pingCommand = pingRaw as unknown as CommandEnvelope;

describe("InMemoryTransport", () => {
  it("returns the negotiated handshake fixture", async () => {
    const transport = new InMemoryTransport();

    await expect(transport.handshake()).resolves.toEqual(negotiatedFixture);
    expect(transport.status()).toBe("connected");
  });

  it("replays fixture events synchronously in sequence order", () => {
    const transport = new InMemoryTransport();
    const delivered: EventEnvelope[] = [];

    transport.subscribe((event) => delivered.push(event));

    expect(delivered.map((event) => event.sequence)).toEqual([1, 2]);
    expect(delivered.map((event) => event.body.kind)).toEqual([
      "turn.started",
      "usage.stats.snapshot",
    ]);
  });

  it("replays to every subscriber and stops after unsubscribe", () => {
    const transport = new InMemoryTransport();
    const first: EventEnvelope[] = [];
    const second: EventEnvelope[] = [];

    const unsubscribeFirst = transport.subscribe((event) => first.push(event));
    transport.subscribe((event) => second.push(event));
    unsubscribeFirst();

    expect(first).toHaveLength(2);
    expect(second).toHaveLength(2);
  });

  it("records sent commands and executes default command responses", async () => {
    const transport = new InMemoryTransport();
    const delivered: EventEnvelope[] = [];
    transport.subscribe((event) => delivered.push(event));

    const result = await transport.command<{ status: string }>(pingCommand);
    expect(result.status).toBe("ok");

    expect(transport.sentCommands).toEqual([pingCommand]);
    expect(delivered).toHaveLength(2);
  });

  it("supports reconnect with last_sequence cursor, emitting stream.replayed and stream.ready", async () => {
    const transport = new InMemoryTransport();
    // Add additional events to the transport
    transport.emit({ kind: "message.delta", text: "chunk 1" });
    transport.emit({ kind: "message.delta", text: "chunk 2" });
    transport.emit({ kind: "turn.completed" });

    const replayedEvents: EventEnvelope[] = [];
    transport.subscribe((ev) => replayedEvents.push(ev));
    // Clear subscription buffer for reconnect assertion
    replayedEvents.length = 0;

    // Simulate reconnect asking for events after sequence 2
    await transport.reconnect({ last_sequence: 2 as Sequence });

    const kinds = replayedEvents.map((e) => e.body.kind);
    expect(kinds).toContain("stream.replayed");
    expect(kinds).toContain("message.delta");
    expect(kinds).toContain("turn.completed");
    expect(kinds).toContain("stream.ready");

    const replayedHeader = replayedEvents.find((e) => e.body.kind === "stream.replayed");
    expect(replayedHeader).toBeDefined();
    if (replayedHeader && "from" in replayedHeader.body && "through" in replayedHeader.body) {
      expect(replayedHeader.body.from).toBe(3);
      expect(replayedHeader.body.through).toBeGreaterThanOrEqual(5);
    }
  });

  it("throws ConnectionClosedError on operations when closed", async () => {
    const transport = new InMemoryTransport();
    await transport.close();
    expect(transport.status()).toBe("closed");

    await expect(transport.connect()).rejects.toThrow(ConnectionClosedError);
    await expect(transport.command(pingCommand)).rejects.toThrow(ConnectionClosedError);
    await expect(transport.reconnect()).rejects.toThrow(ConnectionClosedError);
  });
});
