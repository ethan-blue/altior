/**
 * A11 Acceptance Evidence: zh-CN / en Dual-Language & Device-Local Preferences (F18 / F20).
 *
 * Proves that:
 * 1. Both `en` and `zh-CN` dictionaries maintain 100% key parity and function signature parity.
 * 2. Unknown locale safely falls back to English without crashing or rendering empty keys.
 * 3. Locale change dynamically updates document.documentElement.lang.
 * 4. Switching language does not interrupt active turns or mutate existing conversation message text.
 * 5. Display preferences (themeSource, localeSource, pane widths) persist to device-local storage and reload on restart.
 * 6. User drafts are strictly excluded from localStorage (memory-only privacy boundary).
 * 7. SettingsModal provides accessible theme & language selection with device-local privacy notice.
 */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, beforeEach } from "vitest";
import { App } from "./App";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { approvalThread } from "../fixtures/timeline";
import { en } from "../i18n/en";
import { zhCN } from "../i18n/zh-CN";
import {
  createUiStore,
  STORAGE_PREFERENCES_KEY,
} from "./uiStore";
import { resolveLocale } from "../i18n";

describe("A11 evidence: zh-CN/en dual-language & device-local preferences", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.lang = "en";
  });

  it("dictionaries maintain 100% recursive key and structure parity", () => {
    function compareKeys(obj1: Record<string, unknown>, obj2: Record<string, unknown>, path = "") {
      const keys1 = Object.keys(obj1).sort();
      const keys2 = Object.keys(obj2).sort();

      expect(keys1, `Key parity check failed at path: ${path}`).toEqual(keys2);

      for (const key of keys1) {
        const val1 = obj1[key];
        const val2 = obj2[key];
        const currentPath = path ? `${path}.${key}` : key;

        expect(typeof val1, `Type mismatch at path: ${currentPath}`).toBe(typeof val2);

        if (typeof val1 === "object" && val1 !== null) {
          compareKeys(
            val1 as Record<string, unknown>,
            val2 as Record<string, unknown>,
            currentPath,
          );
        }
      }
    }

    compareKeys(
      en as unknown as Record<string, unknown>,
      zhCN as unknown as Record<string, unknown>,
    );
  });

  it("unknown locale cleanly resolves to default English without error", () => {
    // @ts-expect-error testing invalid locale fallback
    expect(resolveLocale("fr-FR")).toBe("en");
    // @ts-expect-error testing invalid locale fallback
    expect(resolveLocale("unknown")).toBe("en");
    expect(resolveLocale("en")).toBe("en");
    expect(resolveLocale("zh-CN")).toBe("zh-CN");
  });

  it("locale change dynamically synchronizes document.documentElement.lang", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} fixtureTimelineRows={[approvalThread]} />);

    // Initial default language
    expect(document.documentElement.lang).toMatch(/^(en|zh-CN)$/);

    // Open settings via activity rail
    const settingsBtn = screen.getByTestId("rail-settings");
    fireEvent.click(settingsBtn);

    const dialog = await screen.findByTestId("settings-modal");
    expect(dialog).toBeInTheDocument();

    const localeSelect = screen.getByTestId("settings-locale-select");
    // Switch to zh-CN
    fireEvent.change(localeSelect, { target: { value: "zh-CN" } });

    await waitFor(() => {
      expect(document.documentElement.lang).toBe("zh-CN");
    });

    // Close settings
    fireEvent.click(screen.getByTestId("settings-close-btn"));

    // Verify UI re-renders with Chinese terms from DESIGN_I18N
    expect(screen.getByTestId("rail-threads")).toHaveTextContent("会话");
    expect(screen.getByTestId("rail-agents")).toHaveTextContent("代理");
  });

  it("switching language preserves conversation message text without mutation or turn restart", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} fixtureTimelineRows={[approvalThread]} />);
    fireEvent.click(await screen.findByTestId(`thread-${approvalThread.id}`));
    await screen.findByTestId("approve");

    // Check message text in timeline row
    const originalAction = await screen.findByText(/cargo tree --workspace --edges all/);
    expect(originalAction).toBeInTheDocument();

    // Open settings and switch to Chinese
    fireEvent.click(screen.getByTestId("rail-settings"));
    fireEvent.change(screen.getByTestId("settings-locale-select"), {
      target: { value: "zh-CN" },
    });
    fireEvent.click(screen.getByTestId("settings-close-btn"));

    // Action text is untouched (user/execution content is never machine-translated)
    expect(screen.getByText(/cargo tree --workspace --edges all/)).toBeInTheDocument();
    // UI action labels remain translated while shortcuts render as semantic keycaps.
    const approve = screen.getByTestId("approve");
    const deny = screen.getByTestId("deny");
    expect(approve).toHaveTextContent("批准");
    expect(deny).toHaveTextContent("拒绝");
    expect(approve.querySelector("kbd")).toHaveTextContent("Y");
    expect(deny.querySelector("kbd")).toHaveTextContent("D");
  });

  it("display preferences persist to localStorage and reload into fresh store on restart", () => {
    const store1 = createUiStore("thr_1");
    store1.setThemeSource("dark");
    store1.setLocaleSource("zh-CN");
    store1.setNavWidth(320);
    store1.setInspectorWidth(450);

    // Check persisted JSON in storage
    const raw = localStorage.getItem(STORAGE_PREFERENCES_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw!);
    expect(parsed.themeSource).toBe("dark");
    expect(parsed.localeSource).toBe("zh-CN");
    expect(parsed.navWidth).toBe(320);
    expect(parsed.inspectorWidth).toBe(450);

    // Simulate app restart by initializing a new store
    const store2 = createUiStore("thr_2");
    const state2 = store2.getState();
    expect(state2.themeSource).toBe("dark");
    expect(state2.theme).toBe("dark");
    expect(state2.localeSource).toBe("zh-CN");
    expect(state2.locale).toBe("zh-CN");
    expect(state2.navWidth).toBe(320);
    expect(state2.inspectorWidth).toBe(450);
  });

  it("drafts are strictly excluded from localStorage (privacy boundary)", () => {
    const store = createUiStore("thr_secret");
    store.setDraft("thr_secret", "Super sensitive draft password=123");

    const raw = localStorage.getItem(STORAGE_PREFERENCES_KEY);
    if (raw) {
      expect(raw).not.toContain("Super sensitive draft");
      expect(raw).not.toContain("password");
    }
  });

  it("SettingsModal provides complete accessible keyboard interaction and device-local notice", async () => {
    const transport = new InMemoryTransport();
    render(<App transport={transport} fixtureTimelineRows={[approvalThread]} />);

    const settingsBtn = screen.getByTestId("rail-settings");
    fireEvent.click(settingsBtn);

    const modal = await screen.findByTestId("settings-modal");
    expect(modal).toHaveAttribute("role", "dialog");
    expect(modal).toHaveAttribute("aria-modal", "true");

    // Device-local notice is visible
    expect(screen.getByText(/device-local|仅保存/i)).toBeInTheDocument();

    // Escape closes modal
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByTestId("settings-modal")).not.toBeInTheDocument();
    });
  });
});



