import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { failureThread, standardThread, streamingReplyChunks } from "../fixtures/timeline";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { App } from "./App";

afterEach(() => {
  localStorage.clear();
  cleanup();
});

/** Renders App and waits until the authoritative thread list has loaded. */
async function renderWithThreads(transport: () => InMemoryTransport) {
  const view = render(<App transport={transport()} />);
  await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough|契约夹具演练/ });
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
      "契约夹具演练",
    );
    expect(screen.getByRole("textbox", { name: "Composer" })).toBeEnabled();
    await waitFor(() =>
      expect(screen.getByTestId("status-bar")).toHaveTextContent(/connected|已连接/),
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

  it("re-opens onboarding with internationalized notice when clicking new thread without agents", async () => {
    localStorage.clear();
    render(
      <App
        transport={new InMemoryTransport({ initialThreads: [], initialAgents: [] })}
      />,
    );

    // Initial clean vault opens onboarding modal without notice
    const initialDialog = await screen.findByRole("dialog", { name: "Agent Onboarding" });
    expect(initialDialog).toBeInTheDocument();
    expect(screen.queryByTestId("onboarding-notice")).toBeNull();

    // Close onboarding modal
    fireEvent.click(screen.getByTestId("onboarding-close"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Agent Onboarding" })).toBeNull();
    });

    // Clicking "New Thread" without any configured agent re-opens onboarding with i18n notice (en)
    fireEvent.click(screen.getByTestId("new-thread"));
    const reopenedDialog = await screen.findByRole("dialog", { name: "Agent Onboarding" });
    expect(reopenedDialog).toBeInTheDocument();
    const noticeEn = screen.getByTestId("onboarding-notice");
    expect(noticeEn).toBeInTheDocument();
    expect(noticeEn).toHaveTextContent("Please configure an agent before creating a new thread.");

    // Close onboarding modal again
    fireEvent.click(screen.getByTestId("onboarding-close"));
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "Agent Onboarding" })).toBeNull();
    });

    // Switch locale to zh-CN via settings
    fireEvent.click(screen.getByTestId("rail-settings"));
    await screen.findByTestId("settings-modal");
    fireEvent.change(screen.getByTestId("settings-locale-select"), {
      target: { value: "zh-CN" },
    });
    fireEvent.click(screen.getByTestId("settings-close-btn"));

    // Clicking "New Thread" again re-opens onboarding with Chinese notice
    fireEvent.click(screen.getByTestId("new-thread"));
    expect(await screen.findByRole("dialog", { name: "连接 ACP 代理" })).toBeInTheDocument();
    const noticeZh = screen.getByTestId("onboarding-notice");
    expect(noticeZh).toBeInTheDocument();
    expect(noticeZh).toHaveTextContent("创建新会话前请先配置代理。");
  });

  it("an empty search result never unmounts the conversation being read (A03)", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} />);
    await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough|契约夹具演练/ });

    const filter = screen.getByTestId("thread-filter");
    fireEvent.change(filter, { target: { value: "zzz-no-such-thread" } });
    await waitFor(() => {
      expect(screen.getByText("No conversations match.")).toBeInTheDocument();
    });

    // The nav list is empty but the read conversation stays mounted with
    // its rows; this is the exact path behind the review's TypeError probe.
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "契约夹具演练",
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
      // Coupled to fixture so zh/en chrome stays aligned without weakening stream evidence.
      expect(reply?.textContent).toContain(streamingReplyChunks.join(""));
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

  it("marks vault switcher button as aria-disabled", async () => {
    await renderWithThreads(() => new InMemoryTransport());
    const vaultBtn = screen.getByRole("button", { name: /personal vault|个人保险库/i });
    expect(vaultBtn).toHaveAttribute("aria-disabled", "true");
  });

  it("focuses search input and prevents default on Ctrl+K and Meta+K", async () => {
    await renderWithThreads(() => new InMemoryTransport());
    const searchInput = screen.getByTestId("global-search-input");
    expect(searchInput).not.toHaveFocus();

    // Ctrl+K
    const ctrlKEvent = new KeyboardEvent("keydown", {
      key: "k",
      ctrlKey: true,
      cancelable: true,
      bubbles: true,
    });
    const ctrlPreventSpy = vi.spyOn(ctrlKEvent, "preventDefault");
    window.dispatchEvent(ctrlKEvent);
    expect(ctrlPreventSpy).toHaveBeenCalled();
    expect(searchInput).toHaveFocus();

    // Blur
    searchInput.blur();
    expect(searchInput).not.toHaveFocus();

    // Meta+K (Cmd+K on macOS)
    const metaKEvent = new KeyboardEvent("keydown", {
      key: "k",
      metaKey: true,
      cancelable: true,
      bubbles: true,
    });
    const metaPreventSpy = vi.spyOn(metaKEvent, "preventDefault");
    window.dispatchEvent(metaKEvent);
    expect(metaPreventSpy).toHaveBeenCalled();
    expect(searchInput).toHaveFocus();

    // Key "K" with Ctrl
    searchInput.blur();
    const upperKEvent = new KeyboardEvent("keydown", {
      key: "K",
      ctrlKey: true,
      cancelable: true,
      bubbles: true,
    });
    window.dispatchEvent(upperKEvent);
    expect(searchInput).toHaveFocus();

    // Plain "k" without Ctrl/Meta should not focus or prevent default
    searchInput.blur();
    const plainKEvent = new KeyboardEvent("keydown", {
      key: "k",
      cancelable: true,
      bubbles: true,
    });
    const plainPreventSpy = vi.spyOn(plainKEvent, "preventDefault");
    window.dispatchEvent(plainKEvent);
    expect(plainPreventSpy).not.toHaveBeenCalled();
    expect(searchInput).not.toHaveFocus();
  });

  it("does not trigger thread filter search during IME composition until composition ends", async () => {
    const transport = new InMemoryTransport();
    const commandSpy = vi.spyOn(transport, "command");
    render(<App transport={transport} searchDebounceMs={50} />);
    await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough|契约夹具演练/ });

    const searchInput = screen.getByTestId("global-search-input");
    commandSpy.mockClear();

    // Start composition and change value
    fireEvent.compositionStart(searchInput);
    fireEvent.change(searchInput, { target: { value: "ceshi" } });

    // Wait longer than searchDebounceMs
    await new Promise((resolve) => setTimeout(resolve, 80));

    // During composition, search_threads should NOT be dispatched
    const searchCallsDuring = commandSpy.mock.calls.filter(
      (call) => call[0].kind === "search_threads",
    );
    expect(searchCallsDuring).toHaveLength(0);

    // End composition with the committed Chinese text
    fireEvent.change(searchInput, { target: { value: "测试" } });
    fireEvent.compositionEnd(searchInput, { target: { value: "测试" } });

    // After debounce finishes, search_threads is dispatched for "测试"
    await waitFor(() => {
      const searchCalls = commandSpy.mock.calls.filter(
        (call) => call[0].kind === "search_threads",
      );
      expect(searchCalls).toHaveLength(1);
      expect(searchCalls[0]?.[0].payload).toMatchObject({ query: "测试" });
    });
  });

  it("debounces rapid input changes and triggers search only once", async () => {
    const transport = new InMemoryTransport();
    const commandSpy = vi.spyOn(transport, "command");
    render(<App transport={transport} searchDebounceMs={60} />);
    await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough|契约夹具演练/ });

    const searchInput = screen.getByTestId("global-search-input");
    commandSpy.mockClear();

    // Rapid keystrokes within the debounce window
    fireEvent.change(searchInput, { target: { value: "a" } });
    fireEvent.change(searchInput, { target: { value: "ab" } });
    fireEvent.change(searchInput, { target: { value: "abc" } });

    // Before debounce delay elapses, no search_threads command dispatched
    const callsBefore = commandSpy.mock.calls.filter((c) => c[0].kind === "search_threads");
    expect(callsBefore).toHaveLength(0);

    // After debounce delay, search_threads is called exactly once with the final query
    await waitFor(() => {
      const callsAfter = commandSpy.mock.calls.filter((c) => c[0].kind === "search_threads");
      expect(callsAfter).toHaveLength(1);
      expect(callsAfter[0]?.[0].payload).toMatchObject({ query: "abc" });
    });

    // Verify no further calls fire
    await new Promise((resolve) => setTimeout(resolve, 90));
    const finalCalls = commandSpy.mock.calls.filter((c) => c[0].kind === "search_threads");
    expect(finalCalls).toHaveLength(1);
  });

  it("syncs search input with external appState.threadFilter changes without loop", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} searchDebounceMs={0} />);
    await screen.findByRole("heading", { level: 1, name: /Contract fixture walkthrough|契约夹具演练/ });

    const searchInput = screen.getByTestId("global-search-input");
    const paneFilter = screen.getByTestId("thread-filter");

    // Changing the thread filter from ThreadsPane updates the top bar search input
    fireEvent.change(paneFilter, { target: { value: "external-sync" } });
    await waitFor(() => {
      expect(searchInput).toHaveValue("external-sync");
    });
  });
});
