/**
 * Connection lifecycle acceptance for the entry/app/store chain (A02, F02).
 *
 * Covers what component tests miss: the real store init path through
 * StrictMode's mount → cleanup → mount cycle, listener bounds across
 * remounts, and the honest failure path of an unavailable bridge.
 * jsdom has no real Tauri bridge; the packaged-WebView handshake journey
 * stays with the release acceptance (A18) and is not faked here.
 */
import { StrictMode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { TauriCoreTransport } from "../ipc/tauriTransport";

describe("App connection lifecycle (A02)", () => {
  it("runs exactly one bootstrap round under StrictMode double effects", async () => {
    const transport = new InMemoryTransport({ autoStreamReplies: false });

    render(
      <StrictMode>
        <App transport={transport} />
      </StrictMode>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
    });

    const kinds = transport.sentCommands.map((c) => c.kind);
    expect(kinds.filter((k) => k === "list_threads")).toHaveLength(1);
    expect(kinds.filter((k) => k === "runtime_status")).toHaveLength(1);
    expect(kinds.filter((k) => k === "open_thread")).toHaveLength(1);
  });

  it("keeps a single event subscription across the StrictMode remount", async () => {
    const transport = new InMemoryTransport({ autoStreamReplies: false });

    render(
      <StrictMode>
        <App transport={transport} />
      </StrictMode>,
    );

    await waitFor(() => {
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
    });

    // Every stream event is processed exactly once despite the listener
    // detach/reattach cycle in between: the two fixture events plus the
    // runtime_status result event, with no duplicated sequence numbers.
    const labels = screen
      .getByTestId("protocol-diagnostics")
      .textContent?.match(/#\d/g);
    expect(labels).toEqual(["#1", "#2", "#3"]);
  });

  it("an unavailable bridge resolves init with honest error state, no fake success", async () => {
    // Production browser shape: no bridge, no dev fallback.
    const transport = new TauriCoreTransport({
      isDev: false,
      fallbackToMemoryInDev: false,
    });

    render(
      <StrictMode>
        <App transport={transport} />
      </StrictMode>,
    );

    // init() must resolve (no unhandled rejection) and the shell must say
    // plainly that the transport is unavailable instead of connecting or
    // negotiating a version.
    await waitFor(() => {
      expect(screen.getByTestId("status-bar").textContent).not.toContain("connecting");
    });
    expect(screen.getByTestId("status-bar").textContent).toContain("unavailable");
    expect(screen.getByTestId("ipc-version").textContent).toContain("unavailable");
  });

  it("release then init does not re-send bootstrap commands", async () => {
    const transport = new InMemoryTransport({ autoStreamReplies: false });
    const transportForStore = transport;
    render(<App transport={transportForStore} />);

    await waitFor(() => {
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
    });
    const afterFirstInit = transport.sentCommands.length;
    expect(afterFirstInit).toBeGreaterThan(0);

    // Simulate a remount by unmounting and rendering a fresh App against
    // the same store chain the way StrictMode does.
    const { unmount } = render(<App transport={transport} />);
    unmount();
    expect(transport.sentCommands.length).toBe(afterFirstInit);
  });
});
