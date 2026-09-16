/**
 * Renderer-owned UI state (docs/UI_ARCHITECTURE.md State ownership, DESIGN_I18N §3, A11):
 * selection, theme, language/locale, pane sizes, drafts, scroll anchors, focus.
 *
 * Local display preferences (theme, language, pane widths) are saved to device-local
 * storage and never synchronized. Drafts remain memory-only and are never written to storage.
 */
import { useSyncExternalStore } from "react";
import { resolveLocale, type LocaleSource, type SupportedLocale } from "../i18n";
export type { LocaleSource, SupportedLocale };

export type ThemeName = "light" | "dark";
export type ThemeSource = "system" | ThemeName;

export interface ThreadDraft {
  readonly text: string;
}

export interface UiPreferences {
  readonly themeSource: ThemeSource;
  readonly localeSource: LocaleSource;
  readonly navWidth?: number;
  readonly inspectorWidth?: number;
}

export interface UiState {
  readonly themeSource: ThemeSource;
  readonly theme: ThemeName;
  readonly localeSource: LocaleSource;
  readonly locale: SupportedLocale;
  readonly selectedThreadId: string;
  /** One draft per thread, preserved across navigation. Never persisted to storage. */
  readonly drafts: Readonly<Record<string, string>>;
  /** First-visible row per thread, for scroll restoration on reopen. */
  readonly anchors: Readonly<Record<string, string>>;
  readonly inspectorOpen: boolean;
  readonly inspectorWidth: number;
  readonly navWidth: number;
  readonly settingsOpen: boolean;
  readonly activeRail: string;
}

export interface UiStore {
  getState(): UiState;
  subscribe(listener: () => void): () => void;
  toggleTheme(): void;
  setThemeSource(source: ThemeSource): void;
  setLocaleSource(source: LocaleSource): void;
  setSettingsOpen(open: boolean): void;
  selectThread(id: string): void;
  setDraft(threadId: string, text: string): void;
  setAnchor(threadId: string, rowId: string): void;
  setInspectorOpen(open: boolean): void;
  setInspectorWidth(width: number): void;
  setNavWidth(width: number): void;
  setActiveRail(rail: string): void;
}

export const INSPECTOR_MIN = 280;
export const INSPECTOR_MAX = 640;
export const NAV_MIN = 208;
export const NAV_MAX = 420;

export const STORAGE_PREFERENCES_KEY = "altior_ui_preferences_v1";

const clamp = (value: number, min: number, max: number) =>
  Math.max(min, Math.min(max, value));

export function getSystemTheme(): ThemeName {
  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  ) {
    return "dark";
  }
  return "light";
}

export function resolveTheme(source: ThemeSource): ThemeName {
  if (source === "system") {
    return getSystemTheme();
  }
  return source === "dark" ? "dark" : "light";
}

export function loadStoredPreferences(): Partial<UiPreferences> {
  if (typeof localStorage === "undefined") return {};
  try {
    const raw = localStorage.getItem(STORAGE_PREFERENCES_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Partial<UiPreferences>;
    return {
      themeSource:
        parsed.themeSource === "light" ||
        parsed.themeSource === "dark" ||
        parsed.themeSource === "system"
          ? parsed.themeSource
          : undefined,
      localeSource:
        parsed.localeSource === "zh-CN" ||
        parsed.localeSource === "en" ||
        parsed.localeSource === "system"
          ? parsed.localeSource
          : undefined,
      navWidth:
        typeof parsed.navWidth === "number"
          ? clamp(parsed.navWidth, NAV_MIN, NAV_MAX)
          : undefined,
      inspectorWidth:
        typeof parsed.inspectorWidth === "number"
          ? clamp(parsed.inspectorWidth, INSPECTOR_MIN, INSPECTOR_MAX)
          : undefined,
    };
  } catch {
    return {};
  }
}

export function saveStoredPreferences(prefs: UiPreferences): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(
      STORAGE_PREFERENCES_KEY,
      JSON.stringify({
        themeSource: prefs.themeSource,
        localeSource: prefs.localeSource,
        navWidth: prefs.navWidth,
        inspectorWidth: prefs.inspectorWidth,
      }),
    );
  } catch {
    // Ignore quota or security errors in constrained environments
  }
}

export function createUiStore(
  initialThreadId: string,
  initialPreferences?: Partial<UiPreferences>,
): UiStore {
  const stored = initialPreferences ?? loadStoredPreferences();

  const themeSource: ThemeSource = stored.themeSource ?? "system";
  const localeSource: LocaleSource = stored.localeSource ?? "system";

  let state: UiState = {
    themeSource,
    theme: resolveTheme(themeSource),
    localeSource,
    locale: resolveLocale(localeSource),
    selectedThreadId: initialThreadId,
    drafts: {},
    anchors: {},
    inspectorOpen: true,
    inspectorWidth: stored.inspectorWidth ?? 360,
    navWidth: stored.navWidth ?? 256,
    settingsOpen: false,
    activeRail: "threads",
  };

  const listeners = new Set<() => void>();
  const set = (patch: Partial<UiState>) => {
    state = { ...state, ...patch };
    for (const listener of listeners) listener();
  };

  const persist = () => {
    saveStoredPreferences({
      themeSource: state.themeSource,
      localeSource: state.localeSource,
      navWidth: state.navWidth,
      inspectorWidth: state.inspectorWidth,
    });
  };

  // Listen to system dark-mode preference changes
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
      const onMediaChange = () => {
        if (state.themeSource === "system") {
          set({ theme: getSystemTheme() });
        }
      };
      if (typeof mediaQuery.addEventListener === "function") {
        mediaQuery.addEventListener("change", onMediaChange);
      } else if (typeof (mediaQuery as unknown as { addListener?: (fn: () => void) => void }).addListener === "function") {
        (mediaQuery as unknown as { addListener: (fn: () => void) => void }).addListener(onMediaChange);
      }
    } catch {
      // media query unsupported
    }
  }

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    toggleTheme: () => {
      const nextTheme: ThemeName = state.theme === "light" ? "dark" : "light";
      set({
        themeSource: nextTheme,
        theme: nextTheme,
      });
      persist();
    },
    setThemeSource: (source: ThemeSource) => {
      set({
        themeSource: source,
        theme: resolveTheme(source),
      });
      persist();
    },
    setLocaleSource: (source: LocaleSource) => {
      set({
        localeSource: source,
        locale: resolveLocale(source),
      });
      persist();
    },
    setSettingsOpen: (open: boolean) => set({ settingsOpen: open }),
    selectThread: (id) => set({ selectedThreadId: id }),
    setDraft: (threadId, text) =>
      set({ drafts: { ...state.drafts, [threadId]: text } }),
    setAnchor: (threadId, rowId) =>
      set({ anchors: { ...state.anchors, [threadId]: rowId } }),
    setInspectorOpen: (open) => set({ inspectorOpen: open }),
    setInspectorWidth: (width) => {
      set({ inspectorWidth: clamp(width, INSPECTOR_MIN, INSPECTOR_MAX) }),
      persist();
    },
    setNavWidth: (width) => {
      set({ navWidth: clamp(width, NAV_MIN, NAV_MAX) }),
      persist();
    },
    setActiveRail: (rail) => set({ activeRail: rail }),
  };
}

/** React binding for the UI store. */
export function useUiState(store: UiStore): UiState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}

