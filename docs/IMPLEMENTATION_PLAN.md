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
dependency-boundary test. Sequence/catch-up delivery semantics and the
production IPC transport remain P0.2 work.

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

Evidence:

- UI reload does not stop a synthetic active turn
- Core restart exposes recovery state without duplicate commands
- wrong protocol version and stale socket/pipe fail clearly

### P0.3 ACP v1 spike

- implement a narrow ACP adapter outside the domain crates
- connect to two real external agents in opt-in smoke tests
- negotiate capabilities rather than inspecting version strings
- map create, prompt, delta, tool, permission, cancel, resume, and failure events
- classify delivery as absent, confirmed, rejected, or indeterminate

Evidence:

- normalized trace fixtures for two agents
- unknown event and malformed frame tests
- process crash, idle timeout, cancellation, and cleanup tests
- real-agent smoke report kept free of user transcript and secrets

### P0.4 Desktop interaction spike

- implement the classic workbench shell and design tokens
- prove virtualized streaming, prepend-stable history, scroll restoration, and
  inspector resizing
- prove keyboard navigation and an inline approval flow
- establish visual baselines on the pinned Windows environment

Evidence:

- 100,000 synthetic rows remain interactive
- a streamed long response does not rerender the complete timeline
- light/dark/narrow/error/approval screenshots reviewed
- Tauri main window has only the declared minimum capabilities

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
