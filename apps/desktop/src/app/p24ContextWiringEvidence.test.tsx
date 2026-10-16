import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import type { TimelineRow } from "../features/timeline/timelineStore";

describe("P2.4 Context & Memory Product Wiring Evidence (TASKS.md §A14)", () => {
  let transport: InMemoryTransport;

  const mockSnapshotTurn1: ContextSnapshotDto = {
    turn_id: "trn_turn00000000000001",
    thread_id: "thr_fixture000000001",
    memory_mode: "long_term",
    created_at: 1700000000100,
    passthrough: false,
    budget: {
      identity_limit_tokens: 1024,
      memory_limit_tokens: 2048,
      prompt_tokens: 60,
      identity_tokens: 120,
      memory_tokens: 450,
      total_tokens: 630,
    },
    identity: [
      {
        document_id: "idd_test000000000001",
        kind: "name",
        tokens: 120,
      },
    ],
    memories: [
      {
        memory_id: "mem_pref_rust00000001",
        kind: "preference",
        scope_kind: "global",
        scope_target: null,
        confidence: 100,
        explicit: true,
        tokens: 450,
        score: 0.94,
        why_selected: "matched terms: [rust, memory]; bm25: 1.820; explicit boost: +0.2",
        provenance_thread_id: "thr_history000000001",
        provenance_turn_id: "trn_history000000001",
      },
    ],
    dropped: [
      {
        memory_id: "mem_dropped000000001",
        tokens: 3000,
        rank: 3,
        reason: "budget_exhausted",
      },
    ],
    degraded: null,
    rendered_prompt: null,
  };

  const mockSnapshotTurn2Off: ContextSnapshotDto = {
    turn_id: "trn_turn00000000000002",
    thread_id: "thr_fixture000000001",
    memory_mode: "off",
    created_at: 1700000000200,
    passthrough: true,
    budget: {
      identity_limit_tokens: 1024,
      memory_limit_tokens: 0,
      prompt_tokens: 30,
      identity_tokens: 0,
      memory_tokens: 0,
      total_tokens: 30,
    },
    identity: [],
    memories: [],
    dropped: [],
    degraded: null,
    rendered_prompt: null,
  };

  const fixtureRows: TimelineRow[] = [
    {
      id: "trn_turn00000000000001",
      kind: "assistant-message",
      text: "I remember you prefer Rust and concise types.",
      status: null,
      permission: null,
      streaming: false,
    },
    {
      id: "trn_turn00000000000002",
      kind: "assistant-message",
      text: "Memory mode is off for this query.",
      status: null,
      permission: null,
      streaming: false,
    },
  ];

  beforeEach(() => {
    transport = new InMemoryTransport({
      initialContextSnapshots: [mockSnapshotTurn1, mockSnapshotTurn2Off],
    });
  });

  it("1. ActivityRail activates Memory Vault navigation item", async () => {
    render(<App transport={transport} />);

    const memoryRailBtn = screen.getByTestId("rail-memory");
    expect(memoryRailBtn).toBeTruthy();
    expect(memoryRailBtn.getAttribute("aria-disabled")).toBe("false");

    // Click memory rail item
    fireEvent.click(memoryRailBtn);

    // Workbench transitions to MemoryPane
    await waitFor(() => {
      expect(screen.getByTestId("workbench-memory")).toBeTruthy();
      expect(screen.getByTestId("memory-pane")).toBeTruthy();
    });
  });

  it("2. MemoryPane renders zero fake cards when empty and displays real records", async () => {
    render(<App transport={transport} />);

    // Navigate to memory rail
    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("memory-pane")).toBeTruthy());

    // Initially empty Personal Vault
    expect(screen.getByTestId("memory-empty")).toBeTruthy();
    expect(screen.getByTestId("memory-empty").textContent).toContain("No memories yet.");
  });

  it("3. Memory lifecycle journey: Propose -> Confirm -> Correct -> Forget", async () => {
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("memory-pane")).toBeTruthy());

    // A. Propose Candidate Memory
    fireEvent.click(screen.getByTestId("memory-add-button"));
    const contentInput = screen.getByTestId("memory-content-input");
    fireEvent.change(contentInput, { target: { value: "User prefers async/await in Rust" } });

    // Explicit check is true by default, uncheck to make candidate
    const explicitCheck = screen.getByTestId("memory-explicit-check");
    fireEvent.click(explicitCheck);

    fireEvent.click(screen.getByTestId("memory-submit-btn"));

    await waitFor(() => {
      expect(screen.getByText("User prefers async/await in Rust")).toBeTruthy();
      expect(screen.getByText("Candidate")).toBeTruthy();
    });

    // B. Confirm the Candidate Memory
    const confirmBtn = screen.getByTestId("memory-confirm-btn");
    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(screen.getByText("Confirmed")).toBeTruthy();
    });

    // C. Correct the Confirmed Memory
    const correctBtn = screen.getByTestId("memory-correct-btn");
    fireEvent.click(correctBtn);

    const editInput = screen.getByTestId("memory-edit-input");
    fireEvent.change(editInput, { target: { value: "User prefers synchronous Rust with std::thread" } });

    fireEvent.click(screen.getByTestId("memory-save-correct-btn"));

    await waitFor(() => {
      expect(screen.getByText("User prefers synchronous Rust with std::thread")).toBeTruthy();
      expect(screen.getByText("Superseded")).toBeTruthy();
    });

    // D. Forget the new Confirmed Memory
    // The new confirmed memory is first in list
    const forgetBtns = screen.getAllByTestId("memory-forget-btn");
    fireEvent.click(forgetBtns[0]!);

    await waitFor(() => {
      expect(screen.getByText("Forgotten")).toBeTruthy();
    });
  });

  it("4. Keyboard-selects a memory row and shows localized Inspector details", async () => {
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("memory-pane")).toBeTruthy());

    fireEvent.click(screen.getByTestId("memory-add-button"));
    fireEvent.change(screen.getByTestId("memory-content-input"), {
      target: { value: "User prefers a compact desktop workbench" },
    });
    fireEvent.click(screen.getByTestId("memory-submit-btn"));

    const row = await screen.findByRole("row", {
      name: /User prefers a compact desktop workbench/,
    });
    expect(row).toHaveAttribute("aria-selected", "false");
    expect(row).toHaveAttribute("tabindex", "0");
    fireEvent.focus(row);

    fireEvent.click(screen.getByTestId("memory-add-button"));
    fireEvent.change(screen.getByTestId("memory-content-input"), {
      target: { value: "User keeps provenance visible" },
    });
    fireEvent.click(screen.getByTestId("memory-submit-btn"));
    const firstRow = await screen.findByRole("row", {
      name: /User keeps provenance visible/,
    });
    expect(firstRow).toHaveAttribute("tabindex", "-1");
    fireEvent.keyDown(row, { key: "ArrowUp" });
    expect(firstRow).toHaveFocus();
    expect(firstRow).toHaveAttribute("tabindex", "0");
    expect(row).toHaveAttribute("tabindex", "-1");
    fireEvent.keyDown(firstRow, { key: "End" });
    expect(row).toHaveFocus();

    const correct = screen.getAllByTestId("memory-correct-btn")[1]!;
    fireEvent.keyDown(correct, { key: "Enter" });
    expect(row).toHaveAttribute("aria-selected", "false");

    fireEvent.keyDown(row, { key: "Enter" });

    await waitFor(() => {
      expect(row).toHaveAttribute("aria-selected", "true");
      const details = screen.getByTestId("inspector-memory-details");
      expect(details).toHaveTextContent("Fact");
      expect(details).toHaveTextContent("Global");
      expect(details).toHaveTextContent("Explicit");
      expect(details).toHaveTextContent("Confirmed");
      expect(details).toHaveTextContent("100%");
    });

    fireEvent.click(correct);
    const editInput = screen.getByTestId("memory-edit-input");
    expect(fireEvent.keyDown(editInput, { key: " " })).toBe(true);
    expect(fireEvent.keyDown(editInput, { key: "Enter" })).toBe(true);
    fireEvent.change(editInput, {
      target: { value: "User prefers a compact workbench with provenance" },
    });
    fireEvent.click(screen.getByTestId("memory-save-correct-btn"));

    await waitFor(() => {
      const details = screen.getByTestId("inspector-memory-details");
      expect(details).toHaveTextContent("Superseded");
      expect(details).toHaveTextContent(/mem_/);
    });
  });

  it("5. Selecting a thread from Memory returns to the thread workbench", async () => {
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("memory-pane")).toBeTruthy());

    fireEvent.click(screen.getByTestId("thread-thr_fixture000000001"));

    await waitFor(() => {
      expect(screen.queryByTestId("memory-pane")).toBeNull();
      expect(screen.getByText("用三条要点概括 P0.2 IPC 契约。")).toBeTruthy();
    });
  });

  it("6. Clears a hidden memory selection when filters change", async () => {
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    fireEvent.click(screen.getByTestId("memory-add-button"));
    fireEvent.change(screen.getByTestId("memory-content-input"), {
      target: { value: "User prefers visible selection state" },
    });
    fireEvent.click(screen.getByTestId("memory-submit-btn"));

    const row = await screen.findByRole("row", {
      name: /User prefers visible selection state/,
    });
    fireEvent.click(row);
    expect(screen.getByTestId("inspector-memory-details")).toBeTruthy();

    fireEvent.change(screen.getByTestId("memory-state-filter"), {
      target: { value: "candidate" },
    });

    await waitFor(() => {
      expect(screen.queryByTestId("inspector-memory-details")).toBeNull();
      expect(screen.getByText("No memory selected")).toBeTruthy();
    });
  });

  it("7. Surfaces memory command failures without exposing transport details", async () => {
    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "propose_memory") {
        throw new Error("provider-secret-token-should-not-render");
      }
      return undefined;
    });
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    fireEvent.click(screen.getByTestId("memory-add-button"));
    fireEvent.change(screen.getByTestId("memory-content-input"), {
      target: { value: "A valid durable preference" },
    });
    fireEvent.click(screen.getByTestId("memory-submit-btn"));

    const alert = await screen.findByTestId("memory-operation-error");
    expect(alert).toHaveAttribute("role", "alert");
    expect(alert).toHaveTextContent("The memory operation failed");
    expect(alert).not.toHaveTextContent("provider-secret-token-should-not-render");
    expect(screen.getByTestId("memory-propose-form")).toBeTruthy();
  });

  it("8. Configures Agent memory mode through UI", async () => {
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("agent-memory-mode-select")).toBeTruthy());

    const select = screen.getByTestId("agent-memory-mode-select") as HTMLSelectElement;
    expect(select.value).toBe("session");

    // Change to long_term
    fireEvent.change(select, { target: { value: "long_term" } });

    await waitFor(() => {
      expect(select.value).toBe("long_term");
    });
  });

  it("8b. Surfaces agent memory mode update failures in the operation banner", async () => {
    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "configure_agent") {
        throw new Error("provider-secret-token-should-not-render");
      }
      return undefined;
    });
    render(<App transport={transport} />);

    fireEvent.click(screen.getByTestId("rail-memory"));
    await waitFor(() => expect(screen.getByTestId("agent-memory-mode-select")).toBeTruthy());

    const select = screen.getByTestId("agent-memory-mode-select") as HTMLSelectElement;
    expect(select.value).toBe("session");

    fireEvent.change(select, { target: { value: "long_term" } });

    const alert = await screen.findByTestId("memory-operation-error");
    expect(alert).toHaveAttribute("role", "alert");
    expect(alert).toHaveTextContent("The memory operation failed");
    expect(alert).not.toHaveTextContent("provider-secret-token-should-not-render");
    // The store adopts the new mode only after Core confirms, so the select
    // stays on the old value instead of silently diverging from Core.
    await waitFor(() => expect(select.value).toBe("session"));
  });

  it("9. Turn selection loads Context snapshot into Inspector with explainability", async () => {
    render(
      <App
        transport={transport}
        fixtureTimelineRows={[
          {
            id: "thr_fixture000000001",
            rows: fixtureRows,
          },
        ]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("I remember you prefer Rust and concise types.")).toBeTruthy();
    });

    // Switch inspector tab to Context
    const contextTab = screen.getByTestId("inspector-tab-context");
    fireEvent.click(contextTab);

    // Focus Turn 1
    fireEvent.mouseDown(screen.getByText("I remember you prefer Rust and concise types."));

    await waitFor(() => {
      expect(screen.getByTestId("context-budget")).toBeTruthy();
      expect(screen.getByTestId("memory-why-selected").textContent).toContain("matched terms: [rust, memory]");
      expect(screen.getByTestId("memory-provenance").textContent).toContain("thr_history000000001");
      expect(screen.getByTestId("context-dropped-list").textContent).toMatch(
        /budget_exhausted|budget exhausted|预算耗尽/,
      );
    });

    // Focus Turn 2 (memory_mode === "off")
    fireEvent.mouseDown(screen.getByText("Memory mode is off for this query."));

    await waitFor(() => {
      expect(screen.getByTestId("context-memory-off")).toBeTruthy();
    });
  });

  it("10. Eliminates race conditions: discards delayed responses from prior turn selections", async () => {
    let resolveTurn1: ((snap: ContextSnapshotDto | null) => void) | null = null;
    const pendingTurn1Promise = new Promise<ContextSnapshotDto | null>((resolve) => {
      resolveTurn1 = resolve;
    });

    // Intercept get_context_snapshot command
    transport.setCommandHandler((cmd) => {
      if (cmd.kind === "get_context_snapshot") {
        const payload = cmd.payload as { turn_id?: string | null };
        if (payload.turn_id === "trn_turn00000000000001") {
          return pendingTurn1Promise;
        }
        if (payload.turn_id === "trn_turn00000000000002") {
          return mockSnapshotTurn2Off;
        }
      }
      return undefined;
    });

    render(
      <App
        transport={transport}
        fixtureTimelineRows={[
          {
            id: "thr_fixture000000001",
            rows: fixtureRows,
          },
        ]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText("I remember you prefer Rust and concise types.")).toBeTruthy();
    });

    fireEvent.click(screen.getByTestId("inspector-tab-context"));

    // 1. Select Turn 1 (which hangs pending)
    fireEvent.mouseDown(screen.getByText("I remember you prefer Rust and concise types."));

    // Inspector shows loading state
    await waitFor(() => {
      expect(screen.getByTestId("context-loading")).toBeTruthy();
    });

    // 2. Quickly select Turn 2 before Turn 1 finishes
    fireEvent.mouseDown(screen.getByText("Memory mode is off for this query."));

    // Turn 2 resolves immediately
    await waitFor(() => {
      expect(screen.getByTestId("context-memory-off")).toBeTruthy();
    });

    // 3. Now let Turn 1 resolve belatedly
    resolveTurn1!(mockSnapshotTurn1);

    // Give time for any stale microtask to execute
    await new Promise((r) => setTimeout(r, 50));

    // Turn 2 view MUST NOT be overwritten by Turn 1 stale response!
    expect(screen.getByTestId("context-memory-off")).toBeTruthy();
    expect(screen.queryByTestId("context-budget")).toBeNull();
  });
});
