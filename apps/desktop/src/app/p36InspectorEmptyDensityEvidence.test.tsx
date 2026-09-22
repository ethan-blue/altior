/**
 * Evidence: inspector empty density — compact select-row card, clear title+hint
 * CTA (zh-CN), and titled Active agent section (no orphan bare fields).
 */
import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Inspector } from "../components/shell";
import { getDictionary, I18nProvider } from "../i18n";
import type { AgentProfile } from "../stores/applicationStore";

afterEach(() => {
  cleanup();
});

const cssPath = resolve(process.cwd(), "src/components/shell.module.css");

const sampleAgent: AgentProfile = {
  id: "agent-alpha",
  name: "alpha (ACP)",
  provider: "acp",
  model: "acp",
  status: "ready",
};

describe("p36 inspector empty density affordance", () => {
  it("zh-CN and EN expose compact title, hint, and activeAgentSection", () => {
    const zh = getDictionary("zh-CN");
    expect(zh.inspector.selectRowToInspect).toBe("暂无选中行");
    expect(zh.inspector.selectRowHint).toContain("时间线");
    expect(zh.inspector.activeAgentSection).toBe("当前代理");

    const en = getDictionary("en");
    expect(en.inspector.selectRowToInspect).toMatch(/No row selected/i);
    expect(en.inspector.selectRowHint).toMatch(/timeline row/i);
    expect(en.inspector.activeAgentSection).toBe("Active agent");
  });

  it("empty details render compact card + titled agent section without orphan headers", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <Inspector
          width={320}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
          activeAgent={sampleAgent}
        />
      </I18nProvider>,
    );

    const empty = screen.getByTestId("inspector-select-row-empty");
    expect(empty).toHaveAttribute("data-empty-kind", "select-row");
    expect(empty).toHaveAttribute("role", "status");
    expect(empty).toHaveTextContent("暂无选中行");
    expect(empty).toHaveTextContent("在时间线中点击一行以查看详情。");
    expect(empty.className).toMatch(/inspectorEmptyCard/);

    const agentSection = screen.getByTestId("inspector-active-agent");
    expect(agentSection.tagName.toLowerCase()).toBe("section");
    expect(agentSection).toHaveAccessibleName("当前代理");
    expect(screen.getByRole("heading", { level: 3, name: "当前代理" })).toBeInTheDocument();
    expect(agentSection).toHaveTextContent("alpha (ACP)");

    // No bare orphan English section chrome from prior locale.
    expect(screen.queryByText("RECENT")).not.toBeInTheDocument();
  });

  it("empty without agent shows only the select-row card", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <Inspector
          width={320}
          onWidthChange={vi.fn()}
          onClose={vi.fn()}
          focusedRow={null}
          activeAgent={null}
        />
      </I18nProvider>,
    );

    expect(screen.getByTestId("inspector-select-row-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("inspector-active-agent")).not.toBeInTheDocument();
    expect(screen.queryByText("当前代理")).not.toBeInTheDocument();
  });

  it("CSS keeps compact empty density + titled stack tokens", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(/\.inspectorEmpty\s*\{[\s\S]*?margin:\s*var\(--spacing-8\)/);
    expect(css).toMatch(/\.inspectorEmptyCard\s*\{[\s\S]*?gap:\s*var\(--spacing-4\)/);
    expect(css).toMatch(/\.inspectorEmptyTitle\s*\{/);
    expect(css).toMatch(/\.inspectorEmptyHint\s*\{/);
    expect(css).toMatch(/background:\s*var\(--color-badge-bg/);
    expect(css).toMatch(/background:\s*var\(--color-surface-subtle/);
  });
});
