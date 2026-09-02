# ADR 0016: P1.4 acceptance journey, binding v5, and journey-hardened runtime

- Status: Accepted
- Date: 2026-09-02
- Phase: P1.4
- Supersedes: none (extends ADR 0013 storage, ADR 0014 runtime, ADR 0015 IPC)

## Context

P1.3 connected the Desktop workbench to a real Core daemon over OS local IPC,
but the product journey had gaps: `configure_agent` could not persist a harness
binding over IPC, opening a thread did not auto-select a default binding, the
Core default discovery path diverged from the Tauri shell's, and no end-to-end
evidence exercised the full eight-step acceptance journey against a real
daemon process, real transport, and persistent SQLite.

Driving the journey to green also exposed three real product defects that
in-process tests had masked:

1. **Silent connection closes.** The daemon closed client connections on
   command dispatch errors without logging or replying, making field
   debugging impossible.
2. **Degraded wire events.** The event pump serialized `permission.requested`,
   `turn.cancelled`, and `turn.failed` as `Unknown` provider events instead of
   the protocol's `KnownEvent` variants; typed clients (Desktop included)
   could not parse them.
3. **Un-cancellable quiet agents.** The ACP prompt loop checked the cancel
   signal only between blocking reads; an agent that stayed silent while
   working (normal behavior) could never be cancelled, because the worker was
   parked in `read_line()` while the child waited for `session/cancel`.

## Decision

1. **Harness binding v5 (`altior-storage` schema v5, `altior-domain`,
   `altior-protocol`).** `AcpHarnessBinding` gains `args`, `env_keys`, and
   `secret_refs` with domain bounds (args ≤ 256 / 64 KiB each, env keys ≤ 256 /
   256 B, secret refs ≤ 256 B opaque). `ConfigureAgentCommand` accepts an
   optional `HarnessBindingConfigDto` and persists a durable binding;
   `env_keys.len() == secret_refs.len()` is enforced at the DTO boundary.
   Schema v5 adds nullable-default columns, forward-only as before.
2. **Default binding auto-selection.** `open_thread` resolves a binding by:
   explicit binding > persisted session binding > first binding for the agent,
   with a typed `MissingHarnessBinding` error otherwise. Sessions persist via
   `thread_session_binding` across daemon restarts, and no-auto-resend is
   enforced durably (turns and checkpoints), not just in memory.
3. **Daemon connection-close observability.** Every abnormal connection close
   (frame error, control/normal dispatch error, event send failure) logs the
   cause to stderr before closing. Client-facing behavior is unchanged.
4. **Known event mapping.** `map_runtime_to_event_body` emits
   `KnownEvent::PermissionRequested` / `TurnCancelled` / `TurnFailed` with
   typed fields; only genuinely unmapped events (e.g. `process.exited`) stay
   `Unknown`.
5. **Interruptible prompt reads.** `AcpChild` now pumps stdout lines through a
   dedicated reader thread into a channel; the transport trait gains
   `read_line_timeout`, and the prompt loop polls the cancellation signal every
   100 ms while the child is silent. This also prevents stdout pipe-buffer
   saturation during `close()` (the reader always drains).
6. **Journey evidence (`crates/altior-core/tests/p14_acceptance_journey.rs`).**
   A single test runs all eight acceptance steps against a real
   `altior-core --daemon` process on a real named pipe with a real SQLite file
   and two unique mock ACP agent binaries (`agent_a_full` with loadSession and
   permission flow, `agent_b_minimal` with cancel/crash triggers). Steps:
   configure+probe two agents over IPC (with SQLite row asserts), create/open
   threads with auto binding, permission request + approval, cooperative
   cancel, client disconnect + `Subscribe(since)` replay with
   `stream.replayed` boundary, agent crash (exit 42) → Indeterminate, daemon
   restart → resend forbidden, and offline FTS search/history. Guards are
   PID-level RAII with no sleeps (deadline + yield), and the harness captures
   daemon stdout/stderr to files for failure diagnostics.

## Consequences

- The eight-step acceptance journey passes deterministically on Windows
  (~1.2 s) and would surface regressions in transport, cancel, replay, or
  resend semantics.
- Cancellation latency for silent agents is bounded by the 100 ms poll
  interval; chatty agents cancel between lines as before.
- One extra reader thread per agent child (mirrors the existing stderr
  capture thread); it exits at EOF and is joined on `terminate`/`close`.
- Deferred (unchanged from ADR 0015): real third-party dual ACP agents on a
  clean OS profile, OS secret manager (DPAPI/Keychain), signed installers, and
  Windows Job Object hierarchy remain P1.5+/P5 scope.
