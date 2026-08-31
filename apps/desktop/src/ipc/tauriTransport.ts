import type { CommandEnvelope } from "./dto/CommandEnvelope";
import type { EventEnvelope } from "./dto/EventEnvelope";
import type { NegotiatedHandshake } from "./dto/NegotiatedHandshake";
import {
  CommandError,
  ConnectionClosedError,
  HandshakeError,
  TransportUnavailableError,
} from "./errors";
import { InMemoryTransport } from "./inMemoryTransport";
import type { CoreTransport, ReconnectCursor, TransportStatus } from "./transport";

export interface TauriInvokeFn {
  <T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

export interface TauriListenFn {
  (
    event: string,
    handler: (event: { payload: EventEnvelope }) => void,
  ): Promise<() => void>;
}

export interface TauriCoreTransportOptions {
  /** Explicit invoke function (used for injection or test). */
  readonly invoke?: TauriInvokeFn;
  /** Explicit listen function (used for injection or test). */
  readonly listen?: TauriListenFn;
  /**
   * If true and Tauri is unavailable, gracefully fall back to InMemoryTransport
   * in development mode only. Defaults to true.
   */
  readonly fallbackToMemoryInDev?: boolean;
  /** Force dev mode flag for fallback testing. */
  readonly isDev?: boolean;
  /** Initial fallback options if fallback is activated. */
  readonly fallbackOptions?: ConstructorParameters<typeof InMemoryTransport>[0];
}

/**
 * Production CoreTransport implementation communicating via Tauri's IPC bridge.
 *
 * Calls `invoke` for commands/handshake and `listen` for event streaming.
 * If Tauri capabilities are missing or unavailable in production, typed
 * `TransportUnavailableError` is thrown; in development mode it can safely
 * fall back to `InMemoryTransport` for local prototyping.
 */
export class TauriCoreTransport implements CoreTransport {
  readonly id = "tauri-core";
  #status: TransportStatus = "disconnected";
  #invoke: TauriInvokeFn | null = null;
  #listen: TauriListenFn | null = null;
  #fallbackDelegate: InMemoryTransport | null = null;
  #listeners = new Set<(event: EventEnvelope) => void>();
  #tauriUnlisten: (() => void) | null = null;
  readonly #fallbackToMemoryInDev: boolean;
  readonly #isDev: boolean;

  constructor(options: TauriCoreTransportOptions = {}) {
    this.#fallbackToMemoryInDev = options.fallbackToMemoryInDev ?? true;
    const isDevEnv =
      options.isDev ??
      (typeof process !== "undefined" && process.env?.NODE_ENV === "development") ??
      false;
    this.#isDev = isDevEnv;

    const detected = this.#resolveTauriBridge(options);
    if (detected) {
      this.#invoke = detected.invoke;
      this.#listen = detected.listen;
    } else if (this.#fallbackToMemoryInDev && this.#isDev) {
      this.#fallbackDelegate = new InMemoryTransport(options.fallbackOptions);
    }
  }

  /** True if this transport is running via in-memory fallback in dev mode. */
  get isFallback(): boolean {
    return this.#fallbackDelegate != null;
  }

  status(): TransportStatus {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.status();
    }
    return this.#status;
  }

  async connect(): Promise<NegotiatedHandshake> {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.connect();
    }
    this.#ensureAvailable();
    this.#status = "connecting";

