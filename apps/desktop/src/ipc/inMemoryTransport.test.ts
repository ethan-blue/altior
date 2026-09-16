import { describe, expect, it } from "vitest";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "./dto/ConfigureAgentCommand";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { HarnessBindingConfigDto } from "./dto/HarnessBindingConfigDto";
import type { Sequence } from "./dto/Sequence";
import type { TestHarnessBindingCommand } from "./dto/TestHarnessBindingCommand";
import { ConnectionClosedError, InvalidCommandError } from "./errors";
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

  it("handles configure_agent with HarnessBindingConfigDto and updates bindings map", async () => {
    const transport = new InMemoryTransport();

    const bindingConfig: HarnessBindingConfigDto = {
      harness_binding_id: "hsb_fixture000000101",
      agent_profile_id: null,
      program: "/usr/local/bin/gamma-agent",
      args: ["--port", "9000"],
      env_keys: ["GAMMA_API_KEY"],
      secret_refs: ["vault://gamma-sec"],
      label: "Gamma ACP Binding",
    };

    const configureCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000101",
      kind: "configure_agent",
      payload: {
        agent_profile_id: null,
        display_name: "Gamma Agent",
        preferred_harness: "acp",
        memory_mode: "session",
        binding: bindingConfig,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    // Mirrors real Core (ADR 0019): result data carries the configured
    // profile and binding identities.
    const res = await transport.command<{
      agent_profile_id: string;
      harness_binding_id: string | null;
    }>(configureCmd);

    expect(res.agent_profile_id).toMatch(/^agp_[0-9a-z]{16,64}$/);
    expect(res.harness_binding_id).toBe("hsb_fixture000000101");
    expect(transport.agents.some((a) => a.id === res.agent_profile_id)).toBe(true);
    expect(transport.bindings.get("hsb_fixture000000101")?.program).toBe("/usr/local/bin/gamma-agent");
    expect(transport.bindings.get("hsb_fixture000000101")?.agent_profile_id).toBe(res.agent_profile_id);
  });

  it("handles configure_agent without binding: Core mints the profile, no binding id", async () => {
    const transport = new InMemoryTransport();

    const configureCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000102",
      kind: "configure_agent",
      payload: {
        agent_profile_id: null,
        display_name: "Legacy Agent",
        preferred_harness: "terminal",
        memory_mode: "session",
        binding: null,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<{
      agent_profile_id: string;
      harness_binding_id: string | null;
    }>(configureCmd);

    expect(res.agent_profile_id).toMatch(/^agp_[0-9a-z]{16,64}$/);
    expect(res.harness_binding_id).toBeNull();
    expect(transport.agents.some((a) => a.id === res.agent_profile_id)).toBe(true);
  });

  it("executes test_harness_binding and echoes the probed binding id; rejects empty program", async () => {
    const transport = new InMemoryTransport();

    const testCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000103",
      kind: "test_harness_binding",
      payload: {
        harness_binding_id: "hsb_fixture000000101",
        program: "/usr/local/bin/gamma-agent",
        args: ["--port", "9000"],
        env_keys: ["GAMMA_API_KEY"],
        secret_refs: ["vault://gamma-sec"],
        label: "Gamma Probe",
      } as TestHarnessBindingCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<{ ok: boolean; probed_binding_id: string }>(testCmd);
    expect(res.ok).toBe(true);
    expect(res.probed_binding_id).toBe("hsb_fixture000000101");

    // A probe without a binding id gets a Core-minted one back (ADR 0019).
    const mintedProbe: CommandEnvelope = {
      ...testCmd,
      operation_id: "op_fixture000000104",
      payload: {
        harness_binding_id: null,
        program: "/usr/local/bin/gamma-agent",
        args: [],
        env_keys: [],
        secret_refs: [],
        label: null,
      } as TestHarnessBindingCommand,
    };
    const mintedRes = await transport.command<{ ok: boolean; probed_binding_id: string | null }>(mintedProbe);
    expect(mintedRes.ok).toBe(true);
    expect(mintedRes.probed_binding_id).toMatch(/^hsb_[0-9a-z]{16,64}$/);

    // An empty program is rejected at the contract boundary before any
    // execution (A01).
    const emptyCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000105",
      kind: "test_harness_binding",
      payload: {
        program: "",
        args: [],
        env_keys: [],
        secret_refs: [],
      } as TestHarnessBindingCommand,
      issued_at: Date.now(),
    };

    await expect(transport.command(emptyCmd)).rejects.toThrow(InvalidCommandError);
  });

  it("rejects commands that violate the identifier contract (A01)", async () => {
    const transport = new InMemoryTransport();

    const invalidOp: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_start_turn_1",
      kind: "list_threads",
      payload: { cursor: null, limit: 50 },
      issued_at: Date.now(),
    };
    const invalidAgent: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000106",
      kind: "create_thread",
      payload: { agent_profile_id: "agent-alpha", title: "x", project_id: null },
      issued_at: Date.now(),
    };
    const invalidThread: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_fixture000000107",
      kind: "open_thread",
      payload: { thread_id: "thread-1_1700000000000", history_limit: 100 },
      issued_at: Date.now(),
    };

    await expect(transport.command(invalidOp)).rejects.toBeInstanceOf(InvalidCommandError);
    await expect(transport.command(invalidAgent)).rejects.toBeInstanceOf(InvalidCommandError);
    await expect(transport.command(invalidThread)).rejects.toBeInstanceOf(InvalidCommandError);
    expect(transport.sentCommands).toHaveLength(0);
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
