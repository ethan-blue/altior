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

Status: hardened runnable spikes complete (ADR 0009–0012), not production
sync or persistence completion.
Four spike crates live outside the contract core, each pinned by the
dependency-boundary test. `altior-storage` (ADR 0009) proves the
durable-ownership rule: forward-only `user_version` migrations that
refuse newer schemas, an append-only journal guarded by
update/delete triggers, payload-checked idempotent `event_id` appends,
bounded unsigned journal reads, thread projections with independent
journal and fold-version recovery markers, and a single-transaction
rebuild proven equal to the incremental fold — including self-healing
reopens after stale and missing markers. `altior-crdt` (ADR 0010)
implements the `SyncDocumentEngine` port twice (Loro 1.13.9,
Automerge 0.11.0) and races them under deterministic adversarial
schedules: concurrent same-offset inserts, delete/insert races, merge
idempotence and order independence, star topologies under a seeded
LCG, and stale-fork catch-up — all converging on both engines. The
measured bake-off (fixed 1000-op script, including Altior framing):
automerge 10150 bytes vs loro 14804 bytes with identical views. Decision:
Automerge for P1 (smaller state, mature delta-sync protocol), Loro
retained as the always-raced second engine; the discovered
container-creation race is neutralized by typed schema-first
`with_schema`; internal namespaces are encoded, and malformed/cross-engine
state imports are bounded typed failures. `altior-crypto` (ADR 0011) builds the two-device
envelope from standard primitives (X25519 static-static ECDH,
HKDF-SHA256, ChaCha20-Poly1305): direction-separated keys, counter
nonces, fully bound associated data, a 64-delivery sliding replay window, and Ed25519
pairing transcripts — with zero RNG in the library (deterministic
seeds) and tamper/replay/substitution all proven as typed failures.
`altior-relay` (ADR 0012) is the content-agnostic queue machine with
zero runtime dependencies: sealed-sender push, cursored repeatable
fetch, idempotent push, byte/depth quotas as visible backpressure,
strict-boundary logical-tick retention, future-cursor rejection, and compaction that preserves fetch
equivalence past the checkpoint while a behind-cursor receiver gets
an explicit Compacted page (the resync trigger). The integration
test runs the whole spike stack: an encrypted envelope crosses the
relay unread, re-delivery is absorbed by the replay window, and
offline receivers past retention are told to resync.

Evidence:

- storage: 13 tests including trigger-enforced immutability
  through a raw second connection, duplicate-append idempotence,
  rebuild-equals-incremental-fold, and reopen self-healing for
  missing/stale markers and stale fold-version recovery
  (`crates/altior-storage/tests/journal.rs`)
- CRDT: 10 adversarial/schema/import scenarios plus the deterministic
  metrics test, both engines passing every invariant
  (`crates/altior-crdt/tests/adversarial.rs`, `bakeoff_metrics.rs`)
- crypto: 16 envelope/replay tests plus 5 pairing-transcript tests —
  round trips, tamper rejection, direction binding, replay windows
  at and past the 64-delivery edge, substitution and rename
  rejection (`crates/altior-crypto/tests/`)
- relay: 12 queue-semantics tests plus 2 end-to-end two-device
  composition with `altior-crypto`
  (`crates/altior-relay/tests/relay.rs`, `two_device_flow.rs`)
- boundaries: 9/9 dependency-manifest assertions including the four
  spike crates (`crates/altior-core/tests/dependency_boundaries.rs`)

Explicitly deferred to P1/P3: crash-safe send-counter persistence,
X25519 contributory/all-zero-shared-secret checks, a forward-secret
ratchet, authenticated durable acknowledgements, and per-device cursors
for multi-device fan-out. The P0.5 in-memory sessions and relay are
reference machines/test doubles, not deployable sync infrastructure.

Exit: ADRs select IPC, process model, storage, CRDT, crypto, and UI dependencies;
ACP and Desktop contracts have executable fixtures.

## P1: ACP desktop continuity

### P1.1 Domain and persistence

- implement AgentProfile, ACP HarnessBinding, Thread, Turn, Event, Permission,
  Project reference, and operation IDs
- add append-safe local turn/event persistence and SQLite projections
- add migrations, recovery markers, and projection rebuild
- implement bounded thread list, history, and search queries