    try {
      const result = await this.#invoke!<NegotiatedHandshake>("core_handshake", {
        client: "altior-desktop",
      });
      this.#status = "connected";
      return result;
    } catch (error) {
      this.#status = "unavailable";
      if (error instanceof HandshakeError || error instanceof TransportUnavailableError) {
        throw error;
      }
      throw new HandshakeError(
        `Failed to negotiate Core handshake: ${error instanceof Error ? error.message : String(error)}`,
        error,
      );
    }
  }

  handshake(): Promise<NegotiatedHandshake> {
    return this.connect();
  }

  async command<T = unknown>(envelope: CommandEnvelope): Promise<T> {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.command<T>(envelope);
    }
    this.#ensureAvailable();
    if (this.#status === "closed") {
      throw new ConnectionClosedError();
    }

    try {
      return await this.#invoke!<T>("core_command", { envelope });
    } catch (error) {
      throw new CommandError(
        `Core command '${envelope.kind}' failed: ${error instanceof Error ? error.message : String(error)}`,
        "COMMAND_FAILED",
        error,
      );
    }
  }

  send(command: CommandEnvelope): Promise<void> {
    return this.command(command).then(() => undefined);
  }

  subscribe(onEvent: (event: EventEnvelope) => void): () => void {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.subscribe(onEvent);
    }
    this.#ensureAvailable();
    this.#listeners.add(onEvent);

    if (this.#listeners.size === 1 && this.#listen) {
      void this.#listen("core_event", ({ payload }) => {
        for (const listener of this.#listeners) {
          listener(payload);
        }
      }).then((unlisten) => {
        this.#tauriUnlisten = unlisten;
      });
    }

    return () => {
      this.#listeners.delete(onEvent);
      if (this.#listeners.size === 0 && this.#tauriUnlisten) {
        this.#tauriUnlisten();
        this.#tauriUnlisten = null;
      }
    };
  }

  async reconnect(cursor?: ReconnectCursor): Promise<NegotiatedHandshake> {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.reconnect(cursor);
    }
    this.#ensureAvailable();
    this.#status = "reconnecting";

    try {
      const result = await this.#invoke!<NegotiatedHandshake>("core_reconnect", {
        cursor,
      });
      this.#status = "connected";
      return result;
    } catch (error) {
      this.#status = "disconnected";
      throw new HandshakeError(
        `Failed to reconnect to Core: ${error instanceof Error ? error.message : String(error)}`,
        error,
      );
    }
  }

  async close(): Promise<void> {
    if (this.#fallbackDelegate) {
      return this.#fallbackDelegate.close();
    }
    this.#status = "closed";
    if (this.#tauriUnlisten) {
      this.#tauriUnlisten();
      this.#tauriUnlisten = null;
    }
    this.#listeners.clear();

    if (this.#invoke) {
      try {
        await this.#invoke("core_close");
      } catch {
        // Ignore close errors
      }
    }
  }

  #ensureAvailable(): void {
    if (this.#fallbackDelegate) return;
    if (!this.#invoke || !this.#listen) {
      this.#status = "unavailable";
      throw new TransportUnavailableError(
        "Tauri IPC capabilities are not available in the current environment.",
      );
    }
  }

  #resolveTauriBridge(options: TauriCoreTransportOptions): {
    invoke: TauriInvokeFn;
    listen: TauriListenFn;
  } | null {
    if (options.invoke && options.listen) {
      return { invoke: options.invoke, listen: options.listen };
    }

    // Check globals if available
    const win = typeof window !== "undefined" ? (window as any) : null;
    const tauriInternals = win?.__TAURI_INTERNALS__;
    const tauriGlobal = win?.__TAURI__;

    const invoke =
      options.invoke ??
      tauriInternals?.invoke ??
      tauriGlobal?.core?.invoke ??
      tauriGlobal?.invoke ??
      null;

    const listen =
      options.listen ??
      tauriInternals?.listen ??
      tauriGlobal?.event?.listen ??
      tauriGlobal?.listen ??
      null;

    if (typeof invoke === "function" && typeof listen === "function") {
      return { invoke, listen };
    }

    return null;
  }
}

/**
 * Factory creating the default transport for the current runtime environment.
 *
 * In Tauri Desktop runtime: creates TauriCoreTransport configured for real Core IPC.
 * In browser development mode: falls back to InMemoryTransport for UI prototyping.
 * In non-Tauri production browser: creates TauriCoreTransport (fails with TransportUnavailableError).
 */
export function createDefaultTransport(
  options: TauriCoreTransportOptions = {},
): CoreTransport {
  const win = typeof window !== "undefined" ? (window as any) : null;
  const isTauri =
    win != null &&
    (win.__TAURI_INTERNALS__ != null ||
      win.__TAURI__ != null ||
      typeof options.invoke === "function");

  const isDev =
    options.isDev ??
    (typeof process !== "undefined" && process.env?.NODE_ENV === "development") ??
    (typeof import.meta !== "undefined" && (import.meta as any).env?.DEV) ??
    false;

  if (isTauri) {
    return new TauriCoreTransport({
      ...options,
      fallbackToMemoryInDev: false,
    });
  }

  if (isDev) {
    return new InMemoryTransport(options.fallbackOptions);
  }

  return new TauriCoreTransport({
    ...options,
    fallbackToMemoryInDev: false,
  });
}
