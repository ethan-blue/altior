# Changelog

All notable changes to the Altior project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.1] - 2026-08-31

### Added

#### P1.2 ACP Subprocess Runtime, Boundary Checkpoints & Supervision (ADR 0014)
- **Real OS child process execution (`altior-acp`)**: Spawns and manages real agent subprocesses using `std::process::Command` with explicit `program` (`BoundedPath`) and `args` arrays (no shell interpolation). Communicates over piped standard streams via newline-delimited JSON-RPC 2.0 with a 1 MiB line length cap.
- **Bounded stderr diagnostics**: Background reader thread asynchronously captures child stderr up to a 64 KiB ring buffer (`MAX_STDERR_CAPTURE_BYTES`) with clear truncation markers.
- **RAII process reap (`KillOnDrop`)**: `AcpChild` implements `Drop` to terminate child processes and wait on their exit status, preventing zombie subprocess leaks on error or exit.
- **Worker threads & flow control backpressure**: Dedicated background worker threads per session communicating over 1024-bounded channels (`CHANNEL_BOUND`). Flow control backpressure via acknowledgement channel (`ack_tx` / `ack_rx`) throttles rapid stdout streaming.
- **Cancel signal & permission routing**: Atomic cancel flags interrupt active prompt loops; thread-safe permission channels route user decisions to pending agent requests.
- **Core runtime ports (`altior-core`)**: Core-owned trait definitions (`HarnessRuntimePort`, `RuntimeCheckpointPort`, `AgentRuntime`) decoupling infrastructure from domain coordination.
- **Thread supervisor state machine**: `ThreadRuntimeSupervisor` coordinates thread lifecycle (`Idle` → `Starting` → `Ready` → `Prompting` → `AwaitingPermission` → `Cancelling` → `Closed` / `Crashed`), enforcing at most 1 active turn per thread, `OperationRegistry` deduplication, and desktop UI detachment resilience.
- **Schema v4 migrations & storage checkpoints (`altior-storage`)**: `SCHEMA_V4` adds `runtime_checkpoint` and `thread_session_binding` tables with composite indexing. Excluded from domain projection digests.
- **Two-phase intent-to-settlement lifecycle**: Pre-call intents (`CheckpointIntent`) are persisted before external boundary calls; post-call outcomes settle to terminal states (`Confirmed`, `Rejected`, `Indeterminate`). Settlement is strictly one-way.
- **Startup recovery of unsettled intents**: `Store::open` atomically transitions lingering `state = 'intent'` rows to `state = 'indeterminate'`, preventing zombie pending operations across restarts.
- **Strict no-auto-resend rule**: Turns settling to `Indeterminate` or terminal states strictly forbid automatic duplicate re-submission; explicit user action with fresh turn/operation IDs is required.
- **Secret reference boundary & `NoSecretsResolver` seam**: `SecretRef` models opaque credential handles; resolved environment variables are redacted across all `Debug`, `Display`, and log formatting. `NoSecretsResolver` provides a safe fail-closed default seam. Automated tests verify zero secret canary leakage into SQLite files or diagnostics.
- **Real process end-to-end integration tests**: Test suite utilizing the `mock_acp_agent` binary verifying happy-path streaming, abnormal exit code 42 crash handling, permission pauses, cancellation, and secret canary containment.

#### P1.1 Domain Entities & Persistence (ADR 0013)
- **Pure domain entities (`altior-domain`)**: 8 core entity classes (`AgentProfile`, `AcpHarnessBinding`, `Thread`, `Turn`, `Permission`, `ProjectRef`, `DomainEvent`, `EventPayload`), kind-prefixed identifier newtypes (`AgentProfileId`, `HarnessBindingId`, `ThreadId`, `TurnId`, `OperationId`, `EventId`, `CoreInstanceId`, `ProjectId`), bounded value objects (`DisplayName`, `ThreadTitle`, `SearchQuery`, `BoundedLabel`, `BoundedPath`, `PermissionDescription`), validated pagination limits, and stable composite cursors.
- **Dual journal persistence (`altior-storage`)**: `domain_journal` durable event store with forward migrations v1→v2→v3, append-only SQLite triggers, 7-field durable tuple collision checks, in-transaction lifecycle validation, and decoupled replay authority.
- **Projections & FTS5 search**: Rebuildable SQLite projections for threads, turns, and permissions; literal FTS5 title search with official `rank='integrity-check', 1` validation; self-healing on reopen via deterministic FNV-1a business projection digests.
- **Device-local CRUD**: Atomic `IMMEDIATE` transactions for agent profiles, harness bindings, and project references with FK and deletion protections.

#### P0.5 Storage, CRDT, Crypto & Relay Foundation Spikes (ADR 0009–0012)
- **Storage spike (ADR 0009)**: Protocol journal persistence with SQLite triggers and projection fold versions.
- **CRDT sync engine (ADR 0010)**: Evaluated Loro vs Automerge; selected Automerge for typed schema and bounded framed imports.
- **Two-device crypto envelope (ADR 0011)**: Direction-separated X25519/HKDF keys, ChaCha20-Poly1305 encryption, replay windows, and signed pairing transcripts.
- **Content-agnostic relay (ADR 0012)**: Zero-dependency relay queue with validated cursored fetch and checkpoint compaction.

#### P0.1–P0.4 Core Contracts, Desktop Spike & Narrow ACP Adapter (ADR 0004–0008)
- **Protocol contracts (ADR 0004)**: Stable domain IDs, clock/delivery vocabulary, handshake/negotiation envelopes, unknown-event preservation.
- **Desktop contracts (ADR 0005)**: Generated TypeScript DTOs from Rust contracts and in-memory transport shell.
- **Core supervision & IPC (ADR 0006)**: Length-prefixed framing, capability tokens, pure session state machines, spawn-or-attach supervisor.
- **ACP v1 adapter mapping (ADR 0007)**: Capability-only negotiation, unknown-event-preserving message mapping, pure lifecycle machine.
- **Desktop UI workbench spike (ADR 0008)**: 5-region shell, zero-dependency virtualized timeline (100k rows, prepend-stable, streaming updates, scroll restoration), keyboard navigation, Tauri v2 capability configuration.

### Known Limitations

This release represents a **developer preview (v0.0.1)** of the core runtime foundation. The following capabilities are planned for upcoming milestones:
- **Desktop UI Integration (P1.3)**: The Tauri/React desktop workbench currently operates against an in-memory transport shell and is not yet connected to the live Core daemon via OS IPC.
- **OS Credential Manager (P1.3/P5)**: Launch configuration secret references (`SecretRef`) use the `NoSecretsResolver` seam; integration with Windows Credential Manager / macOS Keychain is deferred.
- **Windows Job Object Hierarchy (P1.3/P5)**: Child process containment relies on standard library RAII process handles (`AcpChild` `Drop`) rather than Windows Job Objects (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
- **OS Local IPC Server (P1.3)**: Named Pipe / Unix Domain Socket server transport wiring for Core is deferred to the P1.3 Desktop MVP milestone.
- **Personal Identity & Memory (P2)**: Persistent memory extraction, context snapshot assembly, and FTS retrieval are not yet implemented.
- **Personal Vault Sync (P3)**: Multi-device encrypted sync and untrusted relay networking remain spike prototypes.