Status: implemented in full (ADR 0013). `altior-domain` defines the 8 pure
domain entity types (`AgentProfile`, `AcpHarnessBinding`, `Thread` [Open/Pinned/Archived],
`Turn` [Active/Completed/Cancelled/Failed with `DeliveryState`], `Permission`
[Pending/Approved/Denied], `ProjectRef`, `DomainEvent`, `EventPayload`), kind-prefixed
identifier newtypes (`AgentProfileId`, `HarnessBindingId`, `ThreadId`, `TurnId`,
`OperationId`, `EventId`, `CoreInstanceId`, `ProjectId`), validated bounded value
objects (`DisplayName`, `ThreadTitle` [including empty `UNTITLED`], `SearchQuery`,
`BoundedLabel`, `BoundedPath`, `PermissionDescription`), validated unsigned query
limits (`ThreadListLimit`, `HistoryLimit`, `TurnListLimit`, `AgentProfileListLimit`,
`HarnessBindingListLimit`, `ProjectRefListLimit`, `PermissionListLimit`), and stable
composite cursors (`ThreadCursor`, `AgentProfileCursor`, `HarnessBindingCursor`,
`ProjectRefCursor`, `PermissionCursor`, `TurnCursor`). `altior-storage` implements
the persistence seam with forward-only v1→v2→v3 migrations: `domain_journal` is the
sole durable authority for domain events and rebuildable projections (decoupled from
the IPC protocol replay log), guarded by append-only database triggers and a 1 MiB
payload cap. Domain event append enforces 7-field durable tuple collision checks
`(event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at)` with
byte-identical idempotency and `operation_id` preservation during replay. In-transaction
domain validation enforces thread and turn state machines, turn exclusivity, pending
permission decisions, and custom `Other` kind scoping (global without thread/turn,
or thread-scoped). Device-local metadata (`AgentProfile`, `AcpHarnessBinding`, `ProjectRef`)
is managed via atomic `IMMEDIATE` CRUD transactions with same-content idempotency,
typed conflict errors, immutable `created_at` protections, and `ProjectRef` deletion
rejection if referenced by threads in projection, while domain journal replay is
fully decoupled from local profile/project pre-existence. Full-text search over thread
titles uses SQLite FTS5 with literal-phrase query escaping (`fts5_quoted_literal`),
title-clearing support, and official consistency validation
(`INSERT INTO thread_search(thread_search, rank) VALUES('integrity-check', 1)`).
Projection self-healing is backed by a deterministic length-prefixed, type-tagged
FNV-1a 64-bit digest over business projections (`thread`, `turn`, `permission` —
strictly excluding FTS shadow tables), preflight FTS rebuild before table clear to
prevent trigger failures, and automatic single-transaction replay and verification
on reopen. P1.1 delivers a complete, usable persistence slice feeding P1.2.

Evidence:

- 67 domain integration tests (`crates/altior-storage/tests/domain.rs`) covering
  schema v1→v2→v3 migration and journal preservation, domain event append and
  idempotency, 7-field durable tuple collisions, lifecycle state transitions and
  terminal turn validation, permission request/decision flow, thread title update
  and clear across rebuild and reopen, `Other` kind global and thread scoping,
  device-local CRUD operations with immutable field and FK/reference protections,
  safe `ProjectRef` deletion rejection, bounded cursor-based pagination across
  threads, turns, history, permissions, and CRUD entities, literal FTS5 search
  safety with special characters, official FTS integrity check rank=1 validation,
  and self-healing on reopen against missing/stale markers, stale fold versions,
  and corrupted projection digests.
- 13 protocol journal tests (`crates/altior-storage/tests/journal.rs`) validating
  v1 protocol envelope round-trips, append-only trigger enforcement via raw
  connections, duplicate append idempotency, envelope collision detection,
  rebuild equivalence, and reopen self-healing.
- Cargo default gate and `--all-features` pass cleanly with zero warnings.
- Desktop 39 test suite and strict TypeScript check pass cleanly.

### P1.2 ACP runtime

- implement launch configuration and OS-secret references
- create/resume sessions, prompt, stream, steer/cancel where supported, and close
- persist delivery checkpoints before/after adapter boundaries
- supervise subprocess trees and recover or surface interrupted turns
- keep raw ACP diagnostics bounded and device-local

