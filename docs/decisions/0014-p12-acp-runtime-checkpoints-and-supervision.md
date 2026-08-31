# ADR 0014: P1.2 ACP Subprocess Runtime, Boundary Checkpoints, and Process Supervision

Date: 2026-08-31 · Status: accepted · Scope: P1.2 ACP runtime (`docs/IMPLEMENTATION_PLAN.md`), feeding P1.3 Desktop MVP.

## Context

`docs/ARCHITECTURE.md` and `docs/decisions/0007-acp-v1-adapter-scope-and-mapping.md` established the pure mapping, capability negotiation, and delivery classification rules for ACP (Agent Client Protocol). `docs/decisions/0013-p11-domain-entities-and-persistence.md` delivered pure domain entities and durable SQLite journal persistence in `altior-domain` and `altior-storage`.

Prior to P1.2, ACP execution was modeled as pure state machines and in-memory test doubles without real OS subprocess orchestration, boundary crash safety, or persistent adapter intent correlation.

P1.2 delivers the production ACP runtime foundation and supervision layer:
1. Spawning and supervising real OS child processes speaking JSON-RPC 2.0 over standard I/O streams (`stdin`/`stdout`/`stderr`) using strict executable path and argument arrays without shell concatenation.
2. Background worker threads communicating via bounded channels with flow control backpressure and cancel acknowledgement.
3. RAII process termination (`KillOnDrop`) ensuring child processes are reaped on exit or panic without leaving orphan zombie processes.
4. Core runtime port abstractions (`HarnessRuntimePort`, `RuntimeCheckpointPort`, `AgentRuntime`) and thread-level supervisor state machines (`ThreadRuntimeSupervisor`, `AgentRuntimeSupervisor`) coordinating single-active-turn turns, session bindings, permission pauses, and desktop UI detachments.
5. Schema v4 storage migrations adding durable `runtime_checkpoint` and `thread_session_binding` tables.
6. Pre-call `Intent` recording and post-call `Settled` transitions (`Confirmed`, `Rejected`, `Indeterminate`), with automatic startup recovery marking pending intents as `Indeterminate`.
7. Strict prohibition of automatic turn re-send on crashes or transport interruptions.
8. Secret reference abstractions (`SecretRef`) with a redact-by-default boundary and injectable `SecretResolver` seam (`NoSecretsResolver`), ensuring credentials never touch disk, journals, or diagnostics.
9. Real process end-to-end integration tests using an isolated mock agent binary (`mock_acp_agent`).

## Decision

### 1. Real ACP child process transport & streaming

`altior-acp::transport::AcpChild` and `ProcessTransport` manage real OS subprocesses:
- **Executable invocation**: Uses `std::process::Command` with explicit `program` (`BoundedPath`, max 4096 bytes) and `args` (`Vec<String>`, max 256 args, max 64 KiB each). Shell interpolation (e.g. `sh -c` or `cmd.exe /c`) is strictly forbidden.
- **Stream framing**: Standard I/O streams (`stdin`, `stdout`) are piped and framed using newline-delimited JSON-RPC 2.0 with line length caps (`MAX_LINE_BYTES` = 1 MiB).
- **Bounded stderr capture**: `stderr` is captured asynchronously in a dedicated reader thread into a ring buffer capped at `MAX_STDERR_CAPTURE_BYTES` (64 KiB). Truncation appends `\n...[stderr truncated]`.
- **Non-blocking polling**: Subprocess exit status is checked via `try_wait` without blocking the main event loop.

### 2. Worker threads, bounded channels, and cancel ack backpressure

Because newline-delimited JSON-RPC streaming over standard I/O is synchronous and blocking, `altior-core::runtime::adapters::acp::AcpHarnessAdapter` assigns each active session a dedicated background worker thread:
- **Bounded channels**: Worker command and event channels are strictly bounded (`CHANNEL_BOUND` = 1024) to prevent unbounded memory growth under fast agent output.
- **Flow control backpressure**: For streaming events (`Started`, `MessageDelta`, `RawUnknown`), the worker yields and waits on an acknowledgement channel (`ack_tx` / `ack_rx`) before pulling further chunks from the subprocess stdout pipe.
- **Cancel signal**: Cancellation is signaled via an atomic flag (`Arc<AtomicBool>`) and session-level command interruption, allowing prompt loops to abort promptly even when stdout pipes are saturated.
- **Permission routing**: When an agent emits a permission request, a one-shot channel (`std::sync::mpsc::SyncSender<PermissionDecision>`) is registered by `event_id` in a thread-safe registry (`Arc<Mutex<BTreeMap<EventId, ...>>>`), blocking the agent turn until the user issues an approval or denial.

### 3. RAII process cleanup and reap

To prevent orphan background processes on unhandled errors, panics, or application exit:
- `AcpChild` implements `Drop`:
  ```rust
  impl Drop for AcpChild {
      fn drop(&mut self) {
          let _ = self.terminate();
      }
  }
  ```
