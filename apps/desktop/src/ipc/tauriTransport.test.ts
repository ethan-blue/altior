import { afterEach, describe, expect, it, vi } from "vitest";
import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import {
  CommandError,
  HandshakeError,
  TransportUnavailableError,
} from "./errors";
import { negotiatedFixture } from "./fixtures";
import { InMemoryTransport } from "./inMemoryTransport";
import {
  createDefaultTransport,
  resolveTauriBridge,
  TauriCoreTransport,
} from "./tauriTransport";

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
      operation_id: "op_fixture000000201",
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
      operation_id: "op_err_1",
      kind: "ping",
      payload: {},
      issued_at: Date.now(),
    };
    await expect(transport.command(envelope)).rejects.toThrow(CommandError);
  });

  it("undoes an in-flight listen registration when subscribers leave first (A02)", async () => {
    const mockUnlisten = vi.fn();
    let resolveListen: ((unlisten: () => void) => void) | null = null;
    const mockListen = vi.fn().mockImplementation(
      () =>
        new Promise<() => void>((resolve) => {
          resolveListen = resolve;
        }),
    );

    const transport = new TauriCoreTransport({
      invoke: vi.fn(),
      listen: mockListen,
      fallbackToMemoryInDev: false,
    });

    const unsubscribe = transport.subscribe(() => {});
    unsubscribe();

    // The bridge confirms the listen only after the last subscriber left.
    resolveListen!(mockUnlisten);
    await Promise.resolve();
    await Promise.resolve();

    expect(mockUnlisten).toHaveBeenCalled();
  });

  it("reports a failed listen registration as unavailable and calls onError (A02)", async () => {
    const onError = vi.fn();
    const mockListen = vi.fn().mockRejectedValue(new Error("listen denied"));

    const transport = new TauriCoreTransport({
      invoke: vi.fn(),
      listen: mockListen,
      fallbackToMemoryInDev: false,
      onError,
    });

    const unsubscribe = transport.subscribe(() => {});
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(transport.status()).toBe("unavailable");
    expect(onError).toHaveBeenCalledWith(expect.any(Error));
    unsubscribe();
  });

  it("reuses one native listener across subscribe/unsubscribe cycles (A02)", async () => {
    const mockUnlisten = vi.fn();
    const mockListen = vi.fn().mockResolvedValue(mockUnlisten);

    const transport = new TauriCoreTransport({
      invoke: vi.fn(),
      listen: mockListen,
      fallbackToMemoryInDev: false,
    });

    for (let cycle = 0; cycle < 5; cycle += 1) {
      const unsubscribe = transport.subscribe(() => {});
      await Promise.resolve();
      unsubscribe();
    }

    expect(mockListen).toHaveBeenCalledTimes(5);
    expect(mockUnlisten).toHaveBeenCalledTimes(5);
  });
});

describe("createDefaultTransport entry decision (A02)", () => {
  it("uses the in-memory fixture transport only on an explicit dev entry", () => {
    const transport = createDefaultTransport({ isDev: true });
    expect(transport).toBeInstanceOf(InMemoryTransport);
  });

  it("never fakes a session in a plain production browser", () => {
    const transport = createDefaultTransport({ isDev: false });
    expect(transport).toBeInstanceOf(TauriCoreTransport);
    expect(transport.id).toBe("tauri-core");
    // Any use fails loudly with the typed unavailable error.
    expect(() => transport.subscribe(() => {})).toThrow(TransportUnavailableError);
  });

  it("routes a real bridge to the Tauri transport without fallback", async () => {
    const mockInvoke = vi.fn().mockResolvedValue(negotiatedFixture);
    const mockListen = vi.fn().mockResolvedValue(() => {});
    const transport = createDefaultTransport({
      isDev: true,
      invoke: mockInvoke,
      listen: mockListen,
    });

    expect(transport).toBeInstanceOf(TauriCoreTransport);
    await transport.connect();
    expect(mockInvoke).toHaveBeenCalledWith("core_handshake", { client: "altior-desktop" });
  });
});

describe("real WebView internals bridge (A18): stock Tauri v2 has no internals.listen", () => {
  const internalsBackup = (window as any).__TAURI_INTERNALS__;

  afterEach(() => {
    (window as any).__TAURI_INTERNALS__ = internalsBackup;
  });

  function installStockInternals(): {
    invoke: ReturnType<typeof vi.fn>;
    transformCallback: ReturnType<typeof vi.fn>;
    deliver: (ev: { payload: EventEnvelope }) => void;
  } {
    let registered: ((message: unknown) => void) | null = null;
    const invoke = vi.fn().mockImplementation((cmd: string) => {
      if (cmd === "core_handshake") {
        return Promise.resolve(negotiatedFixture);
      }
      if (cmd === "plugin:event|listen") {
        return Promise.resolve(31);
      }
      if (cmd === "plugin:event|unlisten") {
        return Promise.resolve();
      }
      return Promise.reject(new Error(`Unknown command: ${cmd}`));
    });
    const transformCallback = vi.fn().mockImplementation(
      (callback: (message: unknown) => void) => {
        registered = callback;
        return 31;
      },
    );
    (window as any).__TAURI_INTERNALS__ = { invoke, transformCallback };
    return {
      invoke,
      transformCallback,
      deliver: (ev) => registered!(ev),
    };
  }

  it("detects the bridge from invoke + transformCallback alone", () => {
    const { invoke } = installStockInternals();
    const bridge = resolveTauriBridge({});
    expect(bridge).not.toBeNull();
    expect(bridge!.invoke).toBe(invoke);
    expect(typeof bridge!.listen).toBe("function");
  });

  it("keeps the real WebView on the Tauri transport instead of dev fixtures", async () => {
    installStockInternals();
    const transport = createDefaultTransport({ isDev: true });
    expect(transport).toBeInstanceOf(TauriCoreTransport);
    expect((transport as TauriCoreTransport).isFallback).toBe(false);
    await transport.connect();
  });

  it("assembles the official plugin:event contract for listen and unlisten", async () => {
    const { invoke, transformCallback, deliver } = installStockInternals();
    const transport = createDefaultTransport({ isDev: true });
    await transport.connect();

    const received: EventEnvelope[] = [];
    const unsubscribe = transport.subscribe((event) => received.push(event));

    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("plugin:event|listen", {
        event: "core_event",
        target: { kind: "Any" },
        handler: 31,
      }),
    );
    expect(transformCallback).toHaveBeenCalledWith(
      expect.any(Function),
      false,
    );

    const testEvent: EventEnvelope = {
      protocol_version: 1,
      event_id: "evt_internals_1",
      operation_id: null,
      thread_id: "thread-1",
      turn_id: "turn-1",
      sequence: 7 as any,
      occurred_at: Date.now(),
      body: { kind: "turn.started" },
    };
    deliver({ payload: testEvent });
    expect(received).toEqual([testEvent]);

    unsubscribe();
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("plugin:event|unlisten", {
        event: "core_event",
        eventId: 31,
      }),
    );
    await transport.close();
  });

  it("still rejects a bridge when invoke is missing", () => {
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: vi.fn().mockReturnValue(1),
    };
    expect(resolveTauriBridge({})).toBeNull();
  });
});
