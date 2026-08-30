# ADR 0013: Domain entities, domain event journal, and SQLite persistence

Date: 2026-08-30 · Status: accepted · Scope: P1.1 domain and persistence
(`docs/IMPLEMENTATION_PLAN.md`), feeding P1.2 ACP runtime and P1.3 Desktop MVP.

## Context

`docs/ARCHITECTURE.md` establishes the durable-ownership rule that governs
all Altior state: **the event journal is authoritative for syncable
knowledge lifecycle; SQLite is authoritative for device-local settings and
runtime state, and its syncable projections can always be rebuilt from the
journal.**

P0.5 implemented the storage foundation spike (ADR 0009), demonstrating
forward-only migrations keyed by `PRAGMA user_version`, database-enforced
append-only triggers, incremental and wholesale thread summary projections,
and marker-based self-healing on reopen. However, that spike operated solely
on transport-level protocol envelopes (`altior_protocol::EventEnvelope`).

P1.1 delivers the production domain layer and persistence contracts:
1. Pure, infrastructure-independent domain entity models, value objects,
   identifiers, query limits, and cursors in `altior-domain`.
2. A durable `domain_journal` in `altior-storage` acting as the authoritative
   event source for domain entities, decoupled from IPC protocol envelopes.
3. Rebuildable SQLite projections for threads, turns, and permissions,
   including full-text search over thread titles.
4. Device-local CRUD authority for agent profiles, harness bindings, and
   project references, protected by atomic `IMMEDIATE` transactions.
5. In-transaction domain lifecycle validation, durable collision fail-closed
   semantics, and deterministic projection integrity verification.
6. Bounded queries, literal FTS query escaping, and compound indexing.

## Decision

### 1. Pure domain entity contracts in `altior-domain`

All domain records are pure Rust structs and enums free from `rusqlite`,
ACP wire framing, or IPC protocol dependencies:

- **8 core domain entity types**:
  - `AgentProfile`: Altior-owned agent persona with display name, preferred
    harness (`acp`, `terminal`, `native`), memory mode (`off`, `session`,
    `long_term`), and created/updated timestamps.
  - `AcpHarnessBinding`: Launch configuration binding an agent profile to an
    executable command path with a label.
  - `Thread`: Conversation context with agent profile reference, bounded title,
    lifecycle state (`Open`, `Pinned`, `Archived`), optional project reference,
    event counter, and timestamps.
  - `Turn`: Execution step within a thread with lifecycle state (`Active`,
    `Completed`, `Cancelled`, `Failed`), prompt delivery classification
    (`Absent`, `Confirmed`, `Rejected`, `Indeterminate`), optional `operation_id`,
    and start/end timestamps.
  - `Permission`: Request and approval state within a turn (`Pending`,
    `Approved`, `Denied`) across permission kinds (`execute`, `read`, `write`,
    `network`) with a bounded description and requested/decided timestamps.
  - `ProjectRef`: Device-local filesystem path association with a label.
  - `DomainEvent`: Pure event record combining `EventId`, optional `ThreadId`,
    `TurnId`, and `OperationId`, `DomainEventKind`, `EventPayload`, and
    `UnixMillis` timestamp.
  - `EventPayload`: Bounded payload byte buffer validated at construction
    (capped at `JOURNAL_PAYLOAD_MAX` = 1 MiB).
- **Identifier newtypes**: Kind-prefixed, strongly-typed identifiers parsed
  and formatted via `altior-domain::id`: `AgentProfileId` (`agp_`),
  `HarnessBindingId` (`hsb_`), `ThreadId` (`thr_`), `TurnId` (`trn_`),
  `OperationId` (`op_`), `EventId` (`evt_`), `CoreInstanceId` (`cor_`), and
  `ProjectId` (`prj_`).
- **Bounded value objects**: Construction-validated wrappers preventing
  unbounded strings and injection:
  - `DisplayName`: 1 to 256 bytes, non-empty after trimming.
  - `ThreadTitle`: 0 to 512 bytes, supporting explicit empty untitled threads
    (`ThreadTitle::UNTITLED`).
  - `SearchQuery`: 1 to 256 bytes, non-empty after trimming.
  - `BoundedLabel`: 1 to 256 bytes, non-empty after trimming.
  - `BoundedPath`: 1 to 4096 bytes, non-empty after trimming.
  - `PermissionDescription`: 0 to 4096 bytes.
