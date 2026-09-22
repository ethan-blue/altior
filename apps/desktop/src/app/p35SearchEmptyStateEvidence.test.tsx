/**
 * Evidence: search empty state — compact no-match card, clear-filter CTA,
 * zh-CN copy, and no orphan RECENT section when the list is empty.
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ThreadsPane } from "../components/shell";
import { getDictionary, I18nProvider } from "../i18n";

afterEach(() => {
  cleanup();
});

const cssPath = resolve(process.cwd(), "src/components/shell.module.css");

describe("p35 search empty state affordance", () => {
  it("zh-CN and EN expose no-match copy plus clearFilter label", () => {
    const zh = getDictionary("zh-CN");
    expect(zh.nav.noMatches).toContain("没有匹配");
    expect(zh.nav.clearFilter).toBe("清除筛选");

    const en = getDictionary("en");
    expect(en.nav.noMatches).toMatch(/No conversations match/i);
    expect(en.nav.clearFilter).toBe("Clear filter");
  });

  it("search empty renders compact card with clear CTA and skips RECENT", () => {
    const onFilterChange = vi.fn();
    render(
      <I18nProvider localeSource="zh-CN">
        <ThreadsPane
          threads={[]}
          selectedThreadId=""
          onSelect={vi.fn()}
          filter="zzzz-no-match"
          onFilterChange={onFilterChange}
          searchActive={true}
          debounceMs={0}
        />
      </I18nProvider>,
    );

    const empty = screen.getByTestId("threads-search-empty");
    expect(empty).toHaveAttribute("data-empty-kind", "search");
    expect(empty).toHaveAttribute("role", "status");
    expect(empty).toHaveTextContent("没有匹配的会话");
    expect(empty.className).toMatch(/threadsEmptySearch/);

    expect(screen.queryByText("近期会话")).not.toBeInTheDocument();
    expect(screen.queryByText("RECENT")).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId("clear-thread-filter"));
    expect(onFilterChange).toHaveBeenCalledWith("");
  });

  it("vault empty uses vault testid without clear CTA and hides RECENT", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <ThreadsPane
          threads={[]}
          selectedThreadId=""
          onSelect={vi.fn()}
          filter=""
          onFilterChange={vi.fn()}
          searchActive={false}
        />
      </I18nProvider>,
    );

    const empty = screen.getByTestId("threads-empty");
    expect(empty).toHaveAttribute("data-empty-kind", "vault");
    expect(empty).toHaveTextContent("暂无会话");
    expect(screen.queryByTestId("clear-thread-filter")).not.toBeInTheDocument();
    expect(screen.queryByText("近期会话")).not.toBeInTheDocument();
  });

  it("CSS keeps search empty density + focus-visible affordance tokens", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(/\.threadsEmpty\s*\{[\s\S]*?gap:\s*var\(--spacing-8\)/);
    expect(css).toMatch(/\.threadsEmptySearch\s*\{/);
    expect(css).toMatch(/\.threadsEmptyAction:focus-visible\s*\{[\s\S]*?outline:\s*var\(--focus-ring\)/);
    expect(css).toMatch(/background:\s*var\(--color-surface-subtle\)/);
    expect(css).toMatch(/background:\s*var\(--color-badge-bg\)/);
  });
});