- `terminate()` closes the stdin pipe, sends `kill()`, waits on the child process handle to reap its OS exit status, and joins the background stderr capture thread.

### 4. Core runtime ports and supervisor architecture

The runtime architecture is decoupled via Core-owned traits in `altior-core::runtime::ports`:
- `HarnessRuntimePort`: Port implemented by harness adapters (e.g. `AcpHarnessAdapter`) for probing bindings, creating/resuming sessions, dispatching prompts, canceling turns, routing permission decisions, and closing sessions.
- `RuntimeCheckpointPort`: Port implemented by persistence adapters (e.g. `StoreCheckpointAdapter`) for persisting pre-call intents, settling post-call outcomes, recording domain events, and persisting session bindings.
- `AgentRuntime`: Primary use-case port consumed by desktop callers to drive turns and poll runtime stream events.
- `ThreadRuntimeSupervisor`: Pure per-thread state machine managing:
  - Strict lifecycle transitions: `Idle` → `Starting` → `Ready` → `Prompting` → `AwaitingPermission` → `Cancelling` → `Closed` / `Crashed`.
  - Single-active-turn constraint: At most 1 turn active per thread at any time.
  - Operation correlation: Correlates `OperationId`, `TurnId`, `ThreadId`, and `EventId`.
  - Desktop detachment resilience: Decoupled turn ownership ensures background turns complete and persist even if the UI client disconnects.
- `AgentRuntimeSupervisor`: Thread-safe coordinator owning the map of thread supervisors and delegating to injected harness and checkpoint ports.

### 5. Schema v4 storage checkpoints and session bindings

`altior-storage` introduces forward migration `SCHEMA_V4` (keyed by `PRAGMA user_version = 4`):
- `runtime_checkpoint`: Device-local boundary log recording operation intents and final settlement states:
  - Columns: `id` (PK, `chk_`), `thread_id`, `turn_id`, `operation_id`, `boundary_kind`, `state`, `remote_request_id`, `diagnostic_summary`, `created_at`, `settled_at`.
  - Indexes: `(thread_id, created_at, id)`, `(state, created_at, id)`, `(operation_id)`.
  - Excluded from domain projection digest and rebuilds.
- `thread_session_binding`: Durable binding between a thread, harness binding, and opaque external session ID:
  - Columns: `thread_id` (PK), `harness_binding_id`, `opaque_session_id`, `updated_at`.

### 6. Intent-to-settlement lifecycle and startup recovery

External harness interactions follow a durable two-phase checkpoint protocol:
1. **Pre-call Intent**: Before initiating a prompt, permission decision, cancellation, or session close, a `CheckpointIntent` is durably written to `runtime_checkpoint` in `state = 'intent'`.
2. **Post-call Settlement**: Upon completion or failure, the checkpoint is settled to a terminal state (`Confirmed`, `Rejected`, or `Indeterminate`). Settlement is strictly one-way (`Intent` → terminal); re-settling to a different terminal state returns `StorageError::CheckpointSettlementConflict`.
3. **Startup recovery**: On `Store::open`, `recover_unsettled_checkpoints()` atomically queries all lingering `state = 'intent'` rows and updates them to `state = 'indeterminate'` with `settled_at = COALESCE(settled_at, created_at)`. No zombie pending intents survive process restarts.

### 7. Strict prohibition of automatic resend

If a turn fails, crashes, or is interrupted (transitioning to `DeliveryState::Indeterminate` or `TurnState::Failed` / `TurnState::Cancelled`):
- The supervisor records the turn execution record and marks the turn terminal.
- Automatic retries or re-sends on the same `TurnId` / `OperationId` are strictly rejected with `RuntimeError::ResendForbidden` or `AdmissionError::TurnAlreadyFinished`.
- Resending requires explicit user initiation with a fresh `TurnId` and fresh `OperationId`.

### 8. Secret redaction boundary & `NoSecretsResolver` seam

To ensure credential safety:
- Launch configurations store secrets as opaque `SecretRef` identifiers (max 256 bytes). Plaintext values are never placed in launch configurations or serialized to disk.
- `SecretResolver` defines the boundary interface for resolving secrets at spawn time:
  ```rust
  pub trait SecretResolver: Send + Sync {
      fn resolve_secret(&self, secret_ref: &SecretRef) -> Result<String, AcpError>;
  }
  ```
- `NoSecretsResolver` serves as the safe default seam when no platform secret vault is wired, failing closed with `AcpError::SecretResolutionFailed`.
- Environment variable maps in `ResolvedLaunchConfig` and `DiagnosticSummary` redact secret values across all `Debug`, `Display`, and JSON formatting.
- Checkpoint diagnostics and domain journal payloads are validated in automated tests to ensure zero credential canary leakage into SQLite files.

### 9. Real process end-to-end testing harness

Testing uses a dedicated `mock_acp_agent` test binary supporting multi-turn dialogues, configurable chunk delays, prompt crash codes (e.g. exit code 42), permission requests (`execute`, `read`, `write`, `network`), cancellation delays, and unmapped JSON-RPC notifications. Real subprocess tests verify full composition across OS process execution, channel backpressure, SQLite persistence, and failure recovery.

