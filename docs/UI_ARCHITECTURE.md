# Desktop UI architecture

## Product posture

Altior is a personal knowledge and agent workbench, not a team messenger and not
a decorative AI chat page. The interface should feel closer to a good editor,
mail client, or database browser: compact, predictable, keyboard-friendly, and
comfortable for long sessions.

Reference lessons:

- **Lody**: keep the active conversation beside files, diffs, terminals, and
  previews; preserve work in tabs and resizable panes.
- **Cumora**: make streaming and agent activity legible, preserve drafts across
  navigation, and virtualize long timelines. Avoid its team roster, social
  presence, large avatars, and chat-bubble emphasis.
- **Alma**: make memory, skills, providers, projects, and settings discoverable in
  one local application. Keep technical details behind progressive disclosure.
- **Codex**: show turns, tool activity, approvals, plans, and child work as
  inspectable execution, not as hidden loading states.

## First-release information architecture

```text
+--------------------------------------------------------------------------+
| title bar | workspace tabs                                   window tools |
+----+----------------------+--------------------------------+--------------+
|rail| Threads              | Active thread                  | Inspector    |
|    | search/filter        | header: agent/project/mode     | Context      |
|    |                      |                                | Turn details |
|    | pinned               | normalized event timeline      | Files/Diff   |
|    | recent               |                                | Provenance   |
|    |                      |                                |              |
|    |                      +--------------------------------+              |
|    |                      | composer / approval / status   |              |
+----+----------------------+--------------------------------+--------------+
| status: Core | ACP process | sync/local state | background activity       |
+--------------------------------------------------------------------------+
```

The layout has five stable regions:

1. **Title and tab strip**: open work surfaces, navigation history, command
   palette, and native window controls.
2. **Activity rail**: Threads, Projects, Memory, Agents, Devices, Settings.
   Items are icons with labels available through tooltips and accessible names.
3. **Navigation pane**: compact searchable lists. Default width 256 px,
   resizable from 208 to 420 px.
4. **Workbench**: the active thread or management surface.
5. **Inspector**: one contextual pane shared by turn details, files, diffs,
   terminal output, memory provenance, and diagnostics. Default width 360 px,
   resizable from 280 to 640 px and closable.

At narrow widths the inspector becomes an overlay drawer and the navigation pane
may collapse. The first release does not implement a separate mobile layout.

## Visual system

### Principles

- Use borders, alignment, typography, and whitespace before background cards.
- Default control height is compact. Large controls are reserved for onboarding
  or destructive confirmation.
- Rounded corners are subtle and local: inputs, menus, and focused content may
  use them; primary layout surfaces remain rectangular.
- Status color is never the only signal. Pair it with an icon, label, or pattern.
- Animation communicates causality only. Respect reduced-motion settings.
- Do not display decorative "AI" gradients, glowing borders, oversized hero
  panels, or uniform card grids in the workbench.

### Tokens

Tokens live in `apps/desktop/src/styles/tokens.css` and are semantic rather than
feature-specific:

- color: canvas, surface, elevated, border, text, muted, accent, danger, warning,
  success, selection, focus
- spacing: 2, 4, 6, 8, 12, 16, 24, 32
- typography: UI, body, code; compact and normal line heights
- geometry: control heights, row heights, pane limits, border radius, divider
  width, focus ring
- elevation: menu, dialog, tooltip only
- motion: fast, normal, slow and standard easing

Components must not introduce raw colors, shadows, radii, or spacing values when
an existing semantic token applies.

## Screen plan

### Thread workbench

The header shows the thread title and only negotiated controls: ACP agent,
project, permission profile, model/mode when supported, and memory mode when that
feature ships. Unsupported choices are absent or explicitly unavailable.

The timeline renders stable Altior events:

- user and assistant messages
- thought summaries when supported and permitted
- tool calls with running/completed/failed states
- permission requests with the exact requested action and scope
- plans and progress
- terminal output and file artifacts
- turn failure, cancellation, disconnection, and indeterminate-delivery states

Tool and diagnostic rows are compact disclosure sections, not chat bubbles.
Streaming text updates in place. The viewport follows output only when the user
is already at the end; otherwise a "new activity" affordance appears.

The composer preserves one draft per thread. Sending freezes the selected
project, permission, model/mode, skill, and attachment configuration into the
turn input. Steering is shown only when the ACP capability is negotiated.

### Threads

- pinned and recent sections
- full-text search with snippets and match reason
- running, waiting-for-permission, failed, and completed indicators
- archive and delete are distinct; destructive actions require confirmation
- reopening restores the selected thread, inspector, and scroll anchor

### Agents

For the ACP-first release, an Agent row represents an ACP launch configuration:

- display name and adapter
- executable/launch source without exposing credentials
- negotiated capabilities
- authentication/configuration state
- last start result and bounded diagnostics
- test connection, edit, disable, and remove

