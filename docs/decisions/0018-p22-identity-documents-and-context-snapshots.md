# ADR 0018: Device-local identity documents, deterministic context snapshot assembly, and prompt injection

Date: 2026-09-05 · Status: accepted · Scope: P2.2 agent memory injection,
identity documents, and explainable context diagnostics (`docs/IMPLEMENTATION_PLAN.md`).

## Context

Altior separates durable state into two tiers (`docs/ARCHITECTURE.md`):
1. **The event journal**: authoritative for syncable domain knowledge lifecycle
   (threads, turns, permissions, long-term memory events).
2. **Device-local tables**: authoritative for device-local projections, settings,
   and operational explainability records.

P2.1 (ADR 0017) established the journal-backed memory lifecycle (`memory.proposed`,
`memory.confirmed`, etc.) and ranked FTS5 retrieval. P2.2 closes the loop by
injecting confirmed memory and user identity into agent turns above ACP, while
providing total diagnostic explainability to the user.

To preserve Altior's privacy, security, and deterministic reproducibility invariants:
1. **Journal Authoritativeness & Prompt Purity**: The `TurnStarted` event in
   `domain_journal` and the projected `turn` record MUST store the user's raw
   prompt verbatim. The augmented "wire prompt" containing identity framing and
   retrieved memory context is strictly an ephemeral harness artifact sent over
   the ACP boundary; it must never pollute the persistent conversation journal.
2. **Byte-for-Byte Passthrough**: When no identity documents are configured and no
   memories are selected (or memory is disabled), the wire prompt transmitted to
   the agent harness must be byte-for-byte identical to the user prompt. No framing,
   delimiters, metadata, or extra whitespace may be added.
3. **Device-Local Identity Documents**: User-authored profile declarations
   (name, about, preferences, standing instructions) are stored in a dedicated
   device-local SQLite table (`identity_document`). They are not domain journal
   events and do not sync across devices in P2 (P3 may introduce an explicit
   opt-in vault sync family).
4. **Per-Turn Explainability (`context_snapshot`)**: Every turn that assembles
   context writes an immutable `ContextSnapshot` row to SQLite recording token
   accounting, selected identity documents, selected memories with scoring and
   `why_selected` rationale, provenance pointers, and any dropped entries.
   Like identity documents, context snapshots are device-local and never participate
   in domain projection digests or rebuilds.
5. **Secret-Shaped Filtering (Fail-Closed)**: Identity documents and context payloads
   are strictly scanned for credentials via `is_secret_shaped`. Any match triggers
   an immediate fail-closed error with zero database writes.
6. **Deterministic Token Budgeting**: All token estimation, budgeting, selection,
   and rendering logic is pure and deterministic. Clocks are passed explicitly by
   callers.

## Errata: MemoryKind Invariants

ADR 0017 inadvertently referenced candidate knowledge categories (`project_context`,
`decision`, `system_directive`). As implemented, tested, and locked in
`altior-domain::entity::MemoryKind`, the authoritative, forward-compatible
knowledge categories are:
- `Fact` (`"fact"`): An established fact about the world, user, or project.
- `Preference` (`"preference"`): A user preference or stylistic choice.
- `Instruction` (`"instruction"`): An explicit user instruction or constraint.
- `Summary` (`"summary"`): A conversation or session summary.

All downstream components (ranking, budgeting, serialization, DTOs) adhere strictly
to these four variants.

## Decision

### 1. Device-Local Identity Documents (`identity_document`)

Stored in a dedicated SQLite table introduced in schema v7:
```sql
CREATE TABLE identity_document (
    identity_document_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX identity_document_kind_id ON identity_document(kind, identity_document_id);
```

- Bounded entity rules:
  - `identity_document_id`: Typed ID with prefix `idd_` (`IdentityDocumentId`).
  - `kind`: `IdentityDocumentKind` (`Name`, `About`, `Preference`, `Instruction`).
  - `content`: Non-empty, trimmed text capped at 4,096 bytes (`IDENTITY_CONTENT_MAX_BYTES`).
  - Device cap: At most 32 documents per device (`IDENTITY_DOCUMENT_COUNT_MAX`).
