# ADR 0003: Build an ACP-first desktop workbench

- Status: Accepted
- Date: 2026-08-29

## Context

Altior needs a useful desktop product before it needs an Altior-owned model and
tool loop. ACP already offers a replaceable integration boundary and lets the
project validate thread continuity, streaming, permissions, cancellation, and
resume against real agents. Building several harnesses at once would multiply
protocol and UI states before the stable domain contract has production evidence.

The UI must also remain maintainable when most implementation work is performed
by coding agents. Large unstructured component trees, arbitrary utility classes,
duplicated state, and undocumented visual conventions are especially likely to
drift under parallel AI-authored changes.

## Decision

The first usable release is ACP-only. Terminal, Codex app-server, and an
Altior-native harness remain represented by capability-driven interfaces and
fixtures, but are not production implementations for the first release.

The Desktop stack is:

- Tauri 2 as a thin native shell
- React and TypeScript rendered by Vite
- React Aria Components for unstyled accessible interaction primitives
- CSS Modules plus a small global design-token layer
- TanStack Virtual for long thread, event, search, and log views
- Vitest and Testing Library for pure/component behavior
- Playwright for deterministic browser-shell flows and visual baselines

The renderer never owns durable product data. `altior-core` remains authoritative
and exposes versioned snapshot, command, and event contracts. The renderer keeps
only view state such as pane sizes, selection, draft text, tab layout, and scroll
anchors. A typed external store subscribes to Core snapshots and event streams;
feature components consume selectors rather than invoking IPC directly.

Tauri capabilities are minimized by window. The main webview talks only to the
Desktop/Core IPC bridge and narrowly required native window/dialog commands. It
does not receive general filesystem, shell, process, database, or network access.

The visual language is a compact classic desktop workbench:

- flat surfaces separated by borders, not a wall of rounded cards
- persistent navigation and resizable panes
- dense rows with clear selection and keyboard focus
- restrained use of badges; status text and icons carry actual state only
- chat, approvals, tool activity, files, diffs, and provenance remain inspectable
  without replacing the active thread
- light and dark themes share semantic tokens and identical information hierarchy

## Alternatives

### Electron

Electron provides mature packaging and a uniform Chromium runtime, and Lody and
Cumora demonstrate its suitability for agent workbenches. It was rejected for
the initial implementation because Altior already requires a Rust core and has a
strict memory budget. Tauri keeps the native shell narrow and avoids a second
Node-owned application backend.

### Tailwind as the primary styling system

Tailwind enables fast local iteration, as demonstrated by the reference apps.
It was rejected as the primary design-system layer because AI-authored utility
strings tend to duplicate tokens, introduce arbitrary values, and drift toward
generic card-heavy layouts. CSS Modules make component ownership explicit while
global tokens keep the visual system consistent. Utility classes may be added
later only through an ADR and a bounded preset.

### A large pre-styled component library

This would accelerate scaffolding but impose another product's density, geometry,
and theme assumptions. Altior instead uses accessible headless primitives and
owns its presentation.

### Renderer-side database or query cache as source of truth

This was rejected because the Desktop is a client of `altior-core`. Duplicating
durable truth would create restart, migration, and synchronization ambiguity.

## Failure modes and mitigations

- **Streaming rerenders the whole thread**: normalize event state, use selectors,
  batch deltas, and virtualize the timeline.
- **Virtualization breaks scroll anchoring**: test prepend, append, streaming
  height changes, jump-to-event, and restore behavior with synthetic transcripts.
- **Webview gains excessive host authority**: deny by default with Tauri window
  capabilities and keep file/process operations in Core ports.
- **AI changes create visual drift**: require stories/state fixtures, semantic
  tokens, and reviewed Playwright screenshots for shared components and screens.
- **ACP-specific UI leaks into the domain**: feature code renders normalized
  events and negotiated capabilities; raw ACP DTOs remain in the adapter.
- **The first release becomes a framework exercise**: implement only components
  required by the ACP continuity acceptance journey.

## Migration and exit strategy

React is isolated behind typed IPC clients, feature stores, and ordinary DOM/CSS.
Tauri-specific calls stay in `src/platform/tauri`. A future renderer or shell can
reuse protocol fixtures and acceptance journeys without moving domain logic.

React Aria and TanStack Virtual are replaceable implementation dependencies. No
domain or IPC DTO may contain their types. If either cannot meet accessibility,
performance, or WebView2 behavior targets during P0, replace it before P1 rather
than adding application-level workarounds.

Exact dependency versions are selected and pinned during the P0 Desktop spike.
Do not use broad ranges for load-bearing runtime or test dependencies.

## Upstream references

- [Tauri capabilities](https://v2.tauri.app/reference/acl/capability/)
- [React external-store subscription](https://react.dev/reference/react/useSyncExternalStore)
- [React Aria](https://react-spectrum.adobe.com/react-aria/getting-started.html)
- [TanStack Virtual chat behavior](https://tanstack.com/virtual/latest/docs/chat)
- [Playwright visual comparisons](https://playwright.dev/docs/test-snapshots)

## Revisit when

- the ACP-only release passes its continuity and resource acceptance targets
- a second production harness is approved
- WebView2 behavior prevents a required interaction or performance target
- the component catalog shows that the styling strategy is slowing delivery
