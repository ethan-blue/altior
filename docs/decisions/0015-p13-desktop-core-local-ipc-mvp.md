# ADR 0015: P1.3 Desktop Core Local IPC and Application Store MVP

Date: 2026-08-31 · Status: accepted · Scope: P1.3 Desktop MVP (`docs/IMPLEMENTATION_PLAN.md`), feeding P1.4 Acceptance Journey.

## Context

`docs/decisions/0006-local-ipc-transport-and-core-process-model.md` established the pure state machines for IPC session management, framing, and spawn-or-attach supervision. `docs/decisions/0008-p04-desktop-shell-virtualization-and-tauri-minimums.md` established the Desktop 5-region shell with virtualized timeline and Tauri v2 capability configuration. `docs/decisions/0013-p11-domain-entities-and-persistence.md` delivered domain entities, durable journal persistence, and projections. `docs/decisions/0014-p12-acp-runtime-checkpoints-and-supervision.md` implemented the ACP child process runtime, worker thread supervision, and intent checkpoints.

Prior to P1.3:
1. The Desktop frontend (`apps/desktop`) communicated only via an `InMemoryTransport` double with pre-canned fixture data.
2. The Tauri shell (`apps/desktop/src-tauri`) lacked the live OS IPC bridge to discover or spawn the `altior-core` daemon.
3. The IPC transport layer (`altior-ipc`) had pure state machines but lacked the physical OS-level listener/stream implementations (Windows Named Pipes and Unix Domain Sockets) and atomic discovery file publication.
4. `altior-core` lacked the application-level command dispatcher (`CoreApplication`), the persistent server daemon loop (`Daemon`), and the multi-client event broadcasting pump (`EventPump`).

P1.3 bridges the Desktop UI to the live Core runtime over OS-native local IPC, delivering an end-to-end usable Desktop workbench for agent configuration, thread management, turn execution, streaming message deltas, cancellation, permission approvals, full-text search, and diagnostics.

## Decision

### 1. Protocol P1.3 Commands, DTOs, Snapshots, and Events

The Desktop/Core protocol contracts in `altior-protocol` are expanded and exported to TypeScript DTOs (`apps/desktop/src/ipc/dto/`):

- **Commands (12 kinds)**:
  - `create_thread`: Creates a new thread bound to an agent profile and optional project.
  - `open_thread`: Fetches a complete `ThreadSnapshotDto` for a selected thread.
  - `list_threads`: Queries paginated `ThreadListResponseDto` sorted newest-first by updated timestamp.
  - `search_threads`: Performs literal FTS5 search returning matched `ThreadListResponseDto`.
  - `get_history`: Fetches paginated chronological `ThreadHistoryResponseDto` for a thread.
  - `start_turn`: Dispatches a user prompt to the thread runtime supervisor, initiating an active turn.
  - `cancel_turn`: Signals cooperative cancellation to an active turn's worker thread.
  - `respond_permission`: Resolves a pending permission request (`Approved` or `Denied`).
  - `configure_agent`: Creates or updates an `AgentProfile` and `AcpHarnessBinding` with opaque secret references.
  - `test_harness_binding`: Probes an ACP harness binding and returns health latency.
  - `runtime_status`: Retrieves the current `RuntimeDiagnosticsDto` (instance ID, health, active counts).
  - `diagnostics`: Retrieves bounded, redacted diagnostic logs.

- **Data Transfer Objects (DTOs)**:
  - `ThreadDto`, `TurnDto`, `PermissionDto`, `AgentProfileDto`, `HarnessBindingDto`.
  - `ThreadCursorDto`, `TurnCursorDto` (composite cursors for deterministic pagination).
  - `ThreadSummaryDto` (thread record + optional last turn + optional active turn).
  - `ThreadSnapshotDto` (thread record + agent profile + recent turns + pending permissions).
  - `ThreadListResponseDto` (page of summaries + next cursor + `has_more`).
  - `ThreadHistoryResponseDto` (thread ID + page of turns + next cursor + `has_more`).
  - `RuntimeDiagnosticsDto` (instance ID, status, active threads, active turns, redacted summary).

