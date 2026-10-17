/**
 * Evidence: P1 shell aria-labels, onboarding placeholders, splitter aria,
 * and MemoryPane source labels are locale-aware (zh-CN / en).
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ActivityRail,
  AgentOnboardingModal,
  Composer,
  Inspector,
  NavResizeHandle,
  ThreadsPane,
} from "../components/shell";
import { MemoryPane } from "../components/MemoryPane";
import { I18nProvider, getDictionary } from "../i18n";
import type { MemoryRecordDto } from "../ipc/dto/MemoryRecordDto";

afterEach(() => {
  cleanup();
});

const sampleMemory: MemoryRecordDto = {
  memory_id: "mem_p27",
  content: "prefer rust",
  state: "confirmed",
  kind: "preference",
  scope_kind: "global",
  scope_target: null,
  source: "explicit",
  confidence: 90,
  sensitivity: "normal",
  explicit: true,
  created_at: 1_726_560_000_000,
  updated_at: 1_726_560_000_000,
  provenance_thread_id: null,
  provenance_turn_id: null,
  excerpt: null,
  expires_at: null,
  superseded_by: null,
};

describe("p27 shell aria + onboarding + memory source i18n", () => {
  it("Activity / Threads / Composer / Inspector aria-labels are Chinese under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ActivityRail active="threads" onNavigate={vi.fn()} />
        <ThreadsPane
          threads={[]}
          selectedThreadId=""
          onSelect={vi.fn()}
          filter=""
          onFilterChange={vi.fn()}
        />
        <Composer
          draft=""
          onDraftChange={vi.fn()}
          onSend={vi.fn()}
          disabledReason={null}
        />
        <Inspector
          width={320}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
        />
      </I18nProvider>,
    );

    expect(screen.getByRole("navigation", { name: "活动栏" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "会话列表" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "输入框" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "检查器" })).toBeInTheDocument();
  });

  it("splitter aria labels are Chinese under zh-CN", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <Inspector
          width={320}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
        />
        <NavResizeHandle width={240} onWidthChange={vi.fn()} />
      </I18nProvider>,
    );

    expect(screen.getByTestId("inspector-resize")).toHaveAttribute(
      "aria-label",
      "检查器宽度",
    );
    expect(screen.getByTestId("nav-resize")).toHaveAttribute(
      "aria-label",
      "会话栏宽度",
    );
  });

  it("inspector tab arrow keys move real DOM focus (roving tabindex)", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <Inspector
          width={320}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
        />
      </I18nProvider>,
    );

    const detailsTab = screen.getByTestId("inspector-tab-details");
    detailsTab.focus();
    expect(document.activeElement).toBe(detailsTab);

    // ArrowRight walks to the context tab AND moves focus with it; with the
    // roving tabindex, focus left on a now-tabIndex=-1 button would let the
    // next Tab escape the tablist entirely.
    fireEvent.keyDown(detailsTab, { key: "ArrowRight" });
    const contextTab = screen.getByTestId("inspector-tab-context");
    expect(document.activeElement).toBe(contextTab);
    expect(contextTab).toHaveAttribute("aria-selected", "true");
    expect(detailsTab).toHaveAttribute("aria-selected", "false");

    fireEvent.keyDown(contextTab, { key: "ArrowRight" });
    expect(document.activeElement).toBe(screen.getByTestId("inspector-tab-diagnostics"));

    // ArrowLeft wraps back around from details to diagnostics.
    fireEvent.keyDown(detailsTab, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(screen.getByTestId("inspector-tab-diagnostics"));
    expect(screen.getByTestId("inspector-tab-diagnostics")).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("onboarding Variable Key / Secret Ref / Primary ACP Binding placeholders are Chinese", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <AgentOnboardingModal
          isOpen
          onClose={vi.fn()}
          onSave={vi.fn()}
          onTest={vi.fn()}
          isTesting={false}
          testResult={null}
        />
      </I18nProvider>,
    );

    fireEvent.click(screen.getByTestId("add-env-mapping-btn"));
    expect(screen.getByTestId("env-key-input-0")).toHaveAttribute(
      "placeholder",
      "环境变量名",
    );
    expect(screen.getByTestId("secret-ref-input-0")).toHaveAttribute(
      "placeholder",
      "凭据引用 (sec_...、vault://...)",
    );
    expect(screen.getByTestId("agent-label-input")).toHaveAttribute(
      "placeholder",
      "主 ACP 绑定",
    );
    expect(screen.getByTestId("onboarding-close")).toHaveAttribute(
      "aria-label",
      "关闭引导对话框",
    );
  });

  it("MemoryPane renders localized explicit/inferred source labels", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <MemoryPane memories={[sampleMemory]} />
      </I18nProvider>,
    );

    const source = screen.getByTestId("memory-source-label");
    expect(screen.getByRole("columnheader", { name: "来源" })).toBeTruthy();
    expect(source).toHaveTextContent("显式");
    expect(source).not.toHaveTextContent("explicit");
  });

  it("en dictionary keeps English shell aria and onboarding placeholders", () => {
    const en = getDictionary("en");
    expect(en.rail.activityAria).toBe("Activity");
    expect(en.nav.paneAria).toBe("Threads");
    expect(en.composer.ariaLabel).toBe("Composer");
    expect(en.inspector.ariaLabel).toBe("Inspector");
    expect(en.inspector.widthAria).toBe("Inspector width");
    expect(en.nav.widthAria).toBe("Threads pane width");
    expect(en.onboarding.variableKeyPlaceholder).toBe("Variable Key");
    expect(en.onboarding.secretRefPlaceholder).toContain("Secret Ref");
    expect(en.onboarding.primaryBindingPlaceholder).toMatch(/Primary ACP Binding/);
    expect(en.memoryPane.sourceExplicit).toBe("Explicit");
    expect(en.memoryPane.sourceInferred).toBe("Inferred");

    const zh = getDictionary("zh-CN");
    expect(zh.rail.activityAria).toContain("活动");
    expect(zh.memoryPane.sourceExplicit).toBe("显式");
    expect(zh.memoryPane.sourceInferred).toBe("推断");
  });
});
