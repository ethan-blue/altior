# Implementation plan

## Strategy

Build one complete local ACP path before expanding the product. Architectural
seams for other harnesses remain typed and tested, but unused breadth is not a
milestone. Each phase ends in a user-visible vertical journey and deterministic
acceptance evidence.

```text
P0 contracts and spikes
        |
P1 ACP desktop continuity
        |
P2 identity and memory
        |
P3 encrypted Personal Vault sync
        |
P4 project workbench and extensions
        |
P5 release hardening
        |
Later: additional harnesses and bounded multi-agent coordination
```

## P0: Freeze executable foundations

### P0.1 Repository and contract skeleton

- establish crate and Desktop workspace boundaries
- define IDs, clocks, cancellation, typed error, and bounded payload conventions
- version Desktop/Core handshake, commands, snapshots, and event envelopes
- generate TypeScript DTOs or validate them from one Rust-owned schema source
- add synthetic protocol fixture layout and compatibility test harness

Status: implemented in full (ADR 0004, ADR 0005): stable domain IDs, clock
and delivery-state conventions (UnixMillis, LogicalTick, DeliveryState),
version-range handshake negotiation with explicit capability declarations,
versioned command/event/snapshot envelopes with bounded payloads and
unknown-event preservation, a cancellation command convention, typed
errors, deterministic JSON fixtures including old/new handshake
compatibility vectors, TypeScript DTOs generated from the Rust contracts
(ts-rs behind the `dto-export` feature), a Desktop fixture shell running
on the in-memory transport with Rust-owned fixtures, and a
dependency-boundary test. The production OS transport remained P0.2
work.

Evidence:

- dependency-boundary test
- old/new handshake compatibility fixtures
- invalid and oversized IPC payload rejection
- unknown-event preservation roundtrips (raw-provider wrap and the
  preserved fixed-point form Desktop replays)
- Desktop fixture shell runs with the in-memory transport (strict
  typecheck and Vitest suite green)

### P0.2 Core supervision and local IPC

- select and record the local IPC transport and Core process model
- start, discover, health-check, reconnect, and shut down Core safely
- separate Desktop reload/window close from Core turn ownership
- add sequence/catch-up semantics for event subscriptions

Status: implemented in full (ADR 0006). The `altior-ipc` crate owns local
sockets (Windows named pipe / Unix domain socket endpoints derived from
the user name), 4-byte length-prefixed bounded JSON frames, per-launch
hex capability tokens, and the pure session machines: a Core-side
`EventLog` with monotonic sequences and bounded retention, per-connection
`ServerSession`s sharing one log per launch (reload is a new connection
over the same log), and a Desktop-side `ClientSession` with epoch
tracking (resume vs restart), `event_id` deduplication, and a
`CommandLedger` refusing duplicate `OperationId` issues. `altior-core`
adds the spawn-or-attach `Supervisor` state machine (probe, spawn,
attach, bounded-escalation reconnect — timers deliberately excluded so
tests drive it deterministically), turn ownership (Desktop lifecycle
events are provably inert for running turns), and the operation registry
for receiving-side dedup. The OS transport (tokio named pipes / UDS) is
deliberately a named later slice; every machine here is transport-free
and exercised in-process.

Evidence:

- UI reload does not stop a synthetic active turn
  (`crates/altior-core/tests/p02_evidence.rs`)
- Core restart exposes recovery state without duplicate commands
  (`crates/altior-core/tests/p02_evidence.rs`,
  `crates/altior-ipc/tests/session_recovery.rs`)
- wrong protocol version and stale socket/pipe fail clearly
  (`crates/altior-core/tests/p02_evidence.rs`, typed
  `NoCommonProtocolVersion` / stale-endpoint classification)
- retained-window replay with `stream.replayed` boundary, `stream.gap`
  for evicted ranges, and epoch change on `CoreInstanceId` mismatch
  (`crates/altior-ipc/tests/session_recovery.rs`)
- dependency-boundary test extended to `altior-ipc`
  (`crates/altior-core/tests/dependency_boundaries.rs`)

### P0.3 ACP v1 spike

- implement a narrow ACP adapter outside the domain crates
- connect to two real external agents in opt-in smoke tests
- negotiate capabilities rather than inspecting version strings
- map create, prompt, delta, tool, permission, cancel, resume, and failure events
- classify delivery as absent, confirmed, rejected, or indeterminate

Status: implemented in full (ADR 0007). The `altior-acp` crate is a
replaceable adapter beside the contracts (same dependency boundary as
`altior-ipc`, asserted by the dependency-boundary test): newline-delimited
JSON-RPC 2.0 over a subprocess's stdin/stdout with a 1 MiB line cap,
negotiation on capabilities only (the agent's protocol version is recorded
as opaque data, never a feature gate), a mapping table from the modeled ACP
v1 subset onto protocol event bodies with everything unknown preserved
verbatim under `acp.*` provider kinds, prompt delivery classified onto the
frozen `DeliveryState` vocabulary (crash and idle timeout are always
Indeterminate, never Absent; only Absent/Rejected may resend), and a pure
lifecycle machine for cancel, crash, idle timeout, and cleanup with no
timers. The smoke host is opt-in via `ALTIOR_ACP_SMOKE_AGENTS` (two
`;;`-separated agent command lines, watchdog-killed at 120 s); default
gates never spawn processes, touch the network, or depend on credentials.
Running it against two real agents is an explicit operator action with
per-machine credentials, so the in-repo evidence is the fixture-pinned
normalizer plus the deterministic machine tests.