- **Snapshots & Event Envelopes**:
  - Command requests and responses map to typed `SnapshotEnvelope` payloads.
  - Streaming event envelopes wrap `KnownEvent` variants: `TurnStarted`, `MessageDelta`, `ToolCallStarted`, `ToolCallFinished`, `PermissionRequested`, `PermissionDecided`, `TurnCompleted`, `TurnCancelled`, `TurnFailed`, `StreamReplayed`, `StreamReady`, `StreamGap`, and `RawUnknown`.
  - All DTO bindings are deterministically exported via `cargo test -p altior-protocol --features dto-export` with LF newlines and trailing whitespace stripping.

### 2. OS Local IPC Transport, Security, and Framing

The `altior-ipc` transport layer implements physical OS communication:

- **Windows Named Pipes & Unix Domain Sockets**:
  - Windows: Named pipe path `\\.\pipe\altior-ipc-<hash-or-user>` (path length limit ≤ 256 bytes).
  - Unix: Unix domain socket at `$XDG_RUNTIME_DIR/altior/ipc.sock` or `$TMPDIR/altior/ipc.sock` (path length limit ≤ 104 bytes).
  - Physical abstractions: `LocalListener`, `LocalStream`, `PlatformListener`, `PlatformStream`.

- **Atomic Discovery File Publication**:
  - On startup, Core writes `discovery.json` containing `instance_id`, `endpoint`, and a per-launch 32-byte hex `launch_token`.
  - Writes use tempfile creation followed by atomic filesystem rename (`std::fs::rename`).
  - File permissions: On Unix, discovery files and socket directories enforce mode `0600`/`0700` (`umask 0077`). On Windows, default per-user ACLs restrict access to the current user token.
  - Stale discovery files (from previous crashed Core instances) are detected during attach and cleaned up before respawn.

- **Framing & Resource Caps**:
  - 4-byte big-endian length prefix followed by UTF-8 JSON.
  - Hard frame cap: `MAX_FRAME_BYTES = 262_144` (256 KiB). Declared lengths exceeding 256 KiB are rejected immediately before reading data, preventing heap exhaustion attacks.
  - Slow handshake timeout: `DEFAULT_HANDSHAKE_TIMEOUT` (5 seconds). Connections that fail to complete `DesktopHello` within 5 seconds are terminated.
  - Concurrent session cap: `DEFAULT_MAX_CLIENT_SESSIONS` (32 concurrent client connections).

- **Debug Redaction**:
  - `EndpointDiscovery`, `LaunchToken`, `LaunchCredentials`, `CoreDaemonConfig`, and `SecretRef` implement custom `fmt::Debug` implementations that format tokens as `[REDACTED]`. Plaintext secrets and launch tokens never leak to logs, console output, or diagnostics.

### 3. CoreApplication, Daemon Server, and EventPump

`altior-core` integrates domain persistence, ACP runtime supervision, and IPC serving:

- **CoreApplication**:
  - Coordinates `altior_storage::Store`, `altior_core::runtime::AgentRuntimeSupervisor`, and `altior_core::runtime::adapters::acp::AcpHarnessAdapter`.
  - Implements `dispatch_command`: executes domain CRUD transactions, validates inputs, queries FTS5 search indexes, initiates turns, routes permission decisions, and gathers runtime diagnostics.

- **Daemon Server Loop**:
  - `altior-core::application::daemon::Daemon` binds the local OS listener, publishes `discovery.json`, and runs the persistent server loop.
  - Accepts client connections, authenticates `launch_token` against `DesktopHello`, initializes per-connection `ServerSession` machines, and routes incoming commands to `CoreApplication`.
  - Implements control-priority message processing (commands and cancels are processed ahead of bulk data frames).

- **EventPump & Multi-Connection Replay**:
  - Background turn events from `AgentRuntimeSupervisor` flow into the `EventPump`.
  - Maintains a monotonic `EventLog` with sequence numbers and a retained historical replay window.
  - On client connection and `subscribe` command, `EventPump` replays missed events within the retained window wrapped in `stream.replayed`, emits `stream.ready` when caught up, or emits `stream.gap` if the requested sequence was evicted.
  - Multiple Desktop windows or reconnections can connect simultaneously; dropping or reloading a UI client leaves the daemon and background turns executing uninterrupted.

