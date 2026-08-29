# ADR 0002: Aggregate agent runtimes above replaceable harnesses

- Status: Accepted
- Date: 2026-08-29

## Context

Altior must let one person use several agent engines without splitting identity,
memory, skills, permissions, tasks, or conversation history into provider-owned
silos. ACP is the first interoperability path, but useful engines also expose
native integration surfaces. Codex app-server, for example, exposes threads,
turns, approvals, and streamed events through a bidirectional protocol.

Reference products demonstrate complementary strengths:

- Lody demonstrates ACP lifecycle management, capability negotiation, durable
  operations, process supervision, worktree isolation, and race-focused tests.
- Cumora demonstrates persistent engine sessions, wake coalescing, stale-output
  barriers, stream reducers, and multi-agent coordination.
- Alma demonstrates a local-first provider hub with memory, skills, tools,
  references, persistent tasks, and user-controlled extension points.
- Codex demonstrates an open agent harness, app-server integration, normalized
  thread/turn/item streams, approvals, sandboxing, and inspectable subagent work.

Their product models cannot be adopted wholesale. Lody and Cumora are organized
around shared workspaces or teams, Alma's product runtime is not Altior's domain
model, and Codex remains one coding harness rather than the owner of personal
knowledge.

## Decision

Altior will provide an **Aggregated Agent Runtime** in `altior-core`. It owns the
stable personal-agent experience and delegates execution to replaceable harness
adapters.

The domain distinguishes these concepts:

- `AgentProfile`: an Altior-owned identity and behavior profile. It references
  personality, memory policy, skill policy, permission defaults, and preferred
  harness bindings, but contains no provider credentials.
- `HarnessBinding`: a device-local launch binding for one engine and adapter.
  It records negotiated capabilities and opaque secret references.
- `Thread`: an Altior conversation with one active binding snapshot and optional
  provider session identity.
- `Turn`: one delivery-safe unit of user or agent work.
- `Operation`: a durable parent/child coordination record for delegated work.
- `ContextSnapshot`: the bounded, provenance-marked context selected for a turn.

```text
Agent Profile + Personal Vault context
                  |
         Aggregated Agent Runtime
          |       |       |       |
         ACP   Codex AS  Terminal  Native
          |       |       |       |
       external execution engines and model providers
```

Routing is explicit and deterministic:

1. A thread's recorded binding wins.
2. Creating a thread may use an explicitly selected binding or a recorded
   profile preference that satisfies required capabilities.
3. Unavailability never silently changes the agent, model, permission mode, or
   execution location.
4. A user-approved rebind creates a new binding snapshot. If the prior harness
   cannot resume, Altior starts a new provider session from a bounded normalized
   transcript and records that bridge in provenance.
5. Indeterminate delivery never triggers failover or automatic resend.

Multi-agent work uses ordinary child threads, not a separate team domain. A
parent operation may start bounded child threads on different harnesses, collect
their status and result artifacts, and return cited summaries to the parent.
Each child has independent context, permissions, budgets, cancellation, and
delivery identity. Parallel write work additionally requires isolated worktrees
or another explicit conflict boundary.

All adapters translate into the normalized Altior event stream. Provider-native
events may be retained only as bounded device-local diagnostics. Capability
support is negotiated and recorded; version strings are not capability claims.

## Integration order

1. ACP v1 remains the first production adapter and compatibility baseline.
2. The first release ships no second production harness.
3. After ACP continuity is stable, Terminal and Codex app-server may be evaluated
   without changing domain records.
4. Bounded multi-agent coordination follows stable single-thread delivery.
5. The Altior-native harness remains optional and uses the same contracts.

Any later Codex app-server spike starts with local stdio and keeps all Codex DTOs
inside its adapter crate. Its production adoption requires a separate acceptance
decision.

## Consequences

- One personal identity and memory layer can span multiple engines.
- Rich native capabilities are available without making a provider protocol the
  Altior domain model.
- Cross-engine resume is honest about context bridging rather than pretending
  provider sessions are interchangeable.
- Multi-agent coordination can reuse thread, turn, permission, event, and
  synchronization contracts.
- The coordinator and operation journal add implementation work and require
  deterministic crash, replay, cancellation, and concurrency tests.

## Source and provenance policy

Reference behavior may be reimplemented from documented contracts and observed
tests. Source reuse is never implicit:

- Lody is Apache-2.0 and requires attribution and a provenance note for reused
  code.
- Cumora is MIT and requires preservation of its license notice for copied
  substantial portions.
- Codex component reuse must follow the license in the exact upstream component.
- Alma is currently a product/behavior reference only unless a specific source
  distribution and license are verified.

Any source reuse is recorded in `THIRD_PARTY_NOTICES.md` before merge.

## Revisit when

- ACP exposes every required rich-client capability with stable semantics.
- A native adapter cannot preserve Altior's delivery or permission guarantees.
- Cross-engine context bridges prove too lossy for an acceptable user experience.
- Operation coordination needs a domain contract beyond parent/child threads.