- **Validated unsigned pagination limits**:
  - `ThreadListLimit`: 1 to 200 (`THREAD_LIST_LIMIT_MAX`).
  - `HistoryLimit`: 1 to 500 (`HISTORY_LIMIT_MAX`).
  - `TurnListLimit`: 1 to 500 (`TURN_LIST_LIMIT_MAX`).
  - `AgentProfileListLimit`: 1 to 200 (`AGENT_PROFILE_LIST_LIMIT_MAX`).
  - `HarnessBindingListLimit`: 1 to 200 (`HARNESS_BINDING_LIST_LIMIT_MAX`).
  - `ProjectRefListLimit`: 1 to 200 (`PROJECT_REF_LIST_LIMIT_MAX`).
  - `PermissionListLimit`: 1 to 500 (`PERMISSION_LIST_LIMIT_MAX`).
- **Deterministic composite cursors**:
  - `ThreadCursor`: `(updated_at, thread_id)` for newest-first pagination.
  - `AgentProfileCursor`: `(updated_at, agent_profile_id)`.
  - `HarnessBindingCursor`: `(created_at, harness_binding_id)`.
  - `ProjectRefCursor`: `(created_at, project_id)`.
  - `PermissionCursor`: `(requested_at, event_id)`.
  - `TurnCursor`: `(started_at, turn_id)` for chronological turn pagination.

### 2. Dual journal architecture & decoupled replay authority

Persistence maintains two distinct journals for clear separation of concerns:
- `journal` (v1 schema): Retained for transport-level protocol replay of
  `altior_protocol::EventEnvelope` payloads.
- `domain_journal` (v2 schema): The durable, authoritative event log for all
  domain entities and rebuildable projections (`thread`, `turn`, `permission`).

**Decoupled replay authority**: Replaying the `domain_journal` into projection
tables validates structural event invariants, thread existence, and turn states,
but **does not require the referenced `AgentProfile` or `ProjectRef` to exist in
the device-local CRUD tables**. When an encrypted personal vault is synchronized
to a new device, thread and turn history can be fully reconstructed and browsed
offline before or independently of local agent bindings or path mappings.

### 3. Forward-only schema migrations (v1 → v2 → v3)

Schema migrations are strictly forward-only, keyed by `PRAGMA user_version`,
and executed inside atomic transactions:
- **Schema v1**: Creates `journal`, `thread_projection`, and `projection_state`
  (ADR 0009).
- **Schema v2**: Creates `domain_journal` with append-only update/delete triggers,
  projection tables `thread`, `turn`, and `permission`, device-local CRUD tables
  `agent_profile`, `harness_binding`, and `project_ref`, the recovery marker table
  `domain_projection_state`, and the FTS5 virtual table `thread_search` with
  synchronization triggers.
- **Schema v3**: Alters `domain_projection_state` to add `projection_digest TEXT
  NOT NULL DEFAULT ''`, enabling cryptographic tamper detection of projection
  caches.

### 4. Domain event validation, durable collisions, and lifecycle state machines

Every `append_domain_event` executes within an atomic SQLite `IMMEDIATE`
transaction:

1. **Durable tuple collision detection**:
   When an incoming `event_id` already exists in `domain_journal`, storage
   compares all 7 durable fields: `(event_id, thread_id, turn_id, operation_id,
   kind, payload, occurred_at)`. If byte-identical, append succeeds idempotently
   returning `AppendOutcome::Duplicate { seq }`. Any discrepancy fails closed
   with `StorageError::EventIdCollision`.
2. **In-transaction domain lifecycle validation**:
   Before insertion, `validate_domain_event_in_tx` enforces domain rules:
   - `ThreadCreated`: Requires valid `AgentProfileId` syntax, optional valid
     `ProjectId` syntax, and title within capacity. Must not have turn scope.
   - `ThreadTitleChanged`: Requires thread existence; updates title in projection
     and FTS. Empty titles (`ThreadTitle::UNTITLED`) clear the title cleanly.
   - `ThreadStateChanged`: Enforces legal states (`open`, `pinned`, `archived`).
   - `TurnStarted`: Requires thread existence and turn ID uniqueness. Stored
     `operation_id` is reconstructed during rebuild.
   - `TurnCompleted`, `TurnCancelled`, `TurnFailed`: Require matching thread and
     an `active` turn.
   - `PermissionRequested`: Requires `active` turn, valid permission kind
     (`execute`, `read`, `write`, `network`), and non-empty description.
   - `PermissionDecided`: Requires `active` turn, matching pending permission,
     and legal decision (`approved`, `denied`).
   - `MessageDelta`: Requires `active` turn; increments turn event counter and
     thread activity.
   - `Other(kind)`: Custom provider/extension events. Scoped without turn: if
     `thread_id` is None, it journals globally without projection; if
     `thread_id` is present, it validates thread existence and updates thread
     activity without requiring turn scope.

### 5. Device-local CRUD authority with atomic `IMMEDIATE` transactions

`AgentProfile`, `AcpHarnessBinding`, and `ProjectRef` represent device-local
configuration:
- All create, update, upsert, and delete operations use `IMMEDIATE` transactions
  to prevent concurrent write races.