Status: implemented in full (ADR 0014). `altior-acp` implements real subprocess
execution (`AcpChild`, `ProcessTransport`), typed launch configuration
(`LaunchConfig`, `ResolvedLaunchConfig`), opaque secret references (`SecretRef`,
`SecretResolver`), stream line I/O framing with 1 MiB line limits, bounded
stderr capture (64 KiB), and RAII child termination (`KillOnDrop`). `altior-core`
introduces Core-owned runtime ports (`HarnessRuntimePort`, `RuntimeCheckpointPort`,
`AgentRuntime`), the thread supervisor state machine (`ThreadRuntimeSupervisor`,
single active turn per thread, `OperationRegistry` dedup, capability gates, UI
detachment resilience), and `AcpHarnessAdapter` managing dedicated worker threads
with 1024-bounded channels, cancel ack flow control backpressure, and permission
routing. `altior-storage` introduces forward migration v3→v4 with the
`runtime_checkpoint` and `thread_session_binding` tables. Pre-call intents and
post-call settlements (`Confirmed`, `Rejected`, `Indeterminate`) provide crash
safety, with automatic reopen recovery converting unsettled intents to
`Indeterminate`. Automatic turn re-send on crashes or indeterminate outcomes is
strictly forbidden. A redact-by-default secret boundary and `NoSecretsResolver`
seam ensure credentials never leak into logs, diagnostics, or SQLite storage.
Deferred to P1.3/P5: OS Credential Manager integration (DPAPI/Keychain), Windows
Job Object process trees, and formal OS IPC server transport into Desktop. P1.3
Desktop MVP is the next milestone.

Evidence:

- 12 runtime supervisor unit tests (`crates/altior-core/tests/p12_runtime.rs`)
  covering session creation, probing, and resumption, streaming event flows,
  permission pause/decision routing, cancellation races and idempotence,
  single-active-turn limits and multi-thread isolation, transport failure and
  unexpected exit handling with `Indeterminate` marking and resend prohibition,
  diagnostics redaction, and UI detachment resilience.
- 6 end-to-end real subprocess composition tests (`crates/altior-core/tests/p12_composition.rs`)
  verifying live streaming with `mock_acp_agent`, abnormal exit code 42 handling,
  permission request/approval flow, turn cancellation, secret canary isolation in
  SQLite files, and multi-run RAII executable cleanup.
- 6 storage checkpoint integration tests (`crates/altior-storage/tests/checkpoint.rs`)
  covering schema v4 migration, intent recording and round-trips, settlement
  state transitions and conflicts, reopen recovery of unsettled intents to
  `Indeterminate`, and session binding CRUD.
- 6 process adapter tests (`crates/altior-acp/tests/process_adapter.rs`)
  verifying real child process spawning, pipe I/O, stderr capture, and RAII termination.
- 160+ total workspace Rust tests pass cleanly across all crates.
- Cargo default gate and `--all-features` pass cleanly with zero warnings.

### P1.3 Desktop MVP

- onboarding: start Core and configure/test one ACP agent
- workbench shell: activity rail, thread list, timeline, composer, inspector
- thread create/open/search/pin/archive
- normalized message, tool, plan, permission, terminal-like output, error, and
  cancellation renderers
- Agents and Settings surfaces needed to manage ACP
- minimal Projects surface for associating an approved local path

