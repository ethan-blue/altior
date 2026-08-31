import { describe, expect, it, vi } from "vitest";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import {
  CommandError,
  HandshakeError,
  TransportUnavailableError,
} from "./errors";
import { negotiatedFixture } from "./fixtures";
import { TauriCoreTransport } from "./tauriTransport";

describe("TauriCoreTransport", () => {
  it("throws TransportUnavailableError when Tauri bridge is absent in production", async () => {
    const transport = new TauriCoreTransport({
      isDev: false,
      fallbackToMemoryInDev: false,
    });

    expect(transport.isFallback).toBe(false);
    expect(transport.status()).toBe("disconnected");

    await expect(transport.connect()).rejects.toThrow(TransportUnavailableError);
  });

  it("falls back to InMemoryTransport in development mode when configured", async () => {
    const transport = new TauriCoreTransport({
      isDev: true,
      fallbackToMemoryInDev: true,
    });

    expect(transport.isFallback).toBe(true);

    const handshake = await transport.connect();
    expect(handshake.selected_version).toBe(negotiatedFixture.selected_version);
    expect(transport.status()).toBe("connected");

    const events: EventEnvelope[] = [];
    const unsubscribe = transport.subscribe((ev) => events.push(ev));
    expect(events.length).toBeGreaterThan(0);
    unsubscribe();
  });

  it("invokes Tauri commands and listens for core events when bridge is available", async () => {
    const mockInvoke = vi.fn().mockImplementation((cmd: string, args?: any) => {
      if (cmd === "core_handshake") {
        return Promise.resolve(negotiatedFixture);
      }
      if (cmd === "core_command") {
        return Promise.resolve({ ok: true, echo: args.envelope.kind });
      }
      if (cmd === "core_reconnect") {
        return Promise.resolve(negotiatedFixture);
      }
      if (cmd === "core_close") {
        return Promise.resolve();
      }
      return Promise.reject(new Error(`Unknown command: ${cmd}`));
    });

    let listenerCallback: ((ev: { payload: EventEnvelope }) => void) | null = null;
    const mockUnlisten = vi.fn();
    const mockListen = vi.fn().mockImplementation((_event: string, handler: any) => {
      listenerCallback = handler;
      return Promise.resolve(mockUnlisten);
    });

    const transport = new TauriCoreTransport({
      invoke: mockInvoke,
      listen: mockListen,
      fallbackToMemoryInDev: false,
    });

    const handshake = await transport.connect();
    expect(handshake).toEqual(negotiatedFixture);
    expect(mockInvoke).toHaveBeenCalledWith("core_handshake", { client: "altior-desktop" });

    // Send command
    const commandEnvelope: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_test_1",
      kind: "ping",
      payload: { hello: "world" },
      issued_at: Date.now(),
    };
    const response = await transport.command<{ ok: boolean; echo: string }>(commandEnvelope);
    expect(response).toEqual({ ok: true, echo: "ping" });
    expect(mockInvoke).toHaveBeenCalledWith("core_command", { envelope: commandEnvelope });

    // Subscribe to events
    const received: EventEnvelope[] = [];
    const unsubscribe = transport.subscribe((event) => received.push(event));

    expect(mockListen).toHaveBeenCalledWith("core_event", expect.any(Function));

    // Simulate event delivery from Tauri backend
    const testEvent: EventEnvelope = {
      protocol_version: 1,
      event_id: "evt_tauri_1",
      operation_id: null,
      thread_id: "thread-1",
      turn_id: "turn-1",
      sequence: 42 as any,
      occurred_at: Date.now(),
      body: { kind: "turn.started" },
    };
    listenerCallback!({ payload: testEvent });
    expect(received).toEqual([testEvent]);

    // Reconnect with cursor
    await transport.reconnect({ last_sequence: 42 as any });
    expect(mockInvoke).toHaveBeenCalledWith("core_reconnect", {
      cursor: { last_sequence: 42 },
    });

    // Unsubscribe and close
    unsubscribe();
    expect(mockUnlisten).toHaveBeenCalled();

    await transport.close();
    expect(mockInvoke).toHaveBeenCalledWith("core_close");
    expect(transport.status()).toBe("closed");
  });

  it("wraps bridge failures in typed HandshakeError and CommandError", async () => {
    const failingInvoke = vi.fn().mockImplementation((cmd: string) => {
      if (cmd === "core_handshake") {
        return Promise.reject(new Error("Handshake version mismatch"));
      }
      if (cmd === "core_command") {
        return Promise.reject(new Error("Core rejected command"));
      }
      return Promise.reject(new Error("Unknown"));
    });
    const mockListen = vi.fn().mockResolvedValue(() => {});

    const transport = new TauriCoreTransport({
      invoke: failingInvoke,
      listen: mockListen,
      fallbackToMemoryInDev: false,
    });

    await expect(transport.connect()).rejects.toThrow(HandshakeError);

    const envelope: CommandEnvelope = {
      protocol_version: 1,
      operation_id: "op_err",
      kind: "ping",
      payload: {},
      issued_at: Date.now(),
    };
    await expect(transport.command(envelope)).rejects.toThrow(CommandError);
  });
});
