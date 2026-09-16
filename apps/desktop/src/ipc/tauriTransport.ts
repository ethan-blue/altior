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
  /**
   * Called when asynchronous bridge operations fail outside a command
   * round-trip (e.g. the event listener registration). Commands still
   * surface their own typed errors; this covers the fire-and-forget paths.
   */
  readonly onError?: (error: unknown) => void;
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
  readonly #onError?: (error: unknown) => void;

  constructor(options: TauriCoreTransportOptions = {}) {
    this.#fallbackToMemoryInDev = options.fallbackToMemoryInDev ?? true;
    // Explicit boolean resolution: never `false ?? fallback`, which keeps
    // false and silently skips the Vite dev flag (review F02).
    this.#isDev = options.isDev ?? viteDevFlag();
    this.#onError = options.onError;

    const detected = resolveTauriBridge(options);
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
      // Asynchronous listener registration with ready/cancel handling: if
      // the last subscriber leaves before the bridge confirms the listen,
      // the registration is undone as soon as it arrives, so fast
      // subscribe/unsubscribe cycles never leak a native listener.
      void this.#listen("core_event", this.#dispatch)
        .then((unlisten) => {
          if (this.#listeners.size === 0) {
            unlisten();
            this.#tauriUnlisten = null;
            return;
          }
          this.#tauriUnlisten = unlisten;
        })
        .catch((error: unknown) => {
          this.#status = "unavailable";
          this.#onError?.(error);
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

  #dispatch = (event: { payload: EventEnvelope }): void => {
    for (const listener of [...this.#listeners]) {
      listener(event.payload);
    }
  };

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
}

/**
 * The Vite development flag, resolved as a plain boolean. Browser builds
 * have no `process`, so a NODE_ENV check must never gate this (review F02:
 * `false ?? dev` kept dev builds on the Tauri transport).
 */
function viteDevFlag(): boolean {
  return typeof import.meta !== "undefined" && (import.meta as any).env?.DEV === true;
}

/**
 * Resolves the Tauri bridge from explicit injection or, when the shell
 * exposes it, from `__TAURI_INTERNALS__`/`__TAURI__`. With
 * `withGlobalTauri: false` the production WebView only provides the
 * internals object; there is deliberately no global-object fallback beyond
 * that (ADR 0008 §6).
 */
export function resolveTauriBridge(options: TauriCoreTransportOptions): {
  invoke: TauriInvokeFn;
  listen: TauriListenFn;
} | null {
  if (options.invoke && options.listen) {
    return { invoke: options.invoke, listen: options.listen };
  }

  const win = typeof window !== "undefined" ? (window as any) : null;
  const invoke = win?.__TAURI_INTERNALS__?.invoke ?? null;
  const listen = win?.__TAURI_INTERNALS__?.listen ?? null;

  if (typeof invoke === "function" && typeof listen === "function") {
    return { invoke, listen };
  }
  return null;
}

/**
 * The single transport decision for the entry point (ADR 0008, review A02):
 *
 * 1. Tauri bridge present (WebView runtime): real Core IPC, no fallback.
 * 2. Vite dev server (no bridge): explicit development fixture entry.
 * 3. Plain production browser: the Tauri transport, which fails loudly and
 *    honestly — it never fabricates a working session.
 */
export function createDefaultTransport(
  options: TauriCoreTransportOptions = {},
): CoreTransport {
  const bridge = resolveTauriBridge(options);
  const isDev = options.isDev ?? viteDevFlag();

  if (bridge) {
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