- **Same-content idempotency**: Calling create with an existing ID and identical
  fields succeeds idempotently; conflicting data fails with typed errors
  (`AgentProfileAlreadyExists`, `HarnessBindingAlreadyExists`,
  `ProjectRefAlreadyExists`).
- **Immutable field protection**: `created_at` cannot be mutated; `updated_at`
  must not precede `created_at`.
- **Foreign key integrity**: `AcpHarnessBinding` validates that the referenced
  `AgentProfileId` exists locally.
- **Referential deletion check**: Deleting a `ProjectRef` inspects the projected
  `thread` table inside an `IMMEDIATE` transaction; if any thread references the
  project ID, deletion is refused with `StorageError::ProjectReferencedByThreads`.

### 6. External-content FTS5 title search, literal escaping, and integrity checks

Thread search uses SQLite FTS5 configured as an external-content table:
- `thread_search` indexes `thread(rowid, thread_id, title)`. Content is stored
  only in `thread`, avoiding duplicate storage.
- Title changes, insertions, and deletions are synchronized automatically via
  triggers (`AFTER INSERT`, `AFTER DELETE`, `AFTER UPDATE OF title`).
- **Literal phrase escaping**: User queries are sanitized via
  `fts5_quoted_literal`, which wraps the query in double quotes and escapes inner
  quotes (`""`). This prevents FTS5 syntax errors, column filters, or operator
  injections on arbitrary user input.
- **Official integrity verification**: Consistency between the FTS5 index and
  the `thread` projection is validated via SQLite's official command:
  `INSERT INTO thread_search(thread_search, rank) VALUES('integrity-check', 1)`.

### 7. Business projection digest and self-healing projection lifecycle

To detect cache corruption or out-of-band modifications that do not alter the
journal sequence high-water mark:
- **Deterministic digest algorithm**: `domain_projection_digest` computes a
  64-bit streaming FNV-1a hash over all projected rows in `thread`, `turn`, and
  `permission`, ordered strictly by primary keys.
- **Structured length-prefixed framing**: Values are encoded with explicit type
  tags (`T` for thread, `U` for turn, `P` for permission) and length-prefixed
  strings/integers, eliminating delimiter collisions.
- **FTS shadow table exclusion**: The digest operates exclusively on business
  projection tables, completely avoiding SQLite FTS internal shadow tables
  (`thread_search_data`, `thread_search_idx`, etc.) which vary across SQLite
  builds and page vacuum operations.
- **Safe rebuild sequence**:
  1. Preflight FTS repair: Executes `INSERT INTO thread_search(thread_search)
     VALUES('rebuild')` before clearing projections, ensuring stale FTS state
     does not trigger delete errors during projection wipe.
  2. Projection wipe: Deletes all rows in `permission`, `turn`, and `thread`.
  3. Journal replay: Sequentially folds all `domain_journal` rows, reconstructing
     `Thread`, `Turn`, `Permission`, and `operation_id` fields.
  4. Postflight FTS rebuild: Runs `thread_search('rebuild')` against the freshly
     replayed `thread` table.
  5. Digest calculation & marker update: Computes the business projection digest
     and records `(journal_max_seq, DOMAIN_PROJECTION_VERSION, rebuilt_at,
     projection_digest)` in `domain_projection_state`.
- **Reopen self-healing**: On `Store::open`, `ensure_domain_projections_current`
  validates that the stored marker matches `MAX(domain_journal.seq)`, the fold
  version equals `DOMAIN_PROJECTION_VERSION`, the stored digest matches the live
  calculated digest, and the official FTS integrity check succeeds. Any failure
  triggers an immediate single-transaction rebuild.

### 8. Bounded queries, composite cursors, and compound index strategy

All storage reader APIs enforce bounded limits and stable cursor ordering:
- `threads(state, before, limit)`: Paginated by `(updated_at DESC, thread_id ASC)`.
- `turns_for_thread(thread_id, after, limit)`: Paginated by `(started_at ASC, turn_id ASC)`.
- `thread_history(thread_id, after_seq, limit)`: Paginated by `seq ASC`.
- `search_threads(query, before, limit)`: Ordered by `(updated_at DESC, thread_id ASC)`.
- `permissions_for_thread` / `permissions_for_turn`: Paginated by `(requested_at ASC, event_id ASC)`.
- `agent_profiles`, `harness_bindings_for_agent`, `project_refs`: Paginated by timestamp + ID tie-breaker.

**Compound indexes**:
- `domain_journal(thread_id, seq)`: Accelerated thread history queries.
- `domain_journal(turn_id, seq)`: Accelerated turn-scoped replay.
- `thread(updated_at)` & `thread(state)`: Fast thread listing and state filters.
- `turn(thread_id, started_at, turn_id)`: Index-backed turn pagination.
- `permission(thread_id, requested_at, event_id)` & `permission(turn_id, requested_at, event_id)`: Index-backed permission queries.
- `harness_binding(agent_profile_id)`: Fast harness resolution by agent.

