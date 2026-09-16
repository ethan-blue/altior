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
diagnostics, or SQLite files. P1.3 delivers the Desktop MVP and local IPC
layer (ADR 0015): `altior-protocol` expands and exports 12 commands, 12 DTOs,
snapshots, and stream events to TypeScript; `altior-ipc` delivers physical OS
IPC over Windows Named Pipes and Unix Domain Sockets with atomic discovery file
publishing (Unix 0600 / Windows per-user ACLs), 32-byte hex launch token auth,
256 KiB frame bounds, slow handshake timeouts (5s), 32 session caps, and
Debug redaction; `altior-core` introduces `CoreApplication` (command dispatch,
FTS5 search, turn supervision), the `Daemon` server loop, and the `EventPump`
broadcasting live events with monotonic sequence numbers and retained replay
windows (background turns survive UI disconnects); `apps/desktop/src-tauri`
implements `SpawnOrAttach` discovering or launching the Core daemon; and
`apps/desktop` runs the workbench with `TauriCoreTransport` and `applicationStore`
managing real agents, threads, streaming turns, inline permission decisions,
turn cancellation, and diagnostics. P1.4 delivers the full 8-step Acceptance Journey and Harness Binding v5 (ADR 0016):
a single end-to-end integration test (`p14_acceptance_journey.rs`) validates the entire
vertical stack against a live daemon process, physical named pipes / domain sockets,
persistent SQLite, and mock ACP agents. It exercises agent configuration, thread
creation, streaming prompt delivery, inline permission approval, cooperative turn
cancellation, background turn continuity during UI detachment, abnormal exit with
`Indeterminate` settlement, crash recovery forbidding automatic resend, and offline
FTS5 search.

P2 delivers Personal Identity and Memory: P2.1 (ADR 0017) introduces the long-term
memory lifecycle (`memory.proposed`, `confirmed`, `rejected`, `superseded`, `forgotten`,
`expired`), schema v6 persistence, fail-closed secret-shaped content rejection, and
ranked FTS5 retrieval. P2.2 (ADR 0018) introduces identity documents, deterministic
context snapshot assembly with token budgeting, and explainable context injection.
Subsequent hardening slices establish:
- ADR 0019: Deterministic Core entity ID generation and Desktop operation ID CSPRNG boundaries.
- ADR 0020: Bounded timeline history pagination and journal-backed historical records.
- ADR 0021: Epoch-scoped stream event deduplication and bounded replay ringbuffers.
- ADR 0022: Multi-scope context trust boundaries (Global/Personal/Project/Thread) and token estimator versioning.
- ADR 0023: SQLite FTS5 trigram hybrid retrieval and bounded O(K) heap candidate ranking for robust CJK/code search (Schema V8).
- ADR 0024: Projection digest streaming bypass for MessageDelta with authoritative checkpointing upon turn settlement and automatic crash healing.

## Component Acceptance Matrix

| Component | Stack | Status | Verification & Gates |
|---|---|---|---|
| **Core Daemon & Domain** | Rust 2024 (`crates/altior-*`) | **Verified (Automated)** | `cargo test --workspace`, clippy `-D warnings`, fmt check passing 100% |
| **Desktop Renderer** | Vite, React 19, TS (`apps/desktop`) | **Verified (Automated)** | `npm run gate`: typecheck, lint, format, 195 unit tests, WCAG contrast audit, visual regression |
| **Packaged Shell** | Tauri v2, Rust 2021 (`src-tauri`) | **Verified (Dev / Test)** | `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml` (7 tests passed); `bundle.active=false` |
| **Mock ACP Acceptance** | Pure Rust mock agents | **Verified (Automated)** | `p14_acceptance_journey.rs` passes 8/8 lifecycle steps |
| **Real Third-Party ACP** | Live external agent CLIs | **Opt-in / Manual** | Requires local credentials; smoke gate reports `[SKIPPED]` when unconfigured |
| **Multi-Device Sync** | Crypto / Relay / CRDT | **Spike (Pre-Production)** | Reference implementation; production sync remains disabled per ADR 0011/0012 |

