# ADR 0007: ACP v1 adapter scope and mapping

- Status: Accepted
- Date: 2026-08-29

## Context

P0.3 requires a *narrow* ACP (Agent Client Protocol) v1 adapter living
outside the domain crates, proven against two real external agents in
opt-in smoke tests, with deterministic evidence for everything else:
normalized trace fixtures for two agents, unknown-event and malformed
frame tests, and process crash / idle timeout / cancellation / cleanup
tests. `docs/ARCHITECTURE.md` fixes ACP as the first replaceable agent
harness behind stable domain ports, and `docs/WORK_PACKAGES.md` gates P1
on this spike. ACP itself is JSON-RPC 2.0 over a byte stream; the
reference agents speak newline-delimited JSON over the subprocess's
stdin/stdout.

The authoritative shape source for this spike is the published ACP v1
JSON Schema (`schema/v1/schema.json` in the agent-client-protocol
repository), extracted on 2026-08-29 and summarized in the mapping table
below. The adapter models only the subset it maps; everything else is
preserved, following the unknown-event preservation convention of
ADR 0004.

Constraints:

- AGENTS.md: docs/contracts/tests/fixtures are long-term truth; an ADR
  must precede any protocol/framework adoption; tests are deterministic
  (no public network, no sleeps, no machine load, no scheduler luck).