## Alternatives considered

- **Single unified journal for IPC and domain entities**: Rejected. The IPC
  journal stores wire envelopes (`EventEnvelope`) which evolve with protocol
  transports, whereas the domain journal stores pure domain event records.
  Coupling them would compromise the independence of domain contracts.
- **Hashing SQLite FTS shadow tables for integrity check**: Rejected. FTS5
  internal shadow tables (`thread_search_data`, `thread_search_idx`) depend on
  SQLite B-tree balancing, page layouts, and version-specific internals. Hashing
  business projection tables directly is deterministic and portable across
  platforms.
- **Strict local foreign keys during domain journal replay**: Rejected. Enforcing
  that local `AgentProfile` or `ProjectRef` rows exist before replaying a
  `ThreadCreated` event would break vault synchronization when history is synced
  to a device before local settings are configured.
- **Downgrade migrations**: Rejected (fail-closed `SchemaTooNew` invariant from
  ADR 0009).
- **Asynchronous background projection rebuild**: Rejected. Rebuild runs
  synchronously in one atomic transaction on open or append catch-up, preventing
  race conditions and partial-read anomalies.

## Failure modes

- **Corrupted or modified projection cache**: Stored projection digest mismatches
  live digest on open; `Store::open` transparently wipes and rebuilds projections
  from `domain_journal`.
- **FTS index out of sync with thread table**: Preflight FTS rebuild prevents
  trigger failures; post-rebuild official integrity check verifies index parity.
- **Event ID collision with differing payload**: In-transaction durable tuple
  comparison detects discrepancy and rejects append with `StorageError::EventIdCollision`.
- **Illegal turn or permission lifecycle transition**: In-transaction validation
  fails closed with `StorageError::InvalidDomainEvent`, leaving database state
  unmodified.
- **Project reference deleted while referenced by threads**: Refused with
  `StorageError::ProjectReferencedByThreads`.
- **Newer schema version opened by older binary**: Refused with typed
  `StorageError::SchemaTooNew`.

## Migration

- Existing databases migrate forward through `MIGRATIONS` (`SCHEMA_V1` →
  `SCHEMA_V2` → `SCHEMA_V3`).
- Existing v1 protocol journal data is preserved untouched.
- `altior-domain` and `altior-storage` provide complete P1.1 domain contracts,
  enabling P1.2 ACP runtime implementation.

## Exit

- All 8 domain entity classes, bounded value objects, pagination limits, and
  composite cursors implemented and tested in `altior-domain`.
- Domain journal, schema migrations v1→v2→v3, projection rebuild, business
  projection digest, literal FTS5 search, and device-local CRUD implemented
  and tested in `altior-storage`.
- P1.2 ACP runtime (subprocess trees, boundary adapter checkpoints, OS secret
  resolution, and live IPC integration) is the next milestone and has not yet
  been implemented.

## Revisit triggers

- Very large journal histories (>100k events) where single-transaction replay
  exceeds acceptable open latency, requiring chunked background rebuild.
- Multi-device sync (P3) introducing encrypted vault envelopes, vector memory
  indices (P2), or CRDT document persistence.

## Acceptance evidence

- **67 domain integration tests** (`crates/altior-storage/tests/domain.rs`):
  - Forward-only v1→v2→v3 schema migration and v1 journal preservation.
  - Domain event append, same-content idempotency, and 7-field durable tuple collision checks.
  - Thread lifecycle (create, title change, state change, title clear to empty).
  - Turn lifecycle (start, complete, cancel, fail, terminal state rejection of deltas and permissions).
  - Permission lifecycle (request, pending validation, approve, deny).
  - `Other` custom kind scoping (global without thread/turn vs thread-scoped).
  - Replay decoupling (journal replays without local profile or project pre-existence).
  - Atomic `IMMEDIATE` CRUD for agent profiles, harness bindings, and project references.
  - Safe `ProjectRef` deletion rejection when referenced by threads.
  - Bounded pagination and cursor stability across threads, turns, history, permissions, and CRUD entities.
  - Literal FTS5 title search, special character query escaping, and official `rank='integrity-check', 1` verification.
  - Projection self-healing on reopen against missing markers, stale fold versions, and tampered projection digests.
- **13 protocol journal tests** (`crates/altior-storage/tests/journal.rs`):
  - Protocol envelope round-trips, trigger-enforced append-only immutability, duplicate append idempotency, envelope collision detection, rebuild equivalence, and reopen self-healing.
- **Rust gates**: `cargo check`, `cargo test`, and `cargo clippy` pass cleanly across default and `--all-features`.
- **Desktop gates**: Desktop 39 test suite and strict TypeScript check pass cleanly.
