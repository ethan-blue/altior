# ADR 0006: Local IPC transport and Core process model

- Status: Accepted
- Date: 2026-08-29

## Context

P0.2 must select and record how Desktop and `altior-core` talk locally and how
Core's process lifetime is supervised. `docs/ARCHITECTURE.md` fixes the
topology: Core is a separately supervised process that owns durable state and
agent processes, and closing or reloading the UI must not terminate active
work. `docs/SECURITY.md` requires authenticated local IPC with a per-launch
capability token. `docs/IMPLEMENTATION_PLAN.md` P0.2 adds sequence/catch-up
semantics for event subscriptions, and its evidence requires: UI reload does
not stop a synthetic active turn, Core restart exposes recovery state without
duplicate commands, and wrong protocol versions plus stale sockets/pipes fail
clearly.

Constraints:

- Windows is the pinned primary environment; the design must work there first
  and stay implementable on Unix.
- Desktop is less privileged than Core (`docs/SECURITY.md`); nothing in the
  transport may grant Desktop file, secret, or process access.
- Tests must be deterministic: no sleeps, no public network, no scheduler
  luck. Real sockets and real child processes therefore cannot be the only
  evidence; contracts and state machines must be provable in-process.
- Tauri arrives in P0.4 (ADR 0003), so nothing here may depend on it.
- AGENTS.md forbids undocumented frameworks; any runtime dependency this ADR
  selects must be named here.

## Decision

### Transport: local sockets, not stdio, not TCP

1. Desktop and Core communicate over a **local socket owned by Core**:
   a Windows named pipe on Windows, a Unix domain socket on Unix. There is
   exactly one Core per user session.
2. The endpoint name is derived, not configured: the user name scopes it
   (`\\.\pipe\altior-core-<user>` on Windows,
   `$XDG_RUNTIME_DIR/altior/core-<user>.sock` or
   `$TMPDIR` fallback on Unix). `altior-ipc::endpoint` owns the derivation
   as a pure function over injected inputs, so tests never touch the OS.
3. Rejected alternatives:
   - **stdio with Core as a Desktop child.** Couples Core's lifetime to the
     UI process (violates "reload must not stop work" at the process level),
     allows exactly one client, and cannot support reconnect after a Desktop
     restart. ADR 0003 already keeps Tauri out until P0.4.
   - **Loopback TCP.** Port allocation and collision handling, Windows
     firewall prompts, and a weaker local-only story (any local process can
     dial an open port; named pipes and domain sockets carry OS-level
     user scoping for free).
   - **Tauri sidecar channels.** Ties the contract to a UI framework that is
     not in scope yet and inherits stdio's single-client limits.

### Framing: length-prefixed JSON with a hard bound

1. Each message is one frame: a 4-byte big-endian unsigned length followed
   by that many UTF-8 bytes of compact canonical JSON (the same encoding the
   fixture suite pins byte-for-byte).
2. The frame cap is 256 KiB. Envelope payloads are already bounded at 64 KiB
   (ADR 0004); the cap leaves headroom for envelope overhead without
   inviting abuse. A frame that exceeds the cap, a length that disagrees
   with the payload, or invalid JSON is a typed `FrameError` and the
   connection is closed — never a panic, never a partial read that the peer
   must resynchronize.
3. `altior-ipc::frame` implements encode/decode as an incremental state
   machine over fed bytes, so real streaming I/O later only supplies chunks.
   No async runtime is needed for the codec itself.

### Authentication: per-launch capability token

1. On startup Core generates a launch token from OS entropy (hex-encoded,
   minimum 128 bits) and writes a token file — readable only by the current
   user — next to the endpoint, binding `{instance_id, token}`.
2. Desktop reads the token file and presents the token in `DesktopHello`.
   Core rejects the session with a typed error before any negotiation
   output; an unauthenticated local process therefore cannot drive Core,
   satisfying the `docs/SECURITY.md` control.
3. `negotiate()` itself stays pure version/capability math (ADR 0004); token
   validation lives in the IPC session layer, which composes
   `authenticate → negotiate → greet`.
4. Entropy is injected: tests pass fixed bytes; only the production binary
   asks the OS. The domain never generates tokens (ADR 0004 rule).

### Process model: spawn-or-attach, Core outlives Desktop

1. **Desktop does not own Core.** The supervisor probes the endpoint: if a
   healthy Core answers, Desktop attaches; if the endpoint is stale or
   absent, the supervisor emits a spawn decision that the platform layer
   executes (detached process on Windows via safe `std::os::windows::process`
   creation flags; the Unix detach path is deferred and recorded, because
   the safe std API does not cover `setsid`).
2. Supervision is a **pure state machine over events**, not hidden timers:
   health checks are `Ping` commands, reconnect and backoff policies are
   data the host executes. Deterministic tests drive the machine directly.
3. Shutdown is explicit: Desktop sends a stop request only when the user
   quits the whole application AND no turns are active; otherwise Core keeps
   running. Window close and reload are detach events that never reach turn
   ownership.