- `altior-domain` and `altior-protocol` stay platform-neutral; the
  adapter sits *beside* them, not inside them ("outside the domain
  crates").
- Fixtures must be synthetic — no real conversation content, no API keys.
- Delivery classification must use the frozen `DeliveryState` vocabulary
  (ADR 0004): `Absent`, `Confirmed`, `Rejected`, `Indeterminate`; resend
  is allowed only for provably `Absent` or explicitly `Rejected` prompts.
- No async runtime yet: ADR 0006 defers tokio to the OS-facing slice, and
  this spike must not reverse that.

## Decision

### 1. New crate `altior-acp`, outside the contract crates

`crates/altior-acp` depends only on `altior-domain`, `altior-protocol`,
`serde`, and `serde_json` — the same boundary as `altior-ipc`. Nothing
depends on it yet; P1 wires it into Core behind the stable harness port.
The dependency-boundary test in `altior-core` pins this.

### 2. Transport and framing for the spike

The spike models ACP as **newline-delimited JSON-RPC 2.0 over the
subprocess stdin/stdout**, with a hard line cap (1 MiB) and typed
malformed-frame errors. The wire layer is pure: `encode_line` /
`LineDecoder` mirror the IPC frame codec of ADR 0006 (feed bytes, receive
whole messages), so the later async transport only supplies bytes.

Agent-side JSON-RPC dispatch is *method- and shape-driven*: the adapter
never parses more than the method name and the fields it maps.

### 3. Capability negotiation, never version-string inspection

The `initialize` handshake is negotiated on the **capabilities** in the
`InitializeResponse` (`loadSession`, prompt content capabilities, session
capabilities), as the plan requires. The `protocolVersion` number is
recorded as opaque data and sanity-checked (present, integer), but **no
feature gate ever keys off it** — ACP itself bumps the version only for
breaking changes and adds features through capabilities. Falling back on
an unknown capability object is a typed outcome (`NegotiatedCapabilities`
with all-optional booleans), not an error.

### 4. Mapping table (ACP wire → Altior events)

The adapter maps onto the existing protocol event vocabulary
(ADR 0004/0006); P1 grows dedicated `KnownEvent` kinds when the Desktop
actually renders them. Non-turn events map to the bounded preserved form
(`EventBody::Preserved`) with an `acp.` diagnostic prefix so nothing is
silently dropped. The full table:

| ACP wire | Adapter event | Protocol target |
| --- | --- | --- |
| `session/new` → result `sessionId` | session bound (adapter state) | — (thread binding is P1) |
| `session/prompt` request written | `Turn Started` | `KnownEvent::TurnStarted` |
| `session/update` `agent_message_chunk` (text) | `Delta(text)` | `KnownEvent::MessageDelta` |
| `session/prompt` → `stopReason: end_turn` | `Turn Completed` | `KnownEvent::TurnCompleted` |
| `session/prompt` → `stopReason: cancelled` | `Turn Cancelled` | `KnownEvent::TurnCompleted` (cause recorded in the adapter, matching the P0.2 ownership model) |
| `session/prompt` → `refusal`/`max_tokens`/`max_turn_requests` | `Turn Failed { diagnostic }` | preserved form `acp.turn.failed` |
| `session/update` `tool_call`/`tool_call_update` | `Tool { tool_call_id, status }` | preserved form `acp.tool` |
| `session/request_permission` (agent→client request) | `Permission Requested { request_id, tool_call_id }` | preserved form `acp.permission.requested` |
| client answers permission | `Permission Answered` | — (client-side state) |
| `session/cancel` notification | `Cancel Sent` | — (client-side command) |
| `session/load` result (gated on `loadSession`) | `Session Resumed` | — (thread binding is P1) |
| JSON-RPC error response / `Error` | `Failed { diagnostic }` | preserved form `acp.failure` |
| any other `sessionUpdate` variant or content block | `Unmapped` | preserved form, unknown kind kept verbatim |
| `fs/read_text_file` / `fs/write_text_file` (agent→client) | refused with a typed JSON-RPC error | — (Desktop FS access is the P4 workbench behind permission profiles) |

`user_message_chunk` and `agent_thought_chunk` are `Unmapped` in the
spike: the Desktop already knows the user's prompt, and thought rendering
is a P1 UI decision; both are preserved, not dropped.

### 5. Delivery classification

`DeliveryTracker` maps wire outcomes onto `DeliveryState`:

- the prompt line provably never entered the pipe (encode failure, dead
  process before write) → `Absent`;
- a JSON-RPC **error response** named the prompt request → `Rejected`;
- a `PromptResponse` (any `stopReason`, including `cancelled`) arrived →
  `Confirmed`;
- the process crashed, the stream ended, or the idle budget elapsed while
  the prompt was outstanding → `Indeterminate`.

Resend policy is enforced by the tracker (mirror of the ADR 0004 rule):
only `Absent` and `Rejected` may be re-sent; `Indeterminate` surfaces to
the user, never auto-retries.

### 6. Process lifecycle

`AgentLifecycle` is a pure decision machine in the style of the P0.2
`Supervisor`: no timers, no threads, no I/O. Inputs are explicit events
(exited, stream ended, idle budget elapsed, cancel requested); outputs
are decisions (kill-and-wait, answer pending permissions with
`cancelled`, classify outstanding deliveries, reap). The opt-in smoke
host executes the decisions against a real child process; the
deterministic tests drive the machine directly.

Cleanup contract: cancellation must answer every pending
`session/request_permission` with `{"outcome":"cancelled"}`, wait for the
prompt response, and only then kill the child; a crashed process
classifies every outstanding delivery `Indeterminate` (never `Absent` —
bytes may have been consumed).

### 7. Opt-in smoke tests against real agents

Real agents need API keys and subprocess spawning; both violate the
determinism rules for default gates. The smoke integration test is
therefore opt-in: it runs only when `ALTIOR_ACP_SMOKE_AGENTS` is set to a
`;;`-separated list of command lines (two entries satisfy the plan), and
skips with an explicit message otherwise. Default `cargo test` runs stay
hermetic; the fixtures carry the deterministic evidence.

## Alternatives considered

- **Adopt the reference `agent-client-protocol` Rust crate.** It would
  drag generated types for the whole schema and its own error model into
  a boundary that must stay narrow and serde-only. Revisit if the P1
  mapping surface grows past the subset.
- **LSP-style `Content-Length` headers.** The reference agents speak
  newline-delimited JSON; inventing a second framing would diverge from
  every real agent. The line codec is deliberately ADR-0006-shaped so a
  header transport could slot in later if the ecosystem shifts.
- **Gate features on `protocolVersion`.** Explicitly rejected by the
  plan: ACP evolves via capabilities; version inspection leads to
  agent-name special-casing. Version is recorded, never branched on.
- **Extend `KnownEvent` with tool/permission/failure kinds now.** The
  spike maps them into the preserved form instead; P1 adds kinds when
  the Desktop renders them, keeping the protocol fixtures stable through
  P0.
- **Async transport in this slice.** Rejected: ADR 0006 already deferred
  the runtime decision; the pure machines carry the spike.

## Failure modes

- Malformed line / oversized line → typed `AcpError`, connection closed,
  outstanding deliveries `Indeterminate` — never a panic, never a
  resync attempt.
- Unknown `sessionUpdate` kind → preserved event, stream continues
  (proven by the unknown-event fixture).
- Agent advertises `loadSession` but `session/load` errors → typed
  `Rejected`, surfaced; no retry loop.
- Agent dies mid-turn → `ProcessExited`, deliveries `Indeterminate`,
  cleanup decision issued (same path as explicit cancel minus the
  permission answers).
- Two agents racing in the smoke host → each agent runs in its own child
  with its own tracker; no shared mutable state.
- The published v1 schema drifts → the adapter models a subset with
  preserved passthrough, so unknown *fields* are ignored and unknown
  *kinds* survive; fixture tests pin the subset we depend on.

## Migration

P1 replaces the smoke-only subprocess host with a Core-owned harness
process behind the stable port, promotes mapped preserved events to real
`KnownEvent` kinds (one additive protocol change, unknown-event
preservation keeps old Desktops correct), and moves the line codec onto
the async transport selected by ADR 0006.

## Exit strategy

If ACP loses primacy (superseded by another agent protocol), the adapter
is deleted behind the stable harness port: domain and protocol crates
have no ACP types, the preserved-form events degrade gracefully, and the
`DeliveryState`/lifecycle machines are protocol-agnostic and reusable.

## Revisit triggers

- The ACP v1 schema adds a stable session/resume path superseding
  `session/load` (already visible as `sessionCapabilities.resume`).
- The Desktop needs streamed thoughts or plan rendering (promotes
  `Unmapped` rows of the table to real kinds).
- The reference crate's type coverage makes hand-modeling costlier than
  the dependency.
