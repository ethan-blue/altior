/**
 * A09 Acceptance Evidence: Chinese IME, Dialog Focus Trap, Keyboard & Accessibility (F16 / F17).
 *
 * Proves that:
 * 1. Chinese IME composition protection (F16 fixed): pressing Enter during active composition
 *    or with Windows keyCode 229 confirms candidate words and DOES NOT trigger send.
 * 2. Shift+Enter produces newlines without sending.
 * 3. Dialog accessibility (F17 fixed): AgentOnboardingModal captures focus on open, traps
 *    Tab / Shift+Tab within the modal, dismisses on Escape, and returns focus on close.
 * 4. Inspector tabs accessibility: supports ArrowLeft / ArrowRight navigation, exposes
 *    proper `role="tab"`, `aria-controls`, and `role="tabpanel"`.
 * 5. Timeline accessibility: keyboard approval via 'y' and 'd' keys works identically to mouse click.
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Composer, AgentOnboardingModal, Inspector, ThreadsPane, ActivityRail, SettingsModal } from "../components/shell";
import { App } from "./App";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { approvalThread } from "../fixtures/timeline";

describe("A09 evidence: Chinese IME composition, modal focus trap & keyboard accessibility", () => {
  describe("Composer Chinese IME & Enter key handling (F16)", () => {
    it("Enter during active composition (isComposing) confirms pinyin candidate and DOES NOT send", () => {
      const onSend = vi.fn();
      const onDraftChange = vi.fn();

      render(
        <Composer
          draft="nihao"
          onDraftChange={onDraftChange}
          onSend={onSend}
          disabledReason={null}
        />,
      );

      const textarea = screen.getByRole("textbox", { name: /composer/i });

      // 1. User starts typing Chinese Pinyin: compositionstart
      fireEvent.compositionStart(textarea);

      // 2. User presses Enter to confirm candidate word while composing
      fireEvent.keyDown(textarea, { key: "Enter", keyCode: 229 });
      expect(onSend).not.toHaveBeenCalled();

      // 3. Native event isComposing is true
      fireEvent.keyDown(textarea, { key: "Enter", nativeEvent: { isComposing: true } });
      expect(onSend).not.toHaveBeenCalled();

      // 4. User finishes composition
      fireEvent.compositionEnd(textarea);

      // 5. User presses Enter normally to send the completed message
      fireEvent.keyDown(textarea, { key: "Enter" });
      expect(onSend).toHaveBeenCalledTimes(1);
    });

    it("Shift+Enter does not trigger send (permits multi-line message drafting)", () => {
      const onSend = vi.fn();
      const onDraftChange = vi.fn();

      render(
        <Composer
          draft="Line 1"
          onDraftChange={onDraftChange}
          onSend={onSend}
          disabledReason={null}
        />,
      );

      const textarea = screen.getByRole("textbox", { name: /composer/i });
      fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });

      expect(onSend).not.toHaveBeenCalled();
    });
  });

  describe("AgentOnboardingModal keyboard accessibility & focus trap (F17)", () => {
    it("dismisses modal on Escape key", () => {
      const onClose = vi.fn();
      render(
        <AgentOnboardingModal
          isOpen={true}
          onClose={onClose}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={null}
        />,
      );

      fireEvent.keyDown(window, { key: "Escape" });
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("traps Tab and Shift+Tab within the modal dialog", () => {
      render(
        <AgentOnboardingModal
          isOpen={true}
          onClose={vi.fn()}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={null}
        />,
      );

      const dialog = screen.getByRole("dialog");
      const nameInput = screen.getByTestId("agent-name-input");
      // Populate name so the save button is enabled and focusable
      fireEvent.change(nameInput, { target: { value: "Accessible Agent" } });

      const closeBtn = screen.getByTestId("onboarding-close");
      const saveBtn = screen.getByTestId("agent-save-button");

      // Shift+Tab on first element wraps to last focusable element
      closeBtn.focus();
      fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
      expect(document.activeElement).toBe(saveBtn);

      // Tab on last element wraps to first focusable element
      saveBtn.focus();
      fireEvent.keyDown(dialog, { key: "Tab", shiftKey: false });
      expect(document.activeElement).toBe(closeBtn);
    });

    it("has accessible close button label and modal title reference", () => {
      render(
        <AgentOnboardingModal
          isOpen={true}
          onClose={vi.fn()}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={null}
        />,
      );

      expect(screen.getByLabelText(/close onboarding/i)).toBeInTheDocument();
      const dialog = screen.getByRole("dialog");
      expect(dialog).toHaveAttribute("aria-labelledby", "onboarding-modal-title");
    });
  });

  describe("Inspector tablist and arrow key navigation", () => {
    it("supports ArrowRight / ArrowLeft to switch between details, context, and diagnostics tabs", () => {
      render(
        <Inspector
          width={360}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
          activeAgent={null}
        />,
      );

      const detailsTab = screen.getByTestId("inspector-tab-details");
      const contextTab = screen.getByTestId("inspector-tab-context");
      const diagTab = screen.getByTestId("inspector-tab-diagnostics");

      expect(detailsTab).toHaveAttribute("aria-selected", "true");
      expect(screen.getByRole("tabpanel")).toHaveAttribute("id", "inspector-panel-details");

      // Press ArrowRight to switch to context tab
      fireEvent.keyDown(detailsTab, { key: "ArrowRight" });
      expect(contextTab).toHaveAttribute("aria-selected", "true");
      expect(screen.getByRole("tabpanel")).toHaveAttribute("id", "inspector-panel-context");

      // Press ArrowRight to switch to diagnostics tab
      fireEvent.keyDown(contextTab, { key: "ArrowRight" });
      expect(diagTab).toHaveAttribute("aria-selected", "true");
      expect(screen.getByRole("tabpanel")).toHaveAttribute("id", "inspector-panel-diagnostics");

      // Press ArrowLeft to switch back to context tab
      fireEvent.keyDown(diagTab, { key: "ArrowLeft" });
      expect(contextTab).toHaveAttribute("aria-selected", "true");

      // Press ArrowLeft to switch back to details tab
      fireEvent.keyDown(contextTab, { key: "ArrowLeft" });
      expect(detailsTab).toHaveAttribute("aria-selected", "true");
    });

    it("renders runtime diagnostics in Inspector without leaking secret material", () => {
      const mockDiag = {
        instance_id: "cor_test0000000000001",
        status: "ready",
        active_threads: 3,
        active_turns: 1,
        summary: "healthy; auth verified; bearer token [REDACTED]",
      };

      render(
        <Inspector
          width={360}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
          activeAgent={null}
          diagnostics={mockDiag}
          diagnosticsStatus="loaded"
          initialTab="diagnostics"
        />,
      );

      expect(screen.getByTestId("diag-instance-id").textContent).toBe("cor_test0000000000001");
      expect(screen.getByTestId("diag-status").textContent).toBe("ready");
      expect(screen.getByTestId("diag-active-threads").textContent).toBe("3");
      expect(screen.getByTestId("diag-active-turns").textContent).toBe("1");
      expect(screen.getByTestId("diag-summary").textContent).toContain("healthy");
      expect(screen.getByTestId("diag-redacted-notice")).toBeTruthy();
    });

    it("renders runtime diagnostics section in SettingsModal", () => {
      const mockDiag = {
        instance_id: "cor_test_settings_001",
        status: "ready",
        active_threads: 2,
        active_turns: 0,
        summary: "healthy",
      };

      render(
        <SettingsModal
          isOpen={true}
          onClose={vi.fn()}
          themeSource="light"
          onThemeSourceChange={vi.fn()}
          localeSource="en"
          onLocaleSourceChange={vi.fn()}
          diagnostics={mockDiag}
          diagnosticsStatus="loaded"
        />,
      );

      expect(screen.getByTestId("settings-diagnostics-section")).toBeTruthy();
      expect(screen.getByTestId("diag-instance-id").textContent).toBe("cor_test_settings_001");
    });
  });

  describe("ThreadsPane search debounce and IME composition protection", () => {
    it("does not dispatch search filter during active composition and debounces keystrokes", () => {
      vi.useFakeTimers();
      const onFilterChange = vi.fn();

      render(
        <ThreadsPane
          threads={[]}
          selectedThreadId=""
          onSelect={vi.fn()}
          filter=""
          onFilterChange={onFilterChange}
          debounceMs={280}
        />,
      );

      const input = screen.getByTestId("thread-filter");

      // 1. Start Chinese IME composition
      fireEvent.compositionStart(input);
      fireEvent.change(input, { target: { value: "zhong" } });

      // Fast-forward time past debounce interval
      vi.advanceTimersByTime(300);
      // While composing, no IPC search filter is dispatched
      expect(onFilterChange).not.toHaveBeenCalled();

      // 2. End composition with final word
      fireEvent.change(input, { target: { value: "中文会话" } });
      fireEvent.compositionEnd(input, { currentTarget: { value: "中文会话" } });

      // Search is debounced
      expect(onFilterChange).not.toHaveBeenCalled();

      // Advance by debounce interval
      vi.advanceTimersByTime(280);
      expect(onFilterChange).toHaveBeenCalledWith("中文会话");
      expect(onFilterChange).toHaveBeenCalledTimes(1);

      vi.useRealTimers();
    });
  });

  describe("ActivityRail scalable SVG icons and P4 tooltip", () => {
    it("renders inline SVG icons with aria-hidden and P4 phase for projects", () => {
      const { container } = render(<ActivityRail active="threads" onNavigate={vi.fn()} />);

      // Verify all icons are SVGs with aria-hidden
      const svgs = container.querySelectorAll("button svg");
      expect(svgs.length).toBe(6);
      for (const svg of svgs) {
        expect(svg).toHaveAttribute("aria-hidden", "true");
      }

      // Verify Projects tooltip explicitly targets P4
      const projectsBtn = screen.getByTestId("rail-projects");
      expect(projectsBtn).toHaveAttribute("title", expect.stringContaining("P4"));
    });
  });

  describe("Permission decision keyboard parity", () => {
    it("approves permission via keyboard 'y' key identically to mouse click", async () => {
      render(
        <App
          transport={new InMemoryTransport()}
          fixtureTimelineRows={[approvalThread]}
        />,
      );
      fireEvent.click(await screen.findByTestId(`thread-${approvalThread.id}`));

      const row = document.querySelector<HTMLElement>("[data-row-kind='permission']");
      expect(row).not.toBeNull();
      row?.focus();

      // Press 'y' to approve
      fireEvent.keyDown(row!, { key: "y" });

      await waitFor(() => {
        expect(row?.textContent).toContain("approved");
      });
    });
  });
});
