# Altior

Altior is a local-first, multi-device personal knowledge runtime for persistent
AI identity, memory, skills, and agent conversations.

It deliberately has no organization, company, team, billing, or traditional
product-login model. A person owns an encrypted Personal Vault and authorizes
their own devices through cryptographic pairing.

## Status

Architecture and executable-contract phase. The Rust workspace contains the
complete P0.1 contract skeleton: stable domain identifiers, clock and
delivery-state conventions, the Desktop/Core handshake and version
negotiation, versioned command/event/snapshot envelopes with bounded
payloads and unknown-event preservation, and deterministic protocol
fixtures (ADR 0004). `apps/desktop` runs the fixture shell on an in-memory
transport with TypeScript DTOs generated from the Rust contracts (ADR 0005).
P0.2 adds Core supervision and local IPC (ADR 0006): the `altior-ipc`
crate with bounded length-prefixed frames, per-launch capability tokens,
pure session machines (shared per-launch event log, sequence/catch-up with
retained-window replay or explicit gaps, epoch change on Core restart), and
`altior-core` with the spawn-or-attach supervisor, turn ownership that
survives UI reload, and two-sided duplicate-command prevention. P0.3 adds
the narrow ACP v1 adapter (`altior-acp`, ADR 0007): capability-only
negotiation, unknown-event-preserving mapping onto the protocol contracts,
prompt delivery classified onto the frozen `DeliveryState` vocabulary, a
pure process lifecycle machine, and an opt-in real-agent smoke harness
behind `ALTIOR_ACP_SMOKE_AGENTS`. P0.4 adds the Desktop workbench spike
(ADR 0008): the five-region shell with a zero-dependency virtualized
timeline (100,000 rows, prepend-stable history, per-row streaming
updates, scroll restoration), keyboard navigation with inline approval,
and a Tauri v2 shell outside the Rust workspace with pinned capability
minimums. The OS transport (tokio named pipes /
Unix domain sockets) is a named later slice. P0.5 hardens four runnable
foundation spikes (ADR 0009-0012): the SQLite journal-to-projection
store with forward-only migrations and self-healing rebuilds, the
Loro/Automerge `SyncDocumentEngine` bake-off (Automerge selected,
typed schema and framed/bounded imports, both engines raced in CI), the standard-primitives two-device
crypto envelope (direction-separated X25519/HKDF keys +
ChaCha20-Poly1305 with replay windows and signed pairing transcripts),
and the zero-dependency content-agnostic relay with validated cursored
fetch and checkpoint compaction. These remain pre-production reference
implementations: counter persistence, contributory checks, ratcheting,
durable relay storage/acks, and per-device cursors are deferred to P1/P3.
P1.1 adds the domain and persistence layer (ADR 0013): `altior-domain`
defines the 8 core domain entity classes (`AgentProfile`, `AcpHarnessBinding`,
`Thread` [Open/Pinned/Archived], `Turn` [Active/Completed/Cancelled/Failed with
`DeliveryState`], `Permission` [Pending/Approved/Denied], `ProjectRef`,
`DomainEvent`, and `DomainEventKind`), kind-prefixed identifier newtypes
(`AgentProfileId`, `HarnessBindingId`, `ThreadId`, `TurnId`, `OperationId`,
`EventId`, `CoreInstanceId`, `ProjectId`), bounded value objects (`DisplayName`,
`ThreadTitle`, `SearchQuery`, `BoundedLabel`, `BoundedPath`, `PermissionDescription`,
`EventPayload`), validated query limits (`ThreadListLimit`, `HistoryLimit`,
`TurnListLimit`, `AgentProfileListLimit`, `HarnessBindingListLimit`,
`ProjectRefListLimit`, `PermissionListLimit`), and stable composite cursors for
deterministic pagination (`ThreadCursor`, `AgentProfileCursor`,
`HarnessBindingCursor`, `ProjectRefCursor`, `PermissionCursor`, `TurnCursor`).
`altior-storage` evolves the persistence seam with forward-only
v1->v2->v3 migrations: dual journal authority (`domain_journal` is the sole
durable authority for domain entities and rebuildable projections, decoupled
from the transport-level protocol replay log), append-only SQLite triggers,
complete 7-field durable tuple collision fail-closed semantics across
`(event_id, thread_id, turn_id, operation_id, kind, payload, occurred_at)`,
`operation_id` reconstruction, in-transaction typed payload and domain lifecycle
validation (with `Other` custom kind global vs thread-scoped semantics),
device-local atomic `IMMEDIATE` CRUD authority for agent profiles, harness
bindings (with agent FK validation), and project references (with safe `IMMEDIATE`
deletion refusing referenced projects, immutable field protections, and journal
replay decoupled from local profile/project pre-existence), bounded turn and
permission queries with compound indexes, safe literal FTS5 title search with
title clearing and official `rank='integrity-check', 1` consistency checks, and a
deterministic business projection digest (excluding FTS private shadow tables)
that self-heals via single-transaction rebuild on reopen. P1.2 adds the ACP
subprocess runtime, boundary checkpoints, and process supervision (ADR 0014):
`altior-acp` implements real subprocess execution (`AcpChild`, `ProcessTransport`),
stdio JSON-RPC framing (1 MiB line cap), bounded stderr capture (64 KiB), and
RAII child process termination (`KillOnDrop`). `altior-core` introduces Core-owned
runtime ports (`HarnessRuntimePort`, `RuntimeCheckpointPort`, `AgentRuntime`),
the per-thread runtime supervisor (`ThreadRuntimeSupervisor`, single active turn
per thread, capability gates, UI detachment resilience), and `AcpHarnessAdapter`
managing dedicated worker threads with 1024-bounded channels, cancel ack flow
control backpressure, and permission routing. `altior-storage` introduces
forward migration v3→v4 with `runtime_checkpoint` and `thread_session_binding`
tables. Pre-call intents and post-call settlements (`Confirmed`, `Rejected`,
`Indeterminate`) provide crash safety, with automatic reopen recovery converting
unsettled intents to `Indeterminate`. Automatic turn re-send on crashes or
indeterminate outcomes is strictly forbidden. A redact-by-default secret
boundary and `NoSecretsResolver` seam ensure credentials never leak into logs,
diagnostics, or SQLite files. This release represents the **0.0.1 developer preview**:
a complete, verified backend runtime foundation. It is an architecture and developer
preview, not yet a packaged consumer product; P1.3 Desktop MVP (Tauri workbench
wiring, live IPC server, and onboarding) is the next milestone.

## Design anchors

- Rust core daemon with a Tauri/React desktop client
- local SQLite projections and search indexes
- signed event synchronization plus CRDT documents where concurrent editing is needed
- end-to-end encrypted relay, optional self-hosting, and offline-first behavior
- replaceable agent harnesses: ACP, terminal, and a future Altior-native harness
- personality and memory above every agent harness

Start with [Product](docs/PRODUCT.md), [Architecture](docs/ARCHITECTURE.md), and
[Contributor Rules](AGENTS.md). Implementation is ACP-first; see the
[Implementation Plan](docs/IMPLEMENTATION_PLAN.md),
[Desktop UI Architecture](docs/UI_ARCHITECTURE.md), and
[AI Development Discipline](docs/AI_DEVELOPMENT.md).