Status: implemented in full (ADR 0015). `altior-protocol` expands and exports
the complete P1.3 protocol contracts: 12 command kinds (`create_thread`,
`open_thread`, `list_threads`, `search_threads`, `get_history`, `start_turn`,
`cancel_turn`, `respond_permission`, `configure_agent`, `test_harness_binding`,
`runtime_status`, `diagnostics`), 12 versioned DTOs (`ThreadDto`, `TurnDto`,
`PermissionDto`, `AgentProfileDto`, `HarnessBindingDto`, `ThreadCursorDto`,
`TurnCursorDto`, `ThreadSummaryDto`, `ThreadSnapshotDto`, `ThreadListResponseDto`,
`ThreadHistoryResponseDto`, `RuntimeDiagnosticsDto`), snapshots, and stream events
(`TurnStarted`, `MessageDelta`, `ToolCallStarted`, `ToolCallFinished`,
`PermissionRequested`, `PermissionDecided`, `TurnCompleted`, `TurnCancelled`,
`TurnFailed`, `StreamReplayed`, `StreamReady`, `StreamGap`, `RawUnknown`).
`altior-ipc` delivers the physical OS-level transport: Windows Named Pipes
(`\\.\pipe\altior-ipc-...`, max 256 bytes) and Unix Domain Sockets (`.sock`, max
104 bytes), atomic discovery file (`discovery.json`) publishing with Unix 0600
permissions / Windows per-user ACLs and stale discovery detection, 32-byte hex
`LaunchToken` handshake authentication, 256 KiB hard frame bounds (`MAX_FRAME_BYTES`)
with zero unbounded reads, slow handshake timeout (5s), concurrent session cap
(32 sessions), and redact-by-default `Debug` formatters guaranteeing zero plaintext
token/secret leakage. `altior-core` delivers the `CoreApplication` coordinator
(dispatching domain CRUD, FTS5 title search, and turn supervision), the `Daemon`
server loop (handling OS IPC connections, auth verification, and command routing),
and the `EventPump` (broadcasting live turn streams to all connected client sessions
with monotonic sequence numbers, retained replay windows, and complete UI disconnect
resilience). `apps/desktop/src-tauri` implements `altior_desktop_shell` with
`SpawnOrAttach` logic discovering or spawning the Core daemon, bridging Tauri IPC
commands, and ensuring child survival across UI drops. `apps/desktop` defaults to
`TauriCoreTransport` (falling back to `InMemoryTransport` in dev/browser), and
`applicationStore` coordinates real agent profiles, threads (list/create/search/open),
active turns, streaming message deltas, inline permission prompts/decisions,
turn cancellation, and diagnostics. P1.3 delivers a fully integrated Desktop MVP
feeding P1.4.

Evidence:

- 21 application integration tests (`crates/altior-core/tests/p13_application.rs`)
  covering command dispatching, thread CRUD, FTS5 search, start turn, turn cancellation,
  permission approval/denial routing, duplicate operation prevention, UI disconnect
  resilience, and diagnostics.
- 1 local IPC end-to-end test (`crates/altior-core/tests/p13_local_ipc.rs`)
  verifying end-to-end local IPC server bind, discovery publication, client connection,
  handshake authentication, command dispatch, and event streaming.
- 1 daemon process lifecycle test (`crates/altior-core/tests/p13_daemon_process.rs`)
  verifying real OS daemon subprocess execution, discovery file lifecycle, sequential
  client runs, and clean shutdown.
- 9 IPC transport tests (`crates/altior-ipc/tests/transport.rs`)
  verifying Windows named pipe / Unix domain socket bind/connect, oversized frame rejection,
  path length limits (104/105 bytes), bad token rejection, discovery file lifecycle,
  and zero-plaintext Debug redaction.
- 3 DTO export tests (`crates/altior-protocol/tests/dto_export.rs`)
  verifying deterministic TypeScript DTO export, idempotency, and boolean type mappings.
- 7 Tauri backend tests (`apps/desktop/src-tauri`)
  verifying `SpawnOrAttach`, child process survival across UI state drops, reconnect cursor
  continuity, command/event mapping with deduplication, stale discovery cleanup, and bad
  auth rejection.
- 69 Desktop Vitest tests across 9 test files (`apps/desktop`)
  verifying application store state transitions, timeline virtualization, Tauri transport,
  in-memory transport, and UI components.
- Strict TypeScript typecheck (`tsc --noEmit`) and production build (`vite build`)
  pass with zero errors.
- Cargo workspace tests pass cleanly across all crates.

Explicitly deferred to P1.4/P5: real third-party dual-ACP agent installation journeys,
signed OS installers/packaging (MSIX), OS secret manager integration (DPAPI/Keychain),
and Windows Job Object hierarchy. P1.4 Acceptance Journey is the next milestone.

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

Status: **complete (ADR 0016)**. All eight steps run as a single E2E test
against a real daemon process, real named pipe, persistent SQLite, and two
mock ACP agent binaries (`p14_acceptance_journey.rs`, ~1.2 s, no sleeps).
Enablers: harness binding v5 (`args`/`env_keys`/`secret_refs` persisted via
`configure_agent`), default binding auto-selection on `open_thread`, default
discovery path parity with the Tauri shell, and three journey-hardening fixes
(daemon close observability, typed `KnownEvent` mapping for
permission/cancel/fail, and interruptible prompt reads so silent agents can be
cancelled). Deferred to P1.5+/P5: real third-party dual ACP agents on a clean
OS profile, OS secret manager (DPAPI/Keychain), signed installers, Windows Job
Object hierarchy.

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
