/**
 * Renderer-owned UI state (docs/UI_ARCHITECTURE.md State ownership):
 * selection, theme, pane sizes, drafts, scroll anchors, focus. All of it
 * is ephemeral presentation state; Core owns threads/turns/events.
 */
import { useSyncExternalStore } from "react";

export type ThemeName = "light" | "dark";

export interface ThreadDraft {
  readonly text: string;
}

export interface UiState {
  readonly theme: ThemeName;
  readonly selectedThreadId: string;
  /** One draft per thread, preserved across navigation. */
  readonly drafts: Readonly<Record<string, string>>;
  /** First-visible row per thread, for scroll restoration on reopen. */
  readonly anchors: Readonly<Record<string, string>>;
  readonly inspectorOpen: boolean;
  readonly inspectorWidth: number;
  readonly navWidth: number;
}

export interface UiStore {
  getState(): UiState;
  subscribe(listener: () => void): () => void;
  toggleTheme(): void;
  selectThread(id: string): void;
  setDraft(threadId: string, text: string): void;
  setAnchor(threadId: string, rowId: string): void;
  setInspectorOpen(open: boolean): void;
  setInspectorWidth(width: number): void;
  setNavWidth(width: number): void;
}

export const INSPECTOR_MIN = 280;
export const INSPECTOR_MAX = 640;
export const NAV_MIN = 208;
export const NAV_MAX = 420;

const clamp = (value: number, min: number, max: number) =>
  Math.max(min, Math.min(max, value));

export function createUiStore(initialThreadId: string): UiStore {
  let state: UiState = {
    theme: "light",
    selectedThreadId: initialThreadId,
    drafts: {},
    anchors: {},
    inspectorOpen: true,
    inspectorWidth: 360,
    navWidth: 256,
  };
  const listeners = new Set<() => void>();
  const set = (patch: Partial<UiState>) => {
    state = { ...state, ...patch };
    for (const listener of listeners) listener();
  };
  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    toggleTheme: () =>
      set({ theme: state.theme === "light" ? "dark" : "light" }),
    selectThread: (id) => set({ selectedThreadId: id }),
    setDraft: (threadId, text) =>
      set({ drafts: { ...state.drafts, [threadId]: text } }),
    setAnchor: (threadId, rowId) =>
      set({ anchors: { ...state.anchors, [threadId]: rowId } }),
    setInspectorOpen: (open) => set({ inspectorOpen: open }),
    setInspectorWidth: (width) =>
      set({ inspectorWidth: clamp(width, INSPECTOR_MIN, INSPECTOR_MAX) }),
    setNavWidth: (width) => set({ navWidth: clamp(width, NAV_MIN, NAV_MAX) }),
  };
}

/** React binding for the UI store. */
export function useUiState(store: UiStore): UiState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}