## Alternatives considered

- **Async tokio process spawning inside adapter**: Rejected. Standard library synchronous subprocesses combined with dedicated worker threads preserve minimal dependency footprints, avoid mixing async runtimes across workspace crates, and provide deterministic thread lifecycles.
- **Automatic exponential-backoff prompt retries**: Rejected. In agentic workflows, re-executing prompts with indeterminate delivery can cause duplicate side effects (e.g. tool actions, file writes). Turns must fail closed to `Indeterminate` and require human review.
- **Storing secrets in SQLite configuration tables**: Rejected. Violates security baseline; secrets must remain behind opaque references resolved only in memory at launch time.
- **Unbounded mpsc channels for ACP streaming**: Rejected. Fast agent text generation could lead to unbounded memory consumption; 1024-element bounded channels with flow control acks enforce backpressure.

## Failure modes

- **Subprocess crashes mid-turn**: Reader thread receives EOF/error; `AcpChild::try_wait` detects exit code; supervisor transitions state to `Crashed`, settles checkpoint as `Indeterminate`, and records `TurnState::Failed` with `DeliveryState::Indeterminate`.
- **Core process killed while turn active**: On reboot, `Store::open` runs `recover_unsettled_checkpoints()`, transitioning all lingering `Intent` rows to `Indeterminate`.
- **Duplicate prompt execution request**: Disallowed by operation deduplication in `OperationRegistry` and single-active-turn supervisor invariant, returning `AdmissionError::DuplicateOperation` or `TurnAlreadyActive`.
- **Agent flood / runaway stdout streaming**: Bounded line decoder rejects lines >1 MiB (`AcpError::LineTooLarge`); channel backpressure throttles subprocess reader loop.
- **Secret resolver missing or misconfigured**: Launch fails closed with `AcpError::SecretResolutionFailed`; no partial process is spawned.

## Migration

- Existing databases migrate forward via `SCHEMA_V4` in `crates/altior-storage/src/migrations.rs`.
- `domain_journal` and existing v1–v3 tables remain intact and unchanged.
- Projection integrity checks continue to pass cleanly as `runtime_checkpoint` and `thread_session_binding` are device-local tables outside projection digests.

## Exit

- All ACP subprocess transport, line decoding, and lifecycle modules implemented and tested in `altior-acp`.
- Core runtime ports, thread supervisor state machines, and coordinator implemented in `altior-core`.
- SQLite schema v4 runtime checkpoint and session binding persistence implemented in `altior-storage`.
- End-to-end integration tests passing with real subprocesses across happy path, crash mode, permission decisions, cancellation, and secret redaction.

## Revisit triggers & Technical debt

The following items are explicitly deferred to P1.3 Desktop MVP or P5 Release Hardening:
1. **OS Credential Manager integration**: Integration with Windows Credential Manager / DPAPI / macOS Keychain for resolving `SecretRef` values (currently using the `SecretResolver` seam with `NoSecretsResolver`).
2. **Windows Job Object process trees**: Enclosing child processes in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` to guarantee termination of descendant process trees if the agent spawns sub-children (currently guarded by `AcpChild` RAII `Drop`).
3. **Formal OS IPC Server**: Wiring the runtime supervisor into a live Named Pipe / Unix Domain Socket server for Desktop UI client connection (currently tested via direct in-process composition and Core ports).

## Acceptance evidence

- **12 runtime supervisor unit tests** (`crates/altior-core/tests/p12_runtime.rs`):
  - Session creation, probing, and resumption.
  - Streaming event flows and delta aggregation.
  - Permission pause, user approval, and denial routing.
  - Cancellation races and idempotent cancellation handling.
  - Single-active-turn enforcement and multiple-thread isolation.
  - Transport failures and unexpected process exits marking turns `Indeterminate` and strictly forbidding resends.
  - Diagnostics redaction and unknown event preservation.
  - UI client detachment resilience.
- **6 end-to-end real subprocess composition tests** (`crates/altior-core/tests/p12_composition.rs`):
  - Happy path streaming and terminal settlement.
  - Subprocess crash (exit code 42) leading to `Indeterminate` settlement and restart recovery.
  - Permission prompt request and user decision flow.
  - Turn cancellation and clean child process termination.
  - Secret canary validation (zero leakage in DB, journals, or diagnostics).
  - Multi-run RAII executable cleanup and process reap.
- **6 storage checkpoint integration tests** (`crates/altior-storage/tests/checkpoint.rs`):
  - Schema v4 migration.
  - Pre-call intent recording and round-trips.
  - Settlement state transitions and conflict detection.
  - Startup recovery of unsettled intents to `Indeterminate`.
  - Session binding CRUD operations.
- **6 process adapter transport tests** (`crates/altior-acp/tests/process_adapter.rs`):
  - Subprocess spawn, line I/O, stderr capture, graceful exit, and RAII termination.
- **Total Rust test suite**: All 160+ unit and integration tests pass cleanly across all crates.