## Architecture & Rust Edition Alignment

- **Workspace Crates**: Use **Rust Edition 2024** (`rust-version = "1.90"`). This applies to all domain, protocol, storage, runtime, acp, and ipc crates.
- **Tauri Shell Exception**: `apps/desktop/src-tauri` uses **Rust Edition 2021** and is kept deliberately outside the root Cargo workspace (ADR 0008 §6). This keeps core repository gates fast and hermetic while decoupling the packaged UI shell from the backend runtime. It is independently tested via `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`.

## Supported Platforms & Build Prerequisites

- **Supported Operating Systems**:
  - Windows 10 / 11 (x64) with Microsoft Edge WebView2 (built-in)
  - macOS 13+ (Apple Silicon and Intel x86_64)
  - Linux x86_64 (glibc 2.35+; requires WebKitGTK 4.1)
- **Toolchain Prerequisites**:
  - Rust 1.90+ (`cargo`, `rustc`)
  - Node.js v20+ / npm v10+
  - Playwright Chromium (installed via `npx playwright install chromium`)

## Runtime Discovery & Local Storage Architecture

- **Core Discovery**: On launch, `altior-core` writes an atomic discovery file containing its IPC endpoint and a 32-byte hexadecimal launch token.
  - Windows: `%LOCALAPPDATA%\Altior\ipc\core_discovery.json` (secured via per-user ACLs)
  - Unix: `~/.local/share/altior/ipc/core_discovery.json` (secured with `0600` file permissions)
- **Database & Vault**: SQLite database storing the append-only `domain_journal` and derived projections.
  - Windows: `%LOCALAPPDATA%\Altior\data\altior.db`
  - Unix: `~/.local/share/altior/data/altior.db`
- **Diagnostics & Redaction**: Diagnostic logs are written with automatic redaction of API tokens, bearer keys, and sensitive environment variables before output.

## Installation, Upgrade & Data Lifecycle Policy

- **Clean Installation**: Run `npm --prefix apps/desktop run build` followed by `cargo build -p altior-core`. For packaged desktop testing, run `cargo tauri build` within `apps/desktop`.
- **Schema Migration**: Migrations are strictly forward-only (`SCHEMA_V1` through `SCHEMA_V8`). Every migration preserves historical journal records and automatically upgrades projection tables and FTS indexes.
- **Downgrade Invariant**: An older binary running against a newer database schema will fail cleanly with an explicit `UnsupportedNewerSchema` error; silent data downgrade or table deletion is strictly forbidden.
- **Crash Recovery**: If the daemon terminates abruptly mid-turn, `Store::open` automatically detects the unfinalized projection state, replays the authoritative `domain_journal`, heals all projection tables, and restores digest consistency without data loss.
- **Uninstallation**: Removing the application binary leaves the SQLite Personal Vault intact by default so user memories and history are preserved across reinstalls. To completely remove all data, delete the `Altior` data directory.

## Quality Gates & Verification Runbook

Run the complete, unified quality gate with a single command:
```bash
# On Windows (PowerShell):
powershell -ExecutionPolicy Bypass -File scripts/quality-gate.ps1

# On macOS / Linux:
./scripts/quality-gate.sh
```

The gate automatically enforces:
1. **Rust Workspace**: `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --workspace`.
2. **Tauri Shell**: `cargo clippy` and `cargo test` on `apps/desktop/src-tauri/Cargo.toml`.
3. **Desktop Frontend**: TypeScript typecheck, architectural linter, format check, Vitest unit suite, WCAG 2.2 color contrast audit, Playwright geometry and visual regression checks, and production bundle build.
4. **Real ACP Opt-in**: Verifies whether `ALTIOR_ACP_SMOKE_AGENTS` is configured, executing live model tests if present or cleanly reporting `[SKIPPED]` without faking pass status.

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
