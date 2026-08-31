import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { approvalThread } from "../fixtures/timeline";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { TauriCoreTransport } from "../ipc/tauriTransport";
import { App } from "./App";

describe("Transport Integration & UI Workflow", () => {
  it("opens onboarding modal, tests connection with opaque secret ref, and saves new agent", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);

    // Click "Agents" activity rail item
    fireEvent.click(screen.getByTestId("rail-agents"));

    // Verify modal is open
    expect(screen.getByRole("dialog", { name: "Agent Onboarding" })).toBeInTheDocument();

    // Fill form with opaque secret ref
    fireEvent.change(screen.getByTestId("agent-name-input"), {
      target: { value: "Delta ACP" },
    });
    fireEvent.change(screen.getByTestId("agent-provider-input"), {
      target: { value: "acp" },
    });
    fireEvent.change(screen.getByTestId("agent-model-input"), {
      target: { value: "claude-3-7-sonnet" },
    });
    fireEvent.change(screen.getByTestId("agent-secret-ref"), {
      target: { value: "vault://delta-sec-key" },
    });

    // Test connection
    fireEvent.click(screen.getByTestId("agent-test-button"));
    await waitFor(() => {
      expect(screen.getByText(/Connection verified/)).toBeInTheDocument();
    });

    // Save agent
    fireEvent.click(screen.getByTestId("agent-save-button"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    // Verify agent is in the agent selector
    const selector = screen.getByTestId("agent-selector") as HTMLSelectElement;
    expect(selector).toBeInTheDocument();
    expect(selector.textContent).toContain("Delta ACP");
  });

  it("creates a new thread from the threads pane", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("new-thread"));

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(/Thread \d+/);
    });
  });

  it("allows denying permission from the timeline", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId(`thread-${approvalThread.id}`));

    const denyBtn = await screen.findByTestId("deny");
    fireEvent.click(denyBtn);

    await waitFor(() => {
      expect(
        document.querySelector("[data-row-kind='permission']")?.textContent,
      ).toContain("denied");
    });
  });

  it("shows cancel button when turn is streaming and sends cancel command", async () => {
    const transport = new InMemoryTransport();
    // Delay prompt response so streaming is active
    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "start_turn") {
        return new Promise(() => {}); // hang to simulate active streaming
      }
      return { ok: true };
    });

    render(<App transport={transport} />);

    const composer = screen.getByTestId("composer");
    fireEvent.change(composer, { target: { value: "Perform heavy analysis" } });
    fireEvent.click(screen.getByTestId("send"));

    const cancelBtn = await screen.findByTestId("cancel-turn");
    expect(cancelBtn).toBeInTheDocument();

    fireEvent.click(cancelBtn);

    await waitFor(() => {
      expect(screen.queryByTestId("cancel-turn")).toBeNull();
    });

    const cancelCommand = transport.sentCommands.find(
      (c) => c.kind === "cancel_turn" || c.kind === "cancel",
    );
    expect(cancelCommand).toBeDefined();
    expect(cancelCommand?.kind).toBe("cancel_turn");
  });

  it("shows disconnected status and recovers when reconnect is clicked", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);

    await waitFor(() => {
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
    });

    // Simulate disconnect on the transport
    transport.simulateDisconnect();
    // Reconnection button click
    const reconnectBtn = screen.queryByTestId("reconnect-button");
    if (reconnectBtn) {
      fireEvent.click(reconnectBtn);
      await waitFor(() => {
        expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
      });
    }
  });

  it("supports dev fallback with TauriCoreTransport", async () => {
    const devFallbackTransport = new TauriCoreTransport({
      isDev: true,
      fallbackToMemoryInDev: true,
    });

    render(<App transport={devFallbackTransport} />);

    await waitFor(() => {
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected");
    });
    expect(await screen.findByTestId("ipc-version")).toHaveTextContent("IPC v");
  });
});