- **Two-Sided Duplicate Prevention**:
  - Receiving-side deduplication via Core `OperationRegistry`.
  - Sending-side deduplication via Desktop `CommandLedger`.

### 4. Desktop Client Architecture, Tauri Bridge, and ApplicationStore

`apps/desktop` and `apps/desktop/src-tauri` connect the user interface to the live Core daemon:

- **Tauri Spawn-or-Attach Bridge (`altior_desktop_shell`)**:
  - On startup, the Tauri backend attempts to discover a running Core daemon via `discovery.json`.
  - If a valid daemon is reachable, it attaches using the discovery `endpoint` and `launch_token`.
  - If no daemon exists or the discovery endpoint is stale/unreachable, it cleans up the stale file and spawns `altior-core` as a managed background daemon process.
  - Implements Tauri IPC commands: `send_command`, `poll_events`, `get_connection_status`.
  - Dropping the Tauri UI state handle (`ui_state_drop_does_not_kill_child`) leaves the Core child daemon alive.

- **Production Transport Selection**:
  - `apps/desktop/src/main.tsx` initializes `TauriCoreTransport` as the production default when running inside Tauri (`window.__TAURI__` / `window.__TAURI_INTERNALS__`).
  - Falls back to `InMemoryTransport` during web browser development or headless tests.

- **ApplicationStore State Management (`apps/desktop/src/stores/applicationStore.ts`)**:
  - Pure reactive store managing Desktop state:
    - **Agents**: Lists configured agent profiles, configures new agents with opaque `secretRef`, tests harness latency.
    - **Threads**: Lists threads (paginated), creates threads, switches active thread, searches threads with live FTS5 query debouncing, loads chronological history.
    - **Turns & Timeline**: Dispatches `start_turn`, streams incoming `message_delta` tokens into virtualized timeline rows, renders tool calls and plans, tracks active turn state.
    - **Permissions**: Displays inline permission prompts (`execute`, `read`, `write`, `network`), dispatches `respond_permission` (Approved / Denied), handles cancellation rollback.
    - **Diagnostics & Status**: Fetches runtime health status and redacted diagnostic summaries.

## Alternatives Considered

1. **Embedding Core as a Direct Dynamic Library (DLL/dylib) inside Tauri UI**:
   - *Rejected*: Violates the core architecture principle that closing, reloading, or crashing the Desktop UI must not terminate active agent turns or corrupt local SQLite journals. A standalone Core daemon ensures turn survival across UI reloads.

2. **WebSocket / HTTP Local Server instead of Named Pipes / Unix Sockets**:
   - *Rejected*: Local HTTP servers require TCP port allocation, risk port collisions, trigger Windows Firewall security prompts, and allow any unprivileged local process on the machine to port-scan and connect. Named Pipes and Unix Domain Sockets provide native OS-level user ACL access control.

3. **Plaintext Secrets in IPC DTOs**:
   - *Rejected*: In accordance with ADR 0006 and ADR 0014, plaintext credentials (such as API keys) must never cross IPC boundaries, be logged in diagnostics, or be stored in SQLite. Only opaque references (`SecretRef`) are exchanged.

## Failure Modes and Resilience

1. **Slow Handshake / Unauthenticated Connection Attack**:
   - *Mitigation*: Connections that fail to complete `DesktopHello` within `DEFAULT_HANDSHAKE_TIMEOUT` (5 seconds) or provide an invalid `launch_token` are immediately closed.

2. **Oversized / Malformed IPC Frame**:
   - *Mitigation*: Frame lengths exceeding `MAX_FRAME_BYTES` (256 KiB) are rejected at the 4-byte header stage before allocating memory. Malformed JSON returns typed `ProtocolError::MalformedEnvelope`.

3. **Desktop UI Reload or Crash during Active Prompt**:
   - *Mitigation*: Core daemon holds turn ownership. The agent subprocess continues execution in the background, writing events to SQLite and `EventLog`. When Desktop re-attaches, `subscribe` replays the buffered turn events seamlessly.

