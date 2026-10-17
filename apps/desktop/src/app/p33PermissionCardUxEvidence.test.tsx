/**
 * Evidence: permission approval card UX polish — token focus rings on
 * Approve/Deny, denser action spacing, and no undefined kind class on the row.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TimelineRowView } from "../features/timeline/TimelineRowView";
import { createTimelineStore } from "../features/timeline/timelineStore";
import { I18nProvider } from "../i18n";

afterEach(() => {
  cleanup();
});

const cssPath = resolve(
  process.cwd(),
  "src/features/timeline/timeline.module.css",
);

describe("p33 permission card UX polish", () => {
  it("Approve/Deny declare token focus-visible rings (not browser default only)", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(
      /\.approveButton:focus-visible,\s*\n\s*\.denyButton:focus-visible\s*\{/,
    );
    expect(css).toContain("outline: var(--focus-ring) solid var(--color-focus)");
    expect(css).toContain("outline-offset: 2px");
  });

  it("permission card uses breathing-room spacing tokens for content and actions", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(
      /\.permissionCardContent\s*\{[^}]*gap:\s*var\(--spacing-16\)/s,
    );
    expect(css).toMatch(
      /\.permissionAsk\s*\{[^}]*padding-top:\s*var\(--spacing-12\)/s,
    );
    expect(css).toMatch(
      /\.buttonGroup\s*\{[^}]*gap:\s*var\(--spacing-12\)/s,
    );
    expect(css).toMatch(
      /\.riskNotice\s*\{[^}]*border-bottom:\s*var\(--divider-width\) solid var\(--color-warning\)/s,
    );
  });

  it("permission row keeps a single warning strip and standard focus ring", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(
      /\.row\[data-row-kind="permission"\] \.card\s*\{[^}]*border-left:\s*calc\(var\(--divider-width\) \* 3\) solid var\(--color-warning\)/s,
    );
    expect(css).toContain('.row[data-row-kind="permission"].focused .card');
    expect(css).not.toContain("0 0 0 var(--focus-ring) var(--color-focus)");
  });

  it("permission row className has no undefined kind fragment", () => {
    const store = createTimelineStore([
      {
        id: "perm-ux",
        kind: "permission",
        text: "cargo tree --workspace",
        status: null,
        permission: {
          requestedAction: "cargo tree --workspace",
          scope: "project:altior",
          decision: null,
        },
        streaming: false,
      },
    ]);

    const { container } = render(
      <I18nProvider localeSource="zh-CN">
        <TimelineRowView
          store={store}
          rowId="perm-ux"
          index={0}
          focused={true}
          onFocus={vi.fn()}
          onPermissionDecision={vi.fn()}
        />
      </I18nProvider>,
    );

    const row = container.querySelector<HTMLElement>(
      "[data-row-kind='permission']",
    );
    expect(row).not.toBeNull();
    expect(row!.className).not.toMatch(/\bundefined\b/);
    expect(row!.className).toMatch(/focused/);
    expect(screen.getByTestId("approve")).toBeInTheDocument();
    expect(screen.getByTestId("deny")).toBeInTheDocument();
  });
});
