import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { App } from "./App";

describe("App fixture shell", () => {
  it("shows the negotiated IPC version and capabilities", async () => {
    render(<App transport={new InMemoryTransport()} />);

    expect(await screen.findByTestId("ipc-version")).toHaveTextContent("IPC v2");
    expect(screen.getByText("event.streaming: supported")).toBeInTheDocument();
  });

  it("replays fixture events in stream sequence order", async () => {
    render(<App transport={new InMemoryTransport()} />);

    const timeline = await screen.findByRole("main", { name: "Event timeline" });
    const rows = within(timeline).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("#1");
    expect(rows[0]?.textContent).toContain("turn.started");
    expect(rows[1]?.textContent).toContain("#2");
    expect(rows[1]?.textContent).toContain("usage.stats.snapshot");
  });

  it("renders unknown events as bounded diagnostic rows", async () => {
    render(<App transport={new InMemoryTransport()} />);

    const diagnostic = await screen.findByText(/input_tokens/, { selector: "code" });
    expect(diagnostic).toHaveTextContent("usage.stats.snapshot");
  });

  it("keeps the composer disabled until turn input ships", () => {
    render(<App transport={new InMemoryTransport()} />);

    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(screen.getByRole("textbox")).toBeDisabled();
  });
});