4. **Stale Discovery File from Core Crash**:
   - *Mitigation*: Tauri `SpawnOrAttach` probes the endpoint declared in `discovery.json`. If connection fails, the stale file is deleted and a fresh Core daemon is spawned.

5. **Subprocess Orphan Prevention**:
   - *Mitigation*: `altior-acp::AcpChild` RAII `KillOnDrop` terminates and reaps child processes when Core shuts down.

## Migration and Compatibility

- **Protocol Versioning**: Handshake uses `ProtocolVersionRange` with version intersection. Current supported version is `0.1.0`.
- **TypeScript DTO Generation**: DTOs in `apps/desktop/src/ipc/dto/` are generated from Rust structs via `ts-rs` and checked into version control. Any protocol schema change requires running `cargo test -p altior-protocol --features dto-export` and committing updated TypeScript bindings.
- **Storage Compatibility**: Retains `SCHEMA_V4` SQLite schema established in P1.2.

## What is NOT Yet Done (Deferred to P1.4 / P5)

To ensure clarity and avoid overstatement of current capabilities, the following items remain outside P1.3 and belong to future milestones:
- **P1.4 Acceptance Journey**: End-to-end multi-agent verification using real external third-party ACP agents (e.g. running two separate live CLI/server agents simultaneously with live API credentials on a clean OS profile).
- **OS Secret Manager Integration**: `SecretRef` currently uses the `NoSecretsResolver` seam. Native integration with Windows DPAPI / Credential Manager, macOS Keychain, and Linux SecretService is scheduled for P1.4/P5.
- **Windows Job Object Process Trees**: Child process containment currently relies on standard library RAII process handles (`AcpChild` `Drop`) rather than Windows Job Objects (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
- **Signed Installer & Packaging**: MSIX / NSIS packaging and code signing are scheduled for P5 release hardening.
- **Encrypted Sync & Multi-Device Pairing**: P3 scope.

## Revisit Triggers

1. IPC throughput bottlenecks under extremely high-frequency event streaming (> 10,000 events/sec) requiring shared memory (ring buffers).
2. Addition of non-ACP harnesses (e.g. native terminal PTY or Codex app-server) requiring protocol envelope extensions.
3. Multi-agent delegation requiring parent/child turn correlation across separate processes.

## Evidence

- **Rust Workspace Tests**: All tests across the entire workspace pass cleanly (`cargo test --workspace`).
- **P1.3 Application Integration Tests (`crates/altior-core/tests/p13_application.rs`)**: 21 tests covering CoreApplication command dispatching, thread CRUD, FTS5 search, start turn, turn cancellation, permission approval/denial routing, duplicate operation prevention, UI disconnect resilience, and diagnostics.
- **P1.3 Local IPC End-to-End Test (`crates/altior-core/tests/p13_local_ipc.rs`)**: 1 test verifying end-to-end local IPC server bind, discovery publication, client connection, handshake authentication, command dispatch, and event streaming.
- **P1.3 Daemon Process Test (`crates/altior-core/tests/p13_daemon_process.rs`)**: 1 test verifying real OS daemon subprocess execution, discovery file lifecycle, sequential client runs, and clean shutdown.
- **IPC Transport Tests (`crates/altior-ipc/tests/transport.rs`)**: 9 tests verifying Windows named pipe / Unix domain socket bind/connect, oversized frame rejection, path length limits (104/105 bytes), bad token rejection, discovery file lifecycle, and zero-plaintext Debug redaction.
- **DTO Export Tests (`crates/altior-protocol/tests/dto_export.rs`)**: 3 tests verifying deterministic TypeScript DTO export, idempotency, and boolean type mappings.
- **Tauri Shell Tests (`apps/desktop/src-tauri`)**: 7 tests verifying `SpawnOrAttach`, child process survival across UI state drops, reconnect cursor continuity, command/event mapping with deduplication, stale discovery cleanup, and bad auth rejection.
- **Desktop Vitest Suite (`apps/desktop`)**: 69 tests across 9 test files verifying application store state transitions, timeline virtualization, Tauri transport, in-memory transport, and UI components.
- **Strict TypeScript & Build**: `tsc --noEmit` and `vite build` complete with zero errors or warnings.