Evidence:

- normalized trace fixtures for two agents
  (`crates/altior-acp/tests/traces.rs`, fixtures regenerated only by the
  `#[ignore]`d regeneration test, same pattern as dto-export)
- unknown event and malformed frame tests
  (`crates/altior-acp/tests/unknown_and_malformed.rs`: unknown kinds
  preserve; bad JSON, wrong envelope, missing discriminator, oversized and
  non-UTF-8 lines fail with typed errors)
- process crash, idle timeout, cancellation, and cleanup tests
  (`crates/altior-acp/tests/lifecycle.rs`: crash is Indeterminate and
  collapses straight to reap; cancel answers permissions then waits for the
  settled turn; idle kills without waiting; settled cleanup kills and reaps)
- dependency-boundary test extended to `altior-acp`
  (`crates/altior-core/tests/dependency_boundaries.rs`)
- opt-in smoke harness (`crates/altior-acp/tests/smoke.rs`) proves the
  initialize → session/new → prompt → normalize → confirm-delivery →
  kill-and-reap flow end to end; it skips with a message when the env var
  is unset, and the real-agent run stays an operator action with its own
  credentials — the harness never logs transcript content, only counts and
  delivery classification

### P0.4 Desktop interaction spike

- implement the classic workbench shell and design tokens
- prove virtualized streaming, prepend-stable history, scroll restoration, and
  inspector resizing
- prove keyboard navigation and an inline approval flow
- establish visual baselines on the pinned Windows environment

Status: implemented in full (ADR 0008). `apps/desktop` runs the
five-region workbench shell (activity rail, threads pane, timeline
workbench, inspector, status bar) on token-only CSS with a dark-theme
override, narrow-viewport drawer behavior, and keyboard-operable
resizing clamped to token ranges. The timeline virtualizes with a
zero-dependency engine: a prefix-sum height index with window plus
overscan, prepend shift, and scroll-anchor restore. jsdom has no
layout, so component tests inject viewport height and measured heights
while the pure math is proven separately at 100,000 rows. Streaming is
coalesced through a row-scoped store: each row subscribes via
`useSyncExternalStore`, so a delta rerenders only its own row, never
the mounted timeline. Permission requests render inline with a
provisional UI decision (protocol v1 has no permission-answer command;
the P1 runtime owns the real answer), and unknown events stay
preserved, inspectable rows. The Tauri v2 shell is a separate crate
outside the Rust workspace (empty `[workspace]`, so repo gates stay
hermetic) and its capability minimums — one main window, no global
API, strict CSP, `core:default` only — are pinned by tests. Playwright
baselines are captured on demand via `npm run baselines` and are
operator-reviewed evidence, not image-diff gates; the visual regression
audit is P5.

Evidence:

- window math and store invariants at 100,000 rows
  (`apps/desktop/src/features/timeline/virtualWindow.test.ts` and
  `timelineStore.test.ts`: window bounds and clamping, prepend shift,
  anchor restore, per-row subscription, cached snapshots)
- 100,000-row interactivity in the DOM — window ≤ 60 mounted rows,
  roving keyboard navigation, focus surviving Home/End row recycling
  (`apps/desktop/src/app/p04Evidence.test.tsx`)
- a streamed delta mutates only its own row (a MutationObserver over
  the scroller proves every mutation lands inside the streamed row's
  subtree; every other mounted row keeps byte-identical text)
- prepend-stable history and scroll restoration (prepending older
  history shifts scrollTop by exactly the prepended block height while
  the anchor stays first-visible; reopening a thread restores the
  remembered anchor)
- inline approval and keyboard decisions (approve/deny buttons plus
  `y`/`d` from the focused row, aria-live announcement, provisional
  decision chip)
- Tauri capability minimums pinned statically
  (`apps/desktop/src/platform/tauri/tauriConfig.test.ts`)
- light/dark/narrow/error/approval baselines captured in real Chromium
  via `npm run baselines` (`apps/desktop/scripts/baselines.mjs`,
  screenshots in `apps/desktop/baselines/`)

### P0.5 Storage, CRDT, crypto, and relay spikes

These spikes select later-phase foundations but do not expand the P1 UI:

- SQLite migration and journal-to-projection rebuild
- Loro/Automerge adversarial `SyncDocumentEngine` bake-off
- standard cryptographic library and encrypted two-device envelope spike
- relay transport and compaction model