### Sequence/catch-up: epochs and retained windows

1. Every Core launch has a `CoreInstanceId` (new domain ID, `cor_` prefix,
   generated by infra from entropy). `Sequence` numbers are meaningful only
   within one instance: a restarted Core starts a fresh sequence space.
2. After authentication and negotiation, Core sends a `CoreGreeting`
   carrying `instance_id` and the retained window
   `[retained_from, retained_to]` of its in-memory event buffer. Desktop
   subscribes with `Subscribe { since }`.
3. Catch-up rules (implemented in `altior-ipc::session`):
   - same `instance_id` and `since + 1 ≥ retained_from` → Core replays
     `(since, retained_to]` then continues live, ending the replay with
     `stream.replayed { from, through }`;
   - the requested range is no longer retained → Core sends
     `stream.gap { from }`; Desktop must issue `RequestSnapshot` (the
     P0.1 snapshot envelope) and re-derive state from the snapshot;
   - different `instance_id` (restart) → Desktop treats every old sequence
     as stale: it requests a snapshot, and its command ledger keeps
     `OperationId`s so no command is sent twice across the restart.
4. Replay preserves original sequences and event ids; duplicates are
   detectable by `event_id`, so a reconnecting Desktop can drop re-delivered
   events idempotently.

### Runtime dependency selection (for the later OS wiring slice)

The OS-facing slice (real named pipe/UDS listener, detached spawn) will use
`tokio` as the single async runtime, selected here per AGENTS.md: it is the
maintained standard, provides both Windows named pipes and Unix domain
sockets natively, and matches the Tauri ecosystem already chosen in
ADR 0003. It is allowed only in `altior-ipc` and `altior-core`; the
domain/protocol crates stay runtime-free. This slice lands later; the
contract layer here has zero async dependencies.

## Alternatives considered

- **gRPC/protobuf or JSON-RPC over the same transport.** A second schema
  language duplicates the serde contracts ADR 0004 froze, and the fixture
  byte-canonicality would need a new pipeline. The wire stays JSON.
- **Newline-delimited JSON instead of length prefixes.** Payloads may
  legitimately contain newlines inside strings; escaping rules complicate
  partial-read recovery. Length prefixes make frame boundaries unambiguous.
- **Auth via OS ACLs only.** Named pipe ACLs restrict who may connect but
  not which connection is legitimate for this launch; the token also
  detects stale endpoints (an old Core's token will not match its
  `instance_id`).
- **A sequence space preserved across restarts via persisted counters.**
  Correct, but it drags storage (P0.5/P1.1) into P0.2. Epoch change plus
  snapshot recovery gives the same observable guarantees earlier; the
  persisted counter can replace epochs later without changing the DTOs.

## Failure modes

- **Stale endpoint** (Core died, pipe lingers): probe fails or greeting
  epoch mismatches → typed `EndpointError::Stale`; supervisor spawns a new
  Core. Never a silent attach to a dead pipe.
- **Wrong protocol version**: `NoCommonProtocolVersion` (ADR 0004) after
  authentication; the connection closes with the explicit error.
- **Token file missing/unreadable**: Desktop cannot authenticate → explicit
  `AuthError`; supervisor treats Core as not running.
- **Malformed or oversized frame**: `FrameError`, connection closed; the
  peer's session machine returns to `Disconnected`, ready for reconnect.
- **Duplicate replay after reconnect**: same `event_id` re-delivered;
  Desktop's dedup by `event_id` drops it. Duplicate *commands* are
  prevented by the `OperationId` ledger, not by hoping the user does not
  retry.
- **Clock skew**: nothing in supervision depends on wall clocks; liveness
  is Ping/response, ordering is `Sequence`, epochs are `CoreInstanceId`.
- **Two Cores racing on one endpoint**: the OS allows only one listener per
  pipe/socket name; the loser exits with a typed bind error recorded in its
  diagnostics.

## Migration path

- P0.2 ships the contract layer (frames, endpoint naming, tokens, session
  machines) with deterministic in-process tests; the OS-facing slice
  (tokio listener, detached spawn) composes the same types without
  contract changes.
- If persisted sequence counters arrive with storage, `CoreGreeting` gains
  an optional continuity field; clients that never saw it fall back to
  epoch comparison. No existing field changes meaning.
- If a second Desktop client (another window) is allowed later, the same
  endpoint and token serve multiple sessions; the greeting/subscribe
  design is already per-connection.

## Exit strategy

- If named pipes prove limiting on Windows (ACL edge cases, sandboxed
  deployment), the transport is one type inside `altior-ipc`; switching to
  UDS-over-Windows or a Tauri-native channel replaces the endpoint module
  without touching frames, auth, sessions, or any domain/protocol type.
- If tokio is rejected later, only the OS-facing slice is affected; the
  contract layer here has no async dependency to unwind.

## Revisit when

- The tokio OS-facing slice is implemented (verify the composition
  assumption).
- Storage (P0.5/P1.1) can persist sequence counters across restarts.
- A second simultaneous Desktop client becomes a product requirement.
