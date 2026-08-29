import { describe, expect, it } from "vitest";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import pingRaw from "../../../../crates/altior-protocol/fixtures/command-ping-v1.json";
import { negotiatedFixture } from "./fixtures";
import { InMemoryTransport } from "./inMemoryTransport";

const pingCommand = pingRaw as unknown as CommandEnvelope;

describe("InMemoryTransport", () => {
  it("returns the negotiated handshake fixture", async () => {
    const transport = new InMemoryTransport();

    await expect(transport.handshake()).resolves.toEqual(negotiatedFixture);
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

  it("records sent commands without delivering events", async () => {
    const transport = new InMemoryTransport();
    const delivered: EventEnvelope[] = [];
    transport.subscribe((event) => delivered.push(event));

    await transport.send(pingCommand);

    expect(transport.sentCommands).toEqual([pingCommand]);
    expect(delivered).toHaveLength(2);
  });
});
