/**
 * P0.4 acceptance evidence (docs/IMPLEMENTATION_PLAN.md, ADR 0008).
 *
 * Every test is deterministic: no sleeps, no layout, no network. The
 * window math is injected (`viewportHeight`, `measured`) because jsdom
 * has no geometry; the pure-math half of the evidence lives in
 * `virtualWindow.test.ts`.
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import { Timeline } from "../features/timeline/Timeline";
import { createTimelineStore } from "../features/timeline/timelineStore";
import {
  approvalThread,
  failureThread,
  hundredThousandRowThread,
  olderHistory,
} from "../fixtures/timeline";
import { InMemoryTransport } from "../ipc/inMemoryTransport";

function scrollerOf(container: HTMLElement) {
  return container.querySelector<HTMLElement>("[data-testid='timeline-scroller']")!;
}

function mountedRowIds(container: HTMLElement): string[] {
  return Array.from(
    container.querySelectorAll<HTMLElement>("[data-row-id]"),
  ).map((element) => element.dataset.rowId!);
}

describe("P0.4 evidence", () => {
  it("100,000 synthetic rows remain interactive", async () => {
    const { container } = render(
      <App transport={new InMemoryTransport()} includeHugeThread timelineViewportHeight={600} />,
    );

    fireEvent.click(screen.getByTestId(`thread-${hundredThousandRowThread().id}`));
    await waitFor(() => {
      expect(scrollerOf(container).dataset.rowCount).toBe("100000");
    });

    // Only the window (plus overscan) is mounted, not the whole list.
    const windowSize = Number(scrollerOf(container).dataset.windowSize);
    expect(windowSize).toBeGreaterThan(0);
    expect(windowSize).toBeLessThanOrEqual(60);
    expect(mountedRowIds(container)).toHaveLength(windowSize);

    // Keyboard navigation still works at this size.
    const scroller = scrollerOf(container);
    scroller.focus();
    fireEvent.keyDown(scroller, { key: "ArrowDown" });
    await waitFor(() => {
      expect(document.activeElement?.getAttribute("data-row-id")).toBe("big-0");
    });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    await waitFor(() => {
      expect(document.activeElement?.getAttribute("data-row-id")).toBe("big-1");
    });
  });

  it("focus survives row recycling across Home/End jumps", async () => {
    const { container } = render(
      <App transport={new InMemoryTransport()} includeHugeThread timelineViewportHeight={600} />,
    );
    fireEvent.click(screen.getByTestId(`thread-${hundredThousandRowThread().id}`));
    await waitFor(() => {
      expect(scrollerOf(container).dataset.rowCount).toBe("100000");
    });

    const scroller = scrollerOf(container);
    scroller.focus();
    fireEvent.keyDown(scroller, { key: "ArrowDown" });
    await waitFor(() => {
      expect(document.activeElement?.getAttribute("data-row-id")).toBe("big-0");
    });

    // Jump to the end: big-0 unmounts.
    fireEvent.keyDown(document.activeElement!, { key: "End" });
    await waitFor(() => {
      expect(document.activeElement?.getAttribute("data-row-id")).toBe("big-99999");
    });
    expect(screen.queryByText("deterministic question 0")).toBeNull();

    // Jump home: big-0 remounts and retakes focus.
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    await waitFor(() => {
      expect(document.activeElement?.getAttribute("data-row-id")).toBe("big-0");
    });
  });

  it("a streamed delta mutates only its own row, never the mounted timeline", async () => {
    const store = createTimelineStore(
      Array.from({ length: 2000 }, (_, index) => ({
        id: `r-${index}`,
        kind: index % 2 === 0 ? ("user-message" as const) : ("assistant-message" as const),
        text: `row ${index}`,
        status: null,
        permission: null,
        streaming: index === 1999,
      })),
    );
    const measured = new Map(
      store.getSnapshot().rows.map((row) => [row.id, 24] as const),
    );

    const anchors = { onFocusChange: () => undefined, onPermissionDecision: () => undefined };
    const { container } = render(
      <Timeline
        store={store}
        focusedRowId={null}
        anchorRowId={null}
        viewportHeight={600}
        measured={measured}
        {...anchors}
      />,
    );

    const before = new Map(
      Array.from(container.querySelectorAll<HTMLElement>("[data-row-id]")).map(
        (element) => [element.dataset.rowId!, element.textContent ?? ""],
      ),
    );
    expect(before.size).toBeGreaterThan(10);

    const mutations: MutationRecord[] = [];
    const observer = new MutationObserver((records) => mutations.push(...records));
    observer.observe(scrollerOf(container), {
      subtree: true,
      childList: true,
      characterData: true,
    });

    store.appendDelta("r-1999", " streamed tail");

    await waitFor(() => {
      expect(mutations.length).toBeGreaterThan(0);
    });
    observer.disconnect();

    // The only text change lives inside the streamed row's own subtree.
    for (const record of mutations) {
      const target = record.target as HTMLElement;
      const owner =
        target.nodeType === 1
          ? (target as HTMLElement).closest("[data-row-id]")
          : (target.parentElement?.closest("[data-row-id]") ?? null);
      expect((owner as HTMLElement)?.getAttribute("data-row-id")).toBe("r-1999");
    }

    // Every other mounted row kept its exact text — no full-timeline
    // rerender happened.
    const after = new Map(
      Array.from(container.querySelectorAll<HTMLElement>("[data-row-id]")).map(
        (element) => [element.dataset.rowId!, element.textContent ?? ""],
      ),
    );
    for (const [rowId, text] of before) {
      if (rowId === "r-1999") continue;
      expect(after.get(rowId)).toBe(text);
    }
    expect(store.getRow("r-1999")?.text).toContain("streamed tail");
  });

  it("prepending older history keeps the viewport anchored", async () => {
    const store = createTimelineStore(
      Array.from({ length: 100 }, (_, index) => ({
        id: `p-${index}`,
        kind: index % 2 === 0 ? ("user-message" as const) : ("assistant-message" as const),
        text: `row ${index}`,
        status: null,
        permission: null,
        streaming: false,
      })),
    );
    const prepend = olderHistory(10);
    const measured = new Map<string, number>([
      ...store.getSnapshot().rows.map((row) => [row.id, 20] as const),
      ...prepend.map((row) => [row.id, 20] as const),
    ]);

    const seenFirst: string[] = [];
    const { container } = render(
      <Timeline
        store={store}
        focusedRowId={null}
        anchorRowId={null}
        viewportHeight={200}
        measured={measured}
        onFocusChange={() => undefined}
        onPermissionDecision={() => undefined}
        onFirstVisibleChange={(rowId) => seenFirst.push(rowId)}
      />,
    );

    // Scroll so row 50 is first visible (20px rows: 1000px).
    const scroller = scrollerOf(container);
    scroller.scrollTop = 1000;
    fireEvent.scroll(scroller);
    await waitFor(() => expect(seenFirst.at(-1)).toBe("p-50"));

    store.prependRows(prepend);

    // The anchor keeps its position: scrollTop shifts by exactly the
    // prepended block height (10 rows × 20px = 200px).
    await waitFor(() => {
      expect(scroller.scrollTop).toBe(1200);
    });
    expect(seenFirst.at(-1)).toBe("p-50");
    expect(store.getSnapshot().rows[0]?.id).toBe("old-10");
  });

  it("thread reopen restores the remembered scroll anchor", async () => {
    const huge = hundredThousandRowThread().id;
    const { container } = render(
      <App transport={new InMemoryTransport()} includeHugeThread timelineViewportHeight={600} />,
    );
    fireEvent.click(screen.getByTestId(`thread-${huge}`));
    await waitFor(() => {
      expect(scrollerOf(container).dataset.rowCount).toBe("100000");
    });

    const scroller = scrollerOf(container);
    scroller.scrollTop = 700_000;
    fireEvent.scroll(scroller);
    await waitFor(() => {
      expect(scroller.scrollTop).toBeGreaterThan(0);
    });

    // Navigate away and back.
    fireEvent.click(screen.getByTestId(`thread-${failureThread.id}`));
    await waitFor(() => {
      expect(scrollerOf(container).dataset.rowCount).toBe("4");
    });
    fireEvent.click(screen.getByTestId(`thread-${huge}`));
    await waitFor(() => {
      expect(scrollerOf(container).dataset.rowCount).toBe("100000");
    });

    // The scroll returned to the remembered position, not to the top.
    await waitFor(() => {
      expect(scrollerOf(container).scrollTop).toBeGreaterThan(600_000);
      expect(scrollerOf(container).scrollTop).toBeLessThan(800_000);
    });
  });

  it("an inline approval records the decision and announces it", async () => {
    render(<App transport={new InMemoryTransport()} />);
    fireEvent.click(screen.getByTestId(`thread-${approvalThread.id}`));

    const permissionRow = document.querySelector("[data-row-kind='permission']");
    expect(permissionRow).not.toBeNull();
    expect(permissionRow?.textContent).toContain("cargo tree --workspace --edges all");

    fireEvent.click(screen.getByTestId("approve"));
    await waitFor(() => {
      expect(
        document.querySelector("[data-row-kind='permission']")?.textContent,
      ).toContain("approved");
    });
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("Permission approved");
    });
  });

  it("keyboard shortcuts decide the approval from the focused row", async () => {
    render(<App transport={new InMemoryTransport()} />);
    fireEvent.click(screen.getByTestId(`thread-${approvalThread.id}`));

    const row = document.querySelector<HTMLElement>("[data-row-kind='permission']");
    row?.focus();
    fireEvent.keyDown(row!, { key: "y" });

    await waitFor(() => {
      expect(
        document.querySelector("[data-row-kind='permission']")?.textContent,
      ).toContain("approved");
    });
    expect(screen.getByTestId("inspector-close")).toBeInTheDocument();
  });

  it("inspector resizing is keyboard-operable and clamped to the token range", async () => {
    render(<App transport={new InMemoryTransport()} />);
    const handle = screen.getByTestId("inspector-resize");
    expect(handle).toHaveAttribute("aria-valuenow", "360");

    handle.focus();
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    fireEvent.keyDown(handle, { key: "ArrowLeft" });
    await waitFor(() => {
      expect(handle).toHaveAttribute("aria-valuenow", "328");
    });

    // Widening past the maximum clamps at the token bound (640).
    for (let n = 0; n < 40; n += 1) {
      fireEvent.keyDown(handle, { key: "ArrowRight" });
    }
    await waitFor(() => {
      expect(handle).toHaveAttribute("aria-valuenow", "640");
    });
  });

  it("unknown events render as preserved, inspectable rows", () => {
    render(<App transport={new InMemoryTransport()} />);
    const unknown = document.querySelector("[data-row-kind='unknown']");
    expect(unknown?.textContent).toContain("acp.update.plan");
    expect(unknown?.textContent).toContain("preserved verbatim");
  });

  it("the new-activity affordance appears only when scrolled away", async () => {
    const store = createTimelineStore(
      Array.from({ length: 100 }, (_, index) => ({
        id: `n-${index}`,
        kind: index % 2 === 0 ? ("user-message" as const) : ("assistant-message" as const),
        text: `row ${index}`,
        status: null,
        permission: null,
        streaming: false,
      })),
    );
    const measured = new Map(store.getSnapshot().rows.map((row) => [row.id, 20] as const));
    // The row appended later reports its height too, keeping the list
    // uniform for the exact-scroll assertion below.
    measured.set("n-new", 20);
    const { container } = render(
      <Timeline
        store={store}
        focusedRowId={null}
        anchorRowId={null}
        viewportHeight={200}
        measured={measured}
        onFocusChange={() => undefined}
        onPermissionDecision={() => undefined}
      />,
    );

    const scroller = scrollerOf(container);
    expect(screen.queryByTestId("new-activity")).toBeNull();

    // Scroll away from the end, then new rows land.
    scroller.scrollTop = 500;
    fireEvent.scroll(scroller);
    store.appendRow({
      id: "n-new",
      kind: "assistant-message",
      text: "late reply",
      status: null,
      permission: null,
      streaming: false,
    });

    const affordance = await screen.findByTestId("new-activity");
    expect(affordance).toHaveTextContent("1 new row");

    fireEvent.click(affordance);
    await waitFor(() => {
      expect(scroller.scrollTop).toBe(store.getSnapshot().rows.length * 20 - 200);
    });
    await waitFor(() => {
      expect(screen.queryByTestId("new-activity")).toBeNull();
    });
  });
});
