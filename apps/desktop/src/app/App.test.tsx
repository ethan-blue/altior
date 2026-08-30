import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { App } from "./App";

describe("App workbench shell", () => {
  it("shows the negotiated IPC version", async () => {
    render(<App transport={new InMemoryTransport()} />);

    expect(await screen.findByTestId("ipc-version")).toHaveTextContent("IPC v");
  });

  it("keeps the P0.1 protocol evidence: capabilities and unknown-event diagnostics", async () => {
    render(<App transport={new InMemoryTransport()} />);

    const diagnostics = await screen.findByTestId("protocol-diagnostics");
    expect(diagnostics).toHaveTextContent("event.streaming: supported");

    // The fixture stream replays in sequence order with bounded
    // diagnostics for unknown events.
    await waitFor(() => {
      expect(diagnostics).toHaveTextContent("#1");
      expect(diagnostics).toHaveTextContent("turn.started");
    });
    expect(diagnostics).toHaveTextContent("#2");
    expect(diagnostics).toHaveTextContent("usage.stats.snapshot");
  });

  it("renders the five shell regions with the thread header and composer", async () => {
    render(<App transport={new InMemoryTransport()} />);

    expect(screen.getByRole("navigation", { name: "Activity" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Threads" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Contract fixture walkthrough",
    );
    expect(screen.getByRole("textbox", { name: "Composer" })).toBeEnabled();
    await waitFor(() =>
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected"),
    );
    expect(await screen.findByTestId("ipc-version")).toHaveTextContent("IPC v");
  });

  it("sends from the composer and streams the deterministic fixture reply", async () => {
    render(<App transport={new InMemoryTransport()} />);

    const composer = screen.getByTestId("composer");
    fireEvent.change(composer, { target: { value: "What changed in P0.4?" } });
    fireEvent.click(screen.getByTestId("send"));

    const userRow = document.querySelector(
      "[data-row-kind='user-message'][data-row-id='send-1']",
    );
    expect(userRow?.textContent).toContain("What changed in P0.4?");

    await waitFor(() => {
      const reply = document.querySelector("[data-row-id='send-1-reply']");
      expect(reply?.textContent).toContain(
        "Frames are length-prefixed; sessions replay through a retained window; reload never stops a turn.",
      );
    });
  });

  it("preserves composer drafts across thread navigation", () => {
    render(<App transport={new InMemoryTransport()} />);

    const composer = screen.getByTestId("composer") as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: "draft in progress" } });
    fireEvent.click(screen.getByTestId("thread-fixture/failure"));
    expect(screen.getByTestId("composer")).toHaveValue("");

    fireEvent.click(screen.getByTestId("thread-fixture/standard"));
    expect(screen.getByTestId("composer")).toHaveValue("draft in progress");
  });

  it("toggles the theme on the shell root", () => {
    render(<App transport={new InMemoryTransport()} />);
    const root = document.querySelector("[data-theme]");
    expect(root).toHaveAttribute("data-theme", "light");
    fireEvent.click(screen.getByTestId("theme-toggle"));
    expect(root).toHaveAttribute("data-theme", "dark");
  });
});
