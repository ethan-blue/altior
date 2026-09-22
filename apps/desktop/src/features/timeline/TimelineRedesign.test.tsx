import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Timeline } from "./Timeline";
import { TimelineRowView } from "./TimelineRowView";
import { createTimelineStore, type TimelineRow } from "./timelineStore";

describe("Commercial UI Phase 1: Timeline Card Redesign & Approval Security Card", () => {
  it("renders centered card-based conversation stream without mechanical 72px table grid", () => {
    const store = createTimelineStore([
      {
        id: "r-user",
        kind: "user-message",
        text: "Can you analyze our dependencies?",
        status: null,
        permission: null,
        streaming: false,
      },
      {
        id: "r-assistant",
        kind: "assistant-message",
        text: "Sure, let me check the Cargo.lock file.",
        status: null,
        permission: null,
        streaming: false,
      },
    ]);

    const { container } = render(
      <Timeline
        store={store}
        focusedRowId={null}
        onFocusChange={vi.fn()}
        onPermissionDecision={vi.fn()}
        anchorRowId={null}
        viewportHeight={400}
      />,
    );

    // Verify slots exist and contain cards
    const slots = container.querySelectorAll("[class*='slot']");
    expect(slots.length).toBe(2);

    // Verify row cards exist with role badges
    const userRow = container.querySelector("[data-row-kind='user-message']");
    expect(userRow).not.toBeNull();
    expect(userRow?.querySelector("[class*='card']")).not.toBeNull();
    expect(userRow?.querySelector("[class*='cardHeader']")).not.toBeNull();
    expect(userRow?.querySelector("[class*='roleBadge']")).not.toBeNull();

    const assistantRow = container.querySelector("[data-row-kind='assistant-message']");
    expect(assistantRow).not.toBeNull();
    expect(assistantRow?.querySelector("[class*='card']")).not.toBeNull();
  });

  it("renders distinct visual hierarchies for user, assistant, tool, error, and unknown rows", () => {
    const rows: TimelineRow[] = [
      {
        id: "u1",
        kind: "user-message",
        text: "Hello",
        status: null,
        permission: null,
        streaming: false,
      },
      {
        id: "a1",
        kind: "assistant-message",
        text: "Thinking...",
        status: null,
        permission: null,
        streaming: true,
      },
      {
        id: "t1",
        kind: "tool",
        text: "cargo metadata --format-version 1",
        status: "completed",
        permission: null,
        streaming: false,
      },
      {
        id: "e1",
        kind: "error",
        text: "Failed to connect to host runtime",
        status: null,
        permission: null,
        streaming: false,
      },
      {
        id: "unk1",
        kind: "unknown",
        text: "acp.custom.event: preserved verbatim",
        status: null,
        permission: null,
        streaming: false,
      },
    ];
    const store = createTimelineStore(rows);

    const { container } = render(
      <Timeline
        store={store}
        focusedRowId={null}
        onFocusChange={vi.fn()}
        onPermissionDecision={vi.fn()}
        anchorRowId={null}
        viewportHeight={800}
      />,
    );

    // 1. User message hierarchy
    const userEl = container.querySelector("[data-row-kind='user-message']");
    expect(userEl?.textContent).toContain("Hello");

    // 2. Assistant message streaming indicator & caret
    const asstEl = container.querySelector("[data-row-kind='assistant-message']");
    expect(asstEl?.textContent).toContain("Thinking...");
    expect(asstEl?.querySelector("[class*='caret']")).not.toBeNull();
    expect(asstEl?.querySelector("[class*='streamingBadge']")).not.toBeNull();

    // 3. Tool block presentation
    const toolEl = container.querySelector("[data-row-kind='tool']");
    expect(toolEl?.textContent).toContain("cargo metadata");
    expect(toolEl?.textContent).toContain("completed");

    // 4. Error card presentation
    const errorEl = container.querySelector("[data-row-kind='error']");
    expect(errorEl?.textContent).toContain("Failed to connect to host runtime");
    expect(errorEl?.querySelector("[class*='errorTag']")).not.toBeNull();

    // 5. Unknown preserved presentation
    const unkEl = container.querySelector("[data-row-kind='unknown']");
    expect(unkEl?.textContent).toContain("acp.custom.event");
    expect(unkEl?.textContent).toContain("preserved verbatim");
    expect(unkEl?.querySelector("[class*='unknownTag']")).not.toBeNull();
  });

  describe("Permission Security Approval Card", () => {
    it("renders high-trust security card with context, risk awareness, and visual shortcut hints", () => {
      const store = createTimelineStore([
        {
          id: "perm-1",
          kind: "permission",
          text: "cargo test --all",
          status: null,
          permission: {
            requestedAction: "cargo test --all",
            scope: "project:altior/workspace",
            decision: null,
          },
          streaming: false,
        },
      ]);

      const onDecision = vi.fn();
      const onFocus = vi.fn();

      const { container } = render(
        <TimelineRowView
          store={store}
          rowId="perm-1"
          index={0}
          focused={false}
          onFocus={onFocus}
          onPermissionDecision={onDecision}
        />,
      );

      // Security risk awareness badge in header
      const riskBadge = container.querySelector("[class*='riskBadge']");
      expect(riskBadge).not.toBeNull();

      // Security risk notice banner in card body
      const riskNotice = container.querySelector("[class*='riskNotice']");
      expect(riskNotice).not.toBeNull();

      // Context: command code box & scope
      const monoCode = container.querySelector("[class*='mono']");
      expect(monoCode?.textContent).toBe("cargo test --all");

      const scopeEl = container.querySelector("[class*='scope']");
      expect(scopeEl?.textContent).toBe("project:altior/workspace");

      // Approve & Deny buttons with visual <kbd> hints
      const approveBtn = screen.getByTestId("approve");
      expect(approveBtn).toBeInTheDocument();
      expect(approveBtn.querySelector("kbd")?.textContent).toBe("Y");

      const denyBtn = screen.getByTestId("deny");
      expect(denyBtn).toBeInTheDocument();
      expect(denyBtn.querySelector("kbd")?.textContent).toBe("D");

      // Shortcut note visible
      expect(container.querySelector("[class*='shortcutText']")).not.toBeNull();
    });

    it("supports keyboard shortcuts (y/Y to approve, d/D/n/N to deny) and mouse clicks", () => {
      const store = createTimelineStore([
        {
          id: "perm-kb",
          kind: "permission",
          text: "rm -rf /tmp/cache",
          status: null,
          permission: {
            requestedAction: "rm -rf /tmp/cache",
            scope: "fs:/tmp/cache",
            decision: null,
          },
          streaming: false,
        },
      ]);

      const onDecision = vi.fn();
      const onFocus = vi.fn();

      const { container } = render(
        <TimelineRowView
          store={store}
          rowId="perm-kb"
          index={0}
          focused={true}
          onFocus={onFocus}
          onPermissionDecision={onDecision}
        />,
      );

      const row = container.querySelector<HTMLElement>("[data-row-kind='permission']")!;

      // 1. Press Y -> approve
      fireEvent.keyDown(row, { key: "Y" });
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "approved");

      // 2. Press y -> approve
      onDecision.mockClear();
      fireEvent.keyDown(row, { key: "y" });
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "approved");

      // 3. Press d -> deny
      onDecision.mockClear();
      fireEvent.keyDown(row, { key: "d" });
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "denied");

      // 4. Press D -> deny
      onDecision.mockClear();
      fireEvent.keyDown(row, { key: "D" });
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "denied");

      // 5. Press n -> deny
      onDecision.mockClear();
      fireEvent.keyDown(row, { key: "n" });
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "denied");

      // 6. Click approve button
      onDecision.mockClear();
      fireEvent.click(screen.getByTestId("approve"));
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "approved");

      // 7. Click deny button
      onDecision.mockClear();
      fireEvent.click(screen.getByTestId("deny"));
      expect(onDecision).toHaveBeenCalledWith("perm-kb", "denied");
    });

    it("displays submitting state and submission error properly", async () => {
      const store = createTimelineStore([
        {
          id: "perm-sub",
          kind: "permission",
          text: "git push origin main",
          status: null,
          permission: {
            requestedAction: "git push origin main",
            scope: "git:remote",
            decision: null,
            submission: "submitting",
          },
          streaming: false,
        },
      ]);

      const onDecision = vi.fn();
      render(
        <TimelineRowView
          store={store}
          rowId="perm-sub"
          index={0}
          focused={false}
          onFocus={vi.fn()}
          onPermissionDecision={onDecision}
        />,
      );

      // Buttons are disabled while submitting
      const approveBtn = screen.getByTestId("approve");
      expect(approveBtn).toBeDisabled();

      const denyBtn = screen.getByTestId("deny");
      expect(denyBtn).toBeDisabled();

      // Update to failed submission
      store.setPermissionSubmission("perm-sub", "failed");
      await waitFor(() => {
        const alert = screen.getByRole("alert");
        expect(alert).toBeInTheDocument();
      });
    });

    it("displays high-trust decided audit banner when decision is approved or denied", () => {
      const store = createTimelineStore([
        {
          id: "perm-done",
          kind: "permission",
          text: "git checkout -b feature",
          status: null,
          permission: {
            requestedAction: "git checkout -b feature",
            scope: "git:branch",
            decision: "approved",
          },
          streaming: false,
        },
      ]);

      const { container } = render(
        <TimelineRowView
          store={store}
          rowId="perm-done"
          index={0}
          focused={false}
          onFocus={vi.fn()}
          onPermissionDecision={vi.fn()}
        />,
      );

      expect(screen.queryByTestId("approve")).toBeNull();
      expect(screen.queryByTestId("deny")).toBeNull();

      const decisionChip = container.querySelector("[class*='decisionChip']");
      expect(decisionChip?.textContent).toContain("approved");
      expect(container.querySelector("[class*='mono']")?.textContent).toBe("git checkout -b feature");
      expect(container.querySelector("[class*='scope']")?.textContent).toBe("git:branch");
    });
  });

  describe("A11y, roving tabindex & narrow viewport resilience", () => {
    it("maintains correct tabindex and article role on focused vs unfocused rows", () => {
      const store = createTimelineStore([
        {
          id: "r1",
          kind: "user-message",
          text: "Row 1",
          status: null,
          permission: null,
          streaming: false,
        },
      ]);

      const onFocus = vi.fn();
      const { container, rerender } = render(
        <TimelineRowView
          store={store}
          rowId="r1"
          index={0}
          focused={false}
          onFocus={onFocus}
          onPermissionDecision={vi.fn()}
        />,
      );

      const row = container.querySelector("[data-row-id='r1']")!;
      expect(row).toHaveAttribute("tabindex", "-1");
      expect(row).toHaveAttribute("role", "article");

      // Click to focus
      fireEvent.mouseDown(row);
      expect(onFocus).toHaveBeenCalledWith("r1");

      // Rerender as focused
      rerender(
        <TimelineRowView
          store={store}
          rowId="r1"
          index={0}
          focused={true}
          onFocus={onFocus}
          onPermissionDecision={vi.fn()}
        />,
      );

      expect(row).toHaveAttribute("tabindex", "0");
      expect(row.className).toContain("focused");
    });
  });
});
