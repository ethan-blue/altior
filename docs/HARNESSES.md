# Agent harness contract

## Planned paths and release status

1. **ACP Harness (first release)**: external process communicating through stable
   ACP v1.
2. **Codex App Server Harness (deferred)**: possible rich native adapter evaluated
   over local stdio only after ACP continuity is stable.
3. **Terminal Harness (deferred)**: CLI/TUI process represented as a
   terminal-backed thread.
4. **Native Harness (deferred)**: possible Altior-owned model/tool loop using the
   same domain events.

This follows the useful separation demonstrated by Zed: the application owns the
thread experience while an external agent may own its runtime, authentication,
models, tools, and native configuration.

Only ACP is a first-release implementation commitment. The other entries reserve
contract seams; placeholder adapters and empty product surfaces do not count as
progress.

## Aggregation boundary

Harnesses execute turns; they do not own the Altior agent. `AgentProfile`,
identity documents, memory, skills, permissions, tasks, schedules, and
parent/child operation records remain above every adapter in the Aggregated
Agent Runtime described by ADR 0002.

A thread records one active harness binding snapshot. The runtime may create
child threads on different harnesses for delegated work, but it may not silently
move an existing turn between harnesses. Cross-harness rebinding requires an
explicit policy or user choice and records whether the new provider session was
resumed or bridged from normalized context.

Every binding reports capabilities such as streaming, resume, steering,
cancellation, permissions, tool events, terminal events, usage, and child-agent
visibility. Routing and UI use these negotiated values, never an engine version
guess.

## Normalized event stream

Every harness maps output into stable Altior events such as:

- `turn.started`, `turn.completed`, `turn.failed`, `turn.cancelled`
- `message.delta`, `thought.delta`
- `tool.started`, `tool.updated`, `tool.completed`
- `permission.requested`, `permission.resolved`
- `terminal.started`, `terminal.output`, `terminal.exited`
- `usage.updated`, `plan.updated`

Unknown provider events remain available as bounded diagnostic payloads but may
not leak into the stable database schema.

## Delivery safety

- Every prompt has an Altior operation ID and one owning turn.
- Retry only before confirmed delivery or after a protocol-defined rejection.
- Connection loss after possible delivery is indeterminate and must not trigger
  automatic resend.
- Capability negotiation controls UI and behavior. Unsupported controls disappear
  or fail explicitly; no silent fallback to another model, mode, or agent.
- A multi-agent parent operation never weakens a child thread's permission,
  context, token, concurrency, or cancellation bounds.
- Parallel writers require isolated worktrees or an equally explicit resource
  ownership contract.

## ACP evolution

- Stable ACP v1 is the first production implementation.
- Draft versions compile behind explicit Cargo features.
- Protocol DTOs stay inside `altior-harness-acp`.
- Adapter compatibility tests use synthetic wire transcripts and real-agent smoke
  tests that are opt-in and never part of deterministic unit tests.

## Native protocol adapters

Native adapters are allowed when they expose useful capabilities beyond ACP, but
their DTOs stay inside their adapter crate. The first candidate is Codex
app-server because it exposes conversation history, approvals, and streamed
agent events through a documented bidirectional protocol. Its experimental
remote transports are not part of the initial production path.
