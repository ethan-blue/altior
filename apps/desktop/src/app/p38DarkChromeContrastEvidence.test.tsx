/**
 * Evidence: dark residual chrome contrast outside the 24-token gate —
 * muted rail icons, status bar, and kbd keycaps (composer + header).
 *
 * Proves CSS uses AA-safe tokens without opacity stacking that drops
 * inactive/deferred rail icons or keycap chrome below readable levels,
 * while keeping check-contrast.mjs 24/24 green.
 */
import { cleanup, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { StatusBar, Composer } from "../components/shell";
import { I18nProvider } from "../i18n";
import {
  contrastRatio,
  darkTokens,
  runAudit,
} from "../styles/contrastAudit";

afterEach(() => {
  cleanup();
});

const shellCss = readFileSync(
  resolve(process.cwd(), "src/components/shell.module.css"),
  "utf8",
);
const appCss = readFileSync(
  resolve(process.cwd(), "src/app/App.module.css"),
  "utf8",
);

/** Dark chrome surfaces used by rail / status / kbd (not in the 24-pair gate). */
const darkChrome = {
  railBg: "#111317",
  elevated: "#22262d",
  headerBg: "#16181d",
} as const;

describe("p38 dark residual chrome contrast (rail / status / kbd)", () => {
  it("token gate stays 24/24 after chrome CSS-only fixes", () => {
    const audit = runAudit();
    expect(audit.failed).toBe(0);
    expect(audit.passed).toBe(24);
    expect(audit.total).toBe(24);
  });

  it("muted text clears AA on rail, status surface, and elevated kbd host", () => {
    expect(contrastRatio(darkTokens.muted, darkChrome.railBg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(darkTokens.muted, darkTokens.surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(darkTokens.muted, darkChrome.elevated)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(darkTokens.muted, darkChrome.headerBg)).toBeGreaterThanOrEqual(4.5);
  });

  it("kbd keycap border uses control-border (≥3:1) not decorative border", () => {
    expect(contrastRatio(darkTokens.controlBorder, darkTokens.surface)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(darkTokens.border, darkTokens.surface)).toBeLessThan(3);

    expect(shellCss).toMatch(
      /\.composerKbd\s*\{[^}]*border:\s*1px\s+solid\s+var\(--color-control-border\)/s,
    );
    expect(appCss).toMatch(
      /\.shortcutBadge\s*\{[^}]*border:\s*1px\s+solid\s+var\(--color-control-border\)/s,
    );
    expect(shellCss).not.toMatch(
      /\.composerKbd\s*\{[^}]*border:\s*1px\s+solid\s+var\(--color-border\)/s,
    );
  });

  it("composer shortcuts drop opacity stacking; divider stays muted-toned", () => {
    expect(shellCss).toMatch(/\.composerShortcuts\s*\{[^}]*color:\s*var\(--color-muted\)/s);
    expect(shellCss).not.toMatch(/\.composerShortcuts\s*\{[^}]*opacity:\s*0\.9/s);
    expect(shellCss).toMatch(/\.composerKbdDivider\s*\{[^}]*color:\s*var\(--color-muted\)/s);
    expect(shellCss).not.toMatch(/\.composerKbdDivider\s*\{\s*opacity:\s*0\.5\s*;\s*\}/s);
  });

  it("disabled rail items use disabled-text without blanket opacity", () => {
    expect(shellCss).toMatch(
      /\.railItem:disabled\s*\{[^}]*color:\s*var\(--color-disabled-text\)/s,
    );
    expect(shellCss).not.toMatch(/\.railItem:disabled\s*\{[^}]*opacity:\s*0\.5/s);
  });

  it("zh-CN status bar + composer kbd chrome still render", () => {
    render(
      <I18nProvider localeSource="zh-CN">
        <StatusBar
          coreState="connected"
          threadStatus="completed"
          streamState="live"
        />
        <Composer
          draft=""
          onDraftChange={() => {}}
          onSend={() => {}}
          onCancel={() => {}}
          disabledReason={null}
        />
      </I18nProvider>,
    );

    const bar = screen.getByTestId("status-bar");
    expect(bar.textContent).toMatch(/核心|已连接|会话|流/);
    expect(bar.className).toMatch(/statusBar/);

    const kbds = document.querySelectorAll("kbd");
    expect(kbds.length).toBeGreaterThanOrEqual(2);
    for (const k of kbds) {
      expect(k.className).toMatch(/composerKbd|shortcutBadge|kbd/);
    }
    expect(screen.getByTestId("composer-ready-chip")).toHaveTextContent("代理已就绪");
  });
});
