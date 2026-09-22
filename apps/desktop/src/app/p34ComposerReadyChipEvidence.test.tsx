/**
 * Evidence: composer ready-state chip clarity — distinct labels and token
 * chrome for ready / streaming / stopping / unavailable, with spacing hierarchy.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { Composer } from "../components/shell";
import { getDictionary, I18nProvider } from "../i18n";

afterEach(() => {
  cleanup();
});

const cssPath = resolve(process.cwd(), "src/components/shell.module.css");

function renderComposer(
  props: Partial<ComponentProps<typeof Composer>> = {},
  locale: "zh-CN" | "en" = "zh-CN",
) {
  return render(
    <I18nProvider localeSource={locale}>
      <Composer
        draft=""
        onDraftChange={vi.fn()}
        onSend={vi.fn()}
        onCancel={vi.fn()}
        disabledReason={null}
        {...props}
      />
    </I18nProvider>,
  );
}

describe("p34 composer ready-state chip clarity", () => {
  it("zh-CN chip labels cover ready / streaming / stopping / unavailable", () => {
    const zh = getDictionary("zh-CN");
    expect(zh.composer.smartTools).toBe("代理已就绪");
    expect(zh.composer.statusStreaming).toBe("生成中");
    expect(zh.composer.statusStopping).toBe("正在停止");
    expect(zh.composer.statusUnavailable).toBe("暂不可用");

    const en = getDictionary("en");
    expect(en.composer.smartTools).toBe("Agent ready");
    expect(en.composer.statusStreaming).toBe("Generating");
    expect(en.composer.statusStopping).toBe("Stopping");
    expect(en.composer.statusUnavailable).toBe("Unavailable");
  });

  it("renders ready chip by default with success state chrome", () => {
    renderComposer();
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "ready");
    expect(chip).toHaveAttribute("role", "status");
    expect(chip).toHaveTextContent("代理已就绪");
    expect(chip.className).toMatch(/composerChipReady/);
  });

  it("switches to streaming then stopping labels while a turn is live", () => {
    const { rerender } = renderComposer({ isStreaming: true });
    let chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "streaming");
    expect(chip).toHaveTextContent("生成中");
    expect(chip.className).toMatch(/composerChipStreaming/);

    rerender(
      <I18nProvider localeSource="zh-CN">
        <Composer
          draft=""
          onDraftChange={vi.fn()}
          onSend={vi.fn()}
          onCancel={vi.fn()}
          isStreaming={true}
          cancelPending={true}
          disabledReason={null}
        />
      </I18nProvider>,
    );
    chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "stopping");
    expect(chip).toHaveTextContent("正在停止");
    expect(chip.className).toMatch(/composerChipStopping/);
  });

  it("unavailable state wins over streaming when composer is disabled", () => {
    renderComposer({
      isStreaming: true,
      disabledReason: "本地服务暂不可用。你的输入已保留。",
    });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "unavailable");
    expect(chip).toHaveTextContent("暂不可用");
    expect(chip.className).toMatch(/composerChipUnavailable/);
  });

  it("CSS declares state contrast chrome, pulse, and left-tools spacing", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(
      /\.composerToolsLeft\s*\{[^}]*gap:\s*var\(--spacing-12\)/s,
    );
    expect(css).toMatch(
      /\.composerChipReady\s*\{[^}]*background:\s*var\(--color-success-surface\)/s,
    );
    expect(css).toMatch(
      /\.composerChipReady\s*\{[^}]*color:\s*var\(--color-success\)/s,
    );
    expect(css).toMatch(
      /\.composerChipStreaming\s*\{[^}]*border-color:\s*var\(--color-accent\)/s,
    );
    expect(css).toMatch(
      /\.composerChipStopping\s*\{[^}]*background:\s*var\(--color-warning-surface\)/s,
    );
    expect(css).toMatch(
      /\.composerChipUnavailable\s*\{[^}]*background:\s*var\(--color-disabled-surface\)/s,
    );
    expect(css).toContain("@keyframes composerChipPulse");
    expect(css).toContain("prefers-reduced-motion: reduce");
  });
});
