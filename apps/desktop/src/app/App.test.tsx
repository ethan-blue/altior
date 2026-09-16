import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { failureThread, standardThread } from "../fixtures/timeline";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { App } from "./App";

/** Renders App and waits until the authoritative thread list has loaded. */
async function renderWithThreads(transport: () => InMemoryTransport) {
  const view = render(<App transport={transport()} />);
  await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough/ });
  return view;
}

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
    // Threads arrive from the Core list response; the header renders once
    // the selected conversation is known (A03: no local defaults).
    expect(await screen.findByRole("heading", { level: 1 })).toHaveTextContent(
      "Contract fixture walkthrough",
    );
    expect(screen.getByRole("textbox", { name: "Composer" })).toBeEnabled();
    await waitFor(() =>
      expect(screen.getByTestId("status-bar")).toHaveTextContent("Core · connected"),
    );
    expect(await screen.findByTestId("ipc-version")).toHaveTextContent("IPC v");
  });

  it("shows the honest empty state on a clean vault (A03)", async () => {
    render(
      <App
        transport={new InMemoryTransport({ initialThreads: [], initialAgents: [] })}
      />,
    );

    const empty = await screen.findByTestId("empty-state-none");
    expect(empty).toHaveTextContent("No conversations yet");

    // The empty pane says so too; no alpha/beta demo rows may appear.
    expect(screen.queryByText("alpha (ACP)")).toBeNull();
    expect(screen.getByText("No conversations yet.", { selector: "p" })).toBeInTheDocument();
    // The clean vault opens onboarding.
    expect(await screen.findByRole("dialog", { name: "Agent Onboarding" })).toBeInTheDocument();
  });

  it("an empty search result never unmounts the conversation being read (A03)", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);
    await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough/ });

    const filter = screen.getByTestId("thread-filter");
    fireEvent.change(filter, { target: { value: "zzz-no-such-thread" } });
    await waitFor(() => {
      expect(screen.getByText("No conversations match.")).toBeInTheDocument();
    });

    // The nav list is empty but the read conversation stays mounted with
    // its rows; this is the exact path behind the review's TypeError probe.
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Contract fixture walkthrough",
    );
    expect(
      document.querySelector("[data-row-id='trn_fixture000000101']"),
    ).not.toBeNull();
  });

  it("sends from the composer and streams the deterministic fixture reply", async () => {
    await renderWithThreads(() => new InMemoryTransport());

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

  it("keeps the draft when the prompt is not delivered (A04)", async () => {
    const transport = new InMemoryTransport({ autoStreamReplies: false });
    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "start_turn") {
        throw new Error("agent refused the turn");
      }
      return undefined;
    });
    await renderWithThreads(() => transport);

    const composer = screen.getByTestId("composer");
    fireEvent.change(composer, { target: { value: "please keep me" } });
    fireEvent.click(screen.getByTestId("send"));

    // The text stays editable in the composer; the timeline explains the
    // failure without pretending the turn was saved.
    await waitFor(() => {
      expect(screen.getByTestId("composer")).toHaveValue("please keep me");
    });
    expect(
      document.querySelector("[data-row-kind='error']")?.textContent,
    ).toContain("Prompt not delivered");
  });

  it("clears the draft once the prompt is admitted (A04)", async () => {
    await renderWithThreads(() => new InMemoryTransport({ autoStreamReplies: false }));

    const composer = screen.getByTestId("composer");
    fireEvent.change(composer, { target: { value: "off you go" } });
    fireEvent.click(screen.getByTestId("send"));

    await waitFor(() => {
      expect(screen.getByTestId("composer")).toHaveValue("");
    });
  });

  it("preserves composer drafts across thread navigation", async () => {
    await renderWithThreads(() => new InMemoryTransport());

    const composer = screen.getByTestId("composer") as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: "draft in progress" } });
    fireEvent.click(screen.getByTestId(`thread-${failureThread.id}`));
    expect(screen.getByTestId("composer")).toHaveValue("");

    fireEvent.click(screen.getByTestId(`thread-${standardThread.id}`));
    expect(screen.getByTestId("composer")).toHaveValue("draft in progress");
  });

  it("toggles the theme on the shell root", async () => {
    await renderWithThreads(() => new InMemoryTransport());
    const root = document.querySelector("[data-theme]");
    expect(root).toHaveAttribute("data-theme", "light");
    fireEvent.click(screen.getByTestId("theme-toggle"));
    expect(root).toHaveAttribute("data-theme", "dark");
  });
});
