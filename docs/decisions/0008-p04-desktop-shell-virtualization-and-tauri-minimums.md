# ADR 0008: P0.4 desktop shell, virtualized timeline, and Tauri capability minimums

Date: 2026-08-29
Status: accepted

## Context

`docs/IMPLEMENTATION_PLAN.md` P0.4 asks for the classic workbench shell,
proof of virtualized streaming, prepend-stable history, scroll restoration,
inspector resizing, keyboard navigation, an inline approval flow, and visual
baselines on the pinned Windows environment. ADR 0005 deliberately deferred
`src-tauri/` and `platform/tauri/` to this spike, so the Tauri shell also
arrives here. Protocol v1 events are still minimal
(turn.started / message.delta / turn.completed / stream.gap /
stream.replayed); the full taxonomy lands with the P1 domain runtime, so
P0.4 cannot render "real" tool or permission protocol events yet.

Reference lessons pinned in `docs/UI_ARCHITECTURE.md` drive the design:
Lody keeps the conversation beside resizable panes; Cumora proves streaming
legibility and virtualized long timelines; Codex renders execution as
inspectable rows, not hidden loading states.

## Decision

### 1. Scope: interaction mechanics on the in-memory fake

P0.4 proves the Desktop interaction contract against the existing
in-memory transport. No protocol DTO, command, or fixture format changes.
The timeline consumes a provisional synthetic row model
(`src/fixtures/timeline.ts`) shaped to match the P0.3 normalized agent
events and the preserved-unknown rule; it is replaced by the frozen P1
event taxonomy without touching the windowing or keyboard layers.

### 2. Shell layout

Five stable regions exactly as `docs/UI_ARCHITECTURE.md` draws them:
title/tab strip, activity rail, navigation pane (256 px default, 208–420),
workbench, inspector (360 px default, 280–640, closable). CSS grid with
token-driven sizing; pane resizing is pointer-drag with clamped deltas and
keyboard-operable via the same resize handles. Narrow widths collapse the
navigation pane; the inspector becomes an overlay drawer below a token
threshold. No decorative gradients, no chat-bubble emphasis: tool and
diagnostic rows are compact disclosure sections.

### 3. Virtualization: a pure, zero-dependency windowing engine

`src/features/timeline/virtualWindow.ts` is a pure module: offset math
(estimated heights corrected by measurement, window + overscan, prepend
shift, anchor restore) with no DOM imports. The React layer is a thin
absolute-positioned renderer over that math. jsdom has no layout, so the
acceptance math (100k rows, prepend stability, anchor restore) is proven at
the module level; component tests inject a fake measurer.

Prepend-stable history: when older rows are prepended, the engine returns a
`scrollShift` equal to the measured height of the prepended block so the
viewport keeps its first visible row. Follow-tail: the viewport sticks to
the end only while the user was already at the end (with a threshold); a
"new activity" affordance appears otherwise.

### 4. Streaming: coalesced deltas, per-row memoization

Message deltas reduce into the last row's text through an external store;
rows are memoized components subscribing per-row. Acceptance is a
render-count test: appending many deltas to a 100k-row timeline re-renders
only the streaming row. The composer emits turns through the transport's
existing command path; nothing here adds a protocol command.

### 5. Inline approval: a provisional UI decision

Permission request rows render the exact requested action and scope with
Approve/Deny controls (buttons plus keyboard shortcuts) and a roving-focus
path. Because protocol v1 has no permission-answer command, the decision is
recorded as a typed UI-store decision and surfaced in the inspector;
`docs/IMPLEMENTATION_PLAN.md` P1.2/P1.3 owns the real command. This is
marked provisional in code and docs; no silent fallback.

### 6. Tauri shell with minimum capabilities

`apps/desktop/src-tauri/` arrives now: Tauri v2, a thin shell (no plugin
commands), strict CSP, `withGlobalTauri: false`, and a capabilities file
whose allowlist is the declared minimum (core defaults only). The crate is
intentionally **outside** the Rust workspace so repository gates stay
hermetic and fast; a Desktop test statically pins the capability allowlist
and CSP so capability creep fails CI. Full packaged-app smoke (real
webview, real Core handshake) stays with P5 release hardening, per ADR
0005's separation of the fixture shell from the packaged app.

### 7. Visual baselines via Playwright, opt-in browser download

A Playwright script drives the vite preview server with the same synthetic
fixtures and captures light / dark / narrow / error / approval screenshots
into `apps/desktop/baselines/`. Screenshots are operator-reviewed evidence,
not image-diff gates (image diffing on CI machines is the P5 visual
regression audit). If the browser download is unavailable, the script
stands and the run is an explicit operator action — mirroring the ADR 0007
opt-in smoke stance.

## Alternatives considered

- **react-virtuoso** for the timeline: mature, handles variable heights
  and prepend, but adds a runtime dependency whose internal bailout and
  scroll-anchoring semantics we cannot assert deterministically in jsdom,
  and its behavior changes under us across versions. Revisit if the engine
  below cannot carry P1 content.
- **@tanstack/react-virtual**: smaller and headless, still owns the scroll
  anchoring; same testability argument.
- **CSS `content-visibility: auto`**: no dependency, but no control over
  prepend anchoring, follow-tail, or focus retention on recycled rows.
- **Fixed row heights**: simplest math, but messages are inherently
  variable-height; faking uniform rows would ship a layout lie.
- **Adding the permission-answer protocol command now**: would freeze a
  contract before the ACP runtime (P1.2) proves the real flow; deferred.
- **Putting `src-tauri` in the workspace**: couples every `cargo test`
  run to the Tauri dependency tree and its system webview requirements.

## Failure modes

- **Measurement thrash**: a corrected height re-renders the window and
  re-measures; corrections are clamped and monotonic per row id, and the
  engine prefers estimates until a row is actually rendered.
- **Focus loss on row recycle**: the roving tabindex is keyed by row id,
  not by index; a focused row that scrolls out of the window keeps focus
  on its anchor element when it remounts (asserted in tests).
- **Streaming rerender**: any new non-memoized row component can silently
  reintroduce full-timeline renders; the render-count test is the guard.
- **Capability creep**: adding a Tauri permission without updating this
  ADR fails the static allowlist test.
- **jsdom divergence**: module-level math is the source of truth; DOM
  tests assert wiring, not geometry.

## Migration

P1 replaces the provisional fixture row model with the frozen event
taxonomy behind the same `TimelineRow` shape; the windowing engine and
keyboard layer are unchanged. P1.2 wires the real permission command where
the provisional UI decision now sits.

## Exit strategy

If the timeline outgrows the custom engine (embedded diffs, terminal
streams with partial invalidation), swap `virtualWindow.ts` for a library
behind the same pure interface; components and tests keep their contract.

## Revisit triggers

- a P1 row kind that needs partial invalidation or nested virtualization
- a measured performance miss at the acceptance dataset size in a real
  browser (Playwright baselines exist precisely to catch this)
- Tauri requiring a capability beyond the declared minimum for the P1
  journey
