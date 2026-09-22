/**
 * Evidence: cancel-failed composer chip variant — when cancel_turn fails while
 * the turn is still live, chip must leave "streaming" chrome for danger
 * "停止未确认" / "Stop unconfirmed" and keep Stop retriable.
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

describe("p37 cancel-failed composer chip clarity", () => {
  it("zh-CN / EN expose short statusCancelFailed chip labels (ties to cancelFailed notice)", () => {
    const zh = getDictionary("zh-CN");
    expect(zh.composer.statusCancelFailed).toBe("停止未确认");
    expect(zh.composer.cancelFailed("channel lost")).toContain("未能确认停止");
    expect(zh.composer.cancelFailed("channel lost")).toContain("channel lost");
    expect(zh.composer.stopFailed).toContain("未能确认停止");

    const en = getDictionary("en");
    expect(en.composer.statusCancelFailed).toBe("Stop unconfirmed");
    expect(en.composer.cancelFailed("channel lost")).toMatch(/Could not confirm stop/i);
  });

  it("streaming stays streaming until cancelFailed is set", () => {
    renderComposer({ isStreaming: true });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "streaming");
    expect(chip).toHaveTextContent("生成中");
  });

  it("cancelFailed + streaming shows danger cancel_failed chip, not streaming", () => {
    renderComposer({ isStreaming: true, cancelFailed: true });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "cancel_failed");
    expect(chip).toHaveTextContent("停止未确认");
    expect(chip.className).toMatch(/composerChipCancelFailed/);
    expect(chip.className).not.toMatch(/composerChipStreaming/);

    const stop = screen.getByTestId("cancel-turn");
    expect(stop).not.toBeDisabled();
    expect(stop).toHaveAttribute("data-cancel-failed", "true");
    expect(stop).toHaveAttribute("title", getDictionary("zh-CN").composer.stopFailed);
  });

  it("EN cancel_failed chip uses Stop unconfirmed", () => {
    renderComposer({ isStreaming: true, cancelFailed: true }, "en");
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "cancel_failed");
    expect(chip).toHaveTextContent("Stop unconfirmed");
  });

  it("cancelPending (stopping) wins over cancelFailed", () => {
    renderComposer({
      isStreaming: true,
      cancelPending: true,
      cancelFailed: true,
    });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "stopping");
    expect(chip).toHaveTextContent("正在停止");
  });

  it("unavailable still wins over cancelFailed", () => {
    renderComposer({
      isStreaming: true,
      cancelFailed: true,
      disabledReason: "本地服务暂不可用。你的输入已保留。",
    });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "unavailable");
  });

  it("cancelFailed alone without streaming does not force cancel_failed chrome", () => {
    renderComposer({ cancelFailed: true, isStreaming: false });
    const chip = screen.getByTestId("composer-ready-chip");
    expect(chip).toHaveAttribute("data-chip-state", "ready");
    expect(chip).toHaveTextContent("代理已就绪");
  });

  it("CSS declares danger cancel-failed chip chrome", () => {
    const css = readFileSync(cssPath, "utf8");
    expect(css).toMatch(
      /\.composerChipCancelFailed\s*\{[^}]*background:\s*var\(--color-danger-surface\)/s,
    );
    expect(css).toMatch(
      /\.composerChipCancelFailed\s*\{[^}]*color:\s*var\(--color-danger\)/s,
    );
    expect(css).toMatch(
      /\.composerChipCancelFailed\s*\{[^}]*border-color:\s*var\(--color-danger\)/s,
    );
  });
});