Exit: ADRs select IPC, process model, storage, CRDT, crypto, and UI dependencies;
ACP and Desktop contracts have executable fixtures.

## P1: ACP desktop continuity

### P1.1 Domain and persistence

- implement AgentProfile, ACP HarnessBinding, Thread, Turn, Event, Permission,
  Project reference, and operation IDs
- add append-safe local turn/event persistence and SQLite projections
- add migrations, recovery markers, and projection rebuild
- implement bounded thread list, history, and search queries

### P1.2 ACP runtime

- implement launch configuration and OS-secret references
- create/resume sessions, prompt, stream, steer/cancel where supported, and close
- persist delivery checkpoints before/after adapter boundaries
- supervise subprocess trees and recover or surface interrupted turns
- keep raw ACP diagnostics bounded and device-local

### P1.3 Desktop MVP

- onboarding: start Core and configure/test one ACP agent
- workbench shell: activity rail, thread list, timeline, composer, inspector
- thread create/open/search/pin/archive
- normalized message, tool, plan, permission, terminal-like output, error, and
  cancellation renderers
- Agents and Settings surfaces needed to manage ACP
- minimal Projects surface for associating an approved local path

### P1.4 Acceptance journey

1. Install or start on a clean Windows profile.
2. Configure ACP agent A and agent B without exposing credentials.
3. Create and complete a thread with streaming and a permission decision.
4. Cancel another turn without leaking a subprocess.
5. Reload Desktop while work continues.
6. Restart Core and resume a prior thread.
7. Simulate indeterminate delivery and verify no automatic resend.
8. Search and reopen durable history offline.

Exit: this journey passes for two ACP agents. Terminal, Codex app-server,
Altior-native execution, multi-agent delegation, and sync are not P1 scope.

## P2: Personal identity and memory

- implement identity documents above ACP
- implement ContextSnapshot assembly and token budgeting
- add memory candidate, confirmation, correction, rejection, expiry, and forgetting
- reject secret-shaped content before durable writes
- add bounded explainable FTS retrieval and source evidence
- add Memory table, provenance inspector, context diagnostics, and explicit controls

Acceptance:

- a new ACP thread recalls a confirmed fact
- the user can inspect why it was selected
- correction supersedes without erasing history
- forgetting removes it from future context
- credential fixtures never reach the journal or projection

## P3: Personal Vault synchronization

- device identity, pairing fingerprints, recovery, revocation, and key rotation
- encrypted journal, CRDT document, and opt-in blob planes
- untrusted relay catch-up, acknowledgements, replay protection, and quotas
- compaction that preserves tombstone and revocation frontiers
- Devices and sync diagnostics UI

Acceptance:

- three devices converge after offline create/correct/forget conflicts
- local conversation and memory work remains available without network
- relay inspection reveals no plaintext
- a revoked device cannot publish accepted events
- a 30-day simulated offline device cannot resurrect forgotten memory

## P4: Project workbench and extensions

- file tree, file preview, Git status/diff, and approved PTY through Core ports
- attachments and content-addressed blob handling
- MCP and skill registries above ACP
- scheduler/heartbeat foundation
- permission profiles and path containment
- inspector tabs for Files, Diff, Terminal, Context, and Provenance

Acceptance:

- an ACP agent works inside an approved project
- the user reviews terminal activity and changes beside the thread
- denied paths remain inaccessible
- skill and MCP selection is frozen into the driving turn input

## P5: Release hardening

- Windows packaging, signing, updater, rollback, backup, and restore
- crash recovery, soak, resource, corruption, migration, and downgrade suites
- support bundle with secret redaction and bounded diagnostics
- accessibility and visual regression audit
- license and third-party provenance audit

Exit: all targets in `ACCEPTANCE.md` pass on the reference clean Windows host.

## Later: additional harnesses

Only after the ACP product path is stable:

1. evaluate Terminal Harness as a universal compatibility path
2. evaluate Codex app-server over local stdio for richer native integration
3. run the same normalized harness suite against each adapter
4. add explicit thread rebinding and context-bridge provenance
5. implement bounded parent/child operations for multi-agent work
6. consider an Altior-native harness only when external harnesses cannot deliver a
   documented product outcome

Additional adapters do not change AgentProfile, Thread, Turn, Permission, Event,
Memory, or synchronization records.

## Dependency rules

- P1 implementation waits for its P0 protocol and process contracts.
- P2 may design fixtures during P1 but does not inject memory until turn delivery
  and context provenance are stable.
- P3 uses frozen memory lifecycle and document contracts.
- P4 uses the stable permission and project ports proven by P1.
- P5 hardening begins continuously, but release acceptance follows all migrations
  and durable formats intended for the release.

## Planning and reporting

Each work item follows `AI_DEVELOPMENT.md` and links to one phase outcome. Progress
is reported as acceptance evidence completed, not as percentage estimates or code
volume. A phase is not complete while a required failure case remains mocked,
manual-only, or dependent on a real sleep/public network.