- Secret scanning: Any write containing secret-shaped tokens is rejected immediately
  with `StorageError::SecretShapedContent`.
- Projection isolation: `rebuild_domain_projections` and `domain_projection_digest`
  do not touch or hash `identity_document`.

### 2. Context Snapshot Audit Records (`context_snapshot`)

Stored in a dedicated SQLite table in schema v7:
```sql
CREATE TABLE context_snapshot (
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    memory_mode TEXT NOT NULL,
    passthrough INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL
) STRICT;

CREATE INDEX context_snapshot_thread_created
    ON context_snapshot(thread_id, created_at, turn_id);
```

- One audit row per turn: `turn_id` is the immutable primary key. Automatic resend
  of active/terminal turns is forbidden, and context assembly executes exactly
  once per admitted turn.
- Idempotency & Conflict: Writing a snapshot for an existing `turn_id` with an
  identical payload succeeds idempotently; writing with a conflicting payload returns
  `StorageError::ContextSnapshotConflict`.
- Bounded payload: Serialized JSON is capped at 64 KiB (`CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES`).
- Projection isolation: Context snapshots do not participate in domain rebuilds or digests.

### 3. Pure Context Assembly Pipeline (`altior-core::context`)

Context assembly is organized as a pure, side-effect-free pipeline:
1. **Token Estimator**: Deterministic character/word estimation providing consistent
   budget accounting across all platforms.
2. **Budget Allocator**:
   - `identity_limit_tokens`: Default 512 tokens.
   - `memory_limit_tokens`: Default 1024 tokens.
3. **Identity Ordering**: Sorted by rendering priority:
   `Name` (0) < `About` (1) < `Instruction` (2) < `Preference` (3),
   sub-ordered deterministically by `(created_at, identity_document_id)`.
4. **Memory Ranking & Drop Accounting**:
   - Candidates retrieved via `Store::search_memories(user_prompt)`.
   - Admitted in rank order until `memory_limit_tokens` is reached.
   - Entries exceeding the budget are recorded in `dropped` with
     `reason: ContextDropReason::BudgetExhausted`.
5. **Prompt Assembly & Passthrough**:
   - If no identity documents and no memories are admitted:
     `wire_prompt == user_prompt` (exact byte equality).
     `ContextSnapshot.passthrough == true`.
   - Otherwise, framed blocks are rendered above the user prompt:
     ```text
     [User Identity]
     - Name: ...
     - Preference: ...

     [Relevant Context]
     - [Fact] ...

     <user prompt>
     ```

### 4. Integration Boundary in `CoreApplication::start_prompt_envelope`

In `CoreApplication::start_prompt_envelope`:
1. Thread and AgentProfile are looked up.
2. If `AgentProfile.memory_mode == MemoryMode::LongTerm`, `store.search_memories`
   is queried with the user's prompt text.
3. Active identity documents are loaded via `store.list_identity_documents`.
4. Context is assembled into `(wire_prompt, context_snapshot)`.
5. `TurnStarted` event is committed to `domain_journal` containing the **raw user prompt**.
6. `store.record_context_snapshot(&snapshot)` commits the audit record. If this fails,
   the turn fails closed and no prompt is dispatched.
7. `self.supervisor.prompt` receives the `wire_prompt`.

## Alternatives Considered

1. **Storing wire prompt in `TurnStarted` journal event**:
   - *Rejected*: Violates user privacy and sync efficiency. Syncing a thread across
     devices should sync the user's actual conversation, not repetitive device-local
     system prompt framings.
2. **Implicit in-memory context diagnostics**:
   - *Rejected*: Ephemeral diagnostics are lost on desktop restart and cannot be
     audited later. Storing `context_snapshot` enables post-hoc inspection of why
     an agent gave a specific answer.

## Consequences

- Clean separation between syncable domain truth and local agent context framing.
- Users have full transparency into what context was sent, why each memory was
  selected, and what was dropped due to token limits.
- Zero credential leakage into persistent storage or external harnesses.
