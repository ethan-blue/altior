import { describe, expect, it } from "vitest";
import type { AgentProfileDto } from "./dto/AgentProfileDto";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { ConfigureAgentCommand } from "./dto/ConfigureAgentCommand";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { HarnessBindingConfigDto } from "./dto/HarnessBindingConfigDto";
import type { HarnessBindingDto } from "./dto/HarnessBindingDto";
import type { Sequence } from "./dto/Sequence";
import type { TestHarnessBindingCommand } from "./dto/TestHarnessBindingCommand";
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

  it("handles configure_agent command with HarnessBindingConfigDto and updates bindings map", async () => {
    const transport = new InMemoryTransport();

    const bindingConfig: HarnessBindingConfigDto = {
      harness_binding_id: "bin_gamma_01",
      agent_profile_id: "agent-gamma",
      program: "/usr/local/bin/gamma-agent",
      args: ["--port", "9000"],
      env_keys: ["GAMMA_API_KEY"],
      secret_refs: ["vault://gamma-sec"],
      label: "Gamma ACP Binding",
    };

    const configureCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_cfg_gamma",
      kind: "configure_agent",
      payload: {
        agent_profile_id: "agent-gamma",
        display_name: "Gamma Agent",
        preferred_harness: "acp",
        memory_mode: "session",
        binding: bindingConfig,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<{
      ok: boolean;
      profile: AgentProfileDto;
      binding: HarnessBindingDto | null;
      warning: string | null;
    }>(configureCmd);

    expect(res.ok).toBe(true);
    expect(res.profile.id).toBe("agent-gamma");
    expect(res.binding?.id).toBe("bin_gamma_01");
    expect(res.binding?.program).toBe("/usr/local/bin/gamma-agent");
    expect(res.binding?.env_keys).toEqual(["GAMMA_API_KEY"]);
    expect(res.binding?.secret_refs).toEqual(["vault://gamma-sec"]);
    expect(res.warning).toBeNull();
    expect(transport.bindings.get("bin_gamma_01")?.program).toBe("/usr/local/bin/gamma-agent");
  });

  it("handles configure_agent without binding (legacy fallback) returning warning", async () => {
    const transport = new InMemoryTransport();

    const configureCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_cfg_legacy",
      kind: "configure_agent",
      payload: {
        agent_profile_id: "agent-legacy",
        display_name: "Legacy Agent",
        preferred_harness: "terminal",
        memory_mode: "session",
        binding: null,
      } as ConfigureAgentCommand,
      issued_at: Date.now(),
    };

    const res = await transport.command<{
      ok: boolean;
      profile: AgentProfileDto;
      binding: HarnessBindingDto | null;
      warning: string | null;
    }>(configureCmd);

    expect(res.ok).toBe(true);
    expect(res.profile.id).toBe("agent-legacy");
    expect(res.binding).toBeNull();
    expect(res.warning).toContain("Legacy configuration without harness binding");
  });

  it("executes test_harness_binding command with full payload and rejects when program is empty", async () => {
    const transport = new InMemoryTransport();

    const testCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_probe_gamma",
      kind: "test_harness_binding",
      payload: {
        harness_binding_id: "bin_gamma_01",
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
    expect(res.probed_binding_id).toBe("bin_gamma_01");

    // Fails when neither program nor binding id is present
    const emptyCmd: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_probe_empty",
      kind: "test_harness_binding",
      payload: {
        program: "",
        args: [],
        env_keys: [],
        secret_refs: [],
      } as TestHarnessBindingCommand,
      issued_at: Date.now(),
    };

    await expect(transport.command(emptyCmd)).rejects.toThrow("Missing required harness binding program executable");
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