Future harnesses add rows through the same view model; they do not create new
top-level pages.

### Projects

P1 provides registration, path display, permission scope, and thread association.
The richer file tree, Git diff, terminal, and preview inspector arrive in P4.

### Memory

P2 adds a table/list view rather than a card gallery. Columns and filters expose
scope, kind, confidence, lifecycle, provenance, last update, and sync policy.
Details open in the shared inspector.

### Devices

P3 adds pairing, fingerprints, last-seen diagnostics, revocation, recovery, and
sync status. Security state uses explicit language rather than optimistic green
badges.

### Settings

Use a flat category list and a single settings form, not nested card grids.
Every setting states whether it is device-local or synchronized. Secret fields
show configured/not configured and never echo stored credentials.

## Frontend architecture

```text
apps/desktop/
  src/
    app/             composition, routes, error boundaries, commands
    features/        threads, agents, projects, memory, devices, settings
    components/      shared product components
    primitives/      styled wrappers around accessible primitives
    ipc/             generated DTO client, validation, subscriptions
    stores/          Core snapshots and ephemeral UI state
    platform/tauri/  the only direct Tauri imports
    styles/          tokens, reset, themes, global layout rules
    fixtures/        synthetic event and screen states
    test/            browser shell, IPC fake, helpers
  src-tauri/          thin Tauri shell and capability declarations
```

Dependency direction:

```text
screens -> features -> components/primitives
   |          |
   +------ selectors/actions ------> stores -> typed IPC client
                                                   |
                                             altior-core
```

Feature components never call Tauri commands directly. The IPC client provides a
production transport and an in-memory fake that runs the same snapshots and event
fixtures in tests and component development.

## State ownership

`altior-core` owns:

- threads, turns, events, agent configurations, capabilities, permissions
- projects, memory, devices, synchronization, schedules, and durable settings
- subprocess and terminal state

The renderer owns:

- selected route/thread/item
- open tabs and inspector kind
- pane sizes and collapsed state
- composer drafts and attachment staging
- scroll anchors and disclosure state
- theme source and other presentation preferences after Core confirms them

Core-backed state uses immutable snapshots and subscriptions compatible with
React's external-store contract. UI state stays feature-scoped; there is no
single untyped global object containing the whole application.

## IPC and streaming

1. Desktop performs a version handshake.
2. It requests an initial bounded snapshot for the visible surface.
3. It subscribes before issuing commands that may emit events.
4. Events carry sequence and operation identifiers and reduce into immutable
   feature snapshots.
5. A gap pauses optimistic display, requests catch-up, and surfaces diagnostics
   if recovery fails.
6. Reconnection never resends an indeterminate prompt.

High-frequency message deltas are coalesced to a bounded paint cadence. Durable
event history remains in Core; the renderer may discard offscreen derived state
and rebuild it from a snapshot.

## Component contract

Every shared component includes:

- typed props with no domain-service imports
- keyboard and focus behavior
- light, dark, high-contrast, narrow, loading, empty, error, and disabled states
  where applicable
- a fixture or story for every meaningful state
- behavior tests for interaction logic
- a visual baseline when geometry or styling is load-bearing

Shared primitives must not encode a product workflow. Product components must
not reimplement buttons, menus, dialogs, tooltips, listboxes, or focus traps.

## Performance and accessibility budgets

- useful history visible within the global 2-second startup target
- no full-timeline rerender for a single stream delta
- interactive search at the acceptance dataset size
- keyboard access to every command and pane
- visible focus in every theme
- no focus loss when virtualized rows mount or unmount
- screen-reader announcements for permission requests, failures, and completed
  turns; streaming deltas are not announced character by character

## ACP-first boundary

The first release implements only the ACP journey:

```text
configure ACP agent
 -> verify capabilities/authentication
 -> create thread
 -> prompt and stream
 -> approve/deny tool request
 -> steer or cancel when supported
 -> restart Desktop/Core
 -> resume without duplicate delivery
 -> inspect durable history and diagnostics
```

Terminal, Codex app-server, Native Harness, multi-agent delegation UI, memory
automation, sync, and the complete project workbench do not block this journey.
Their future locations and capability seams are documented, but placeholder UI
must not be shipped merely to make the app look broader.

## Verification

- reducer tests use synthetic ACP transcripts, including unknown and malformed
  events
- component tests cover keyboard, focus, disclosure, permission, and error states
- Playwright runs against the in-memory IPC fake for deterministic journeys
- reviewed screenshots cover the shell, thread timeline, approval, settings,
  empty state, failure state, and both themes on one pinned Windows environment
- a smaller packaged-app smoke suite verifies Tauri/Core handshake, real window
  capabilities, one real ACP agent, restart, and process cleanup
