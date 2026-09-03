# ADR 0017: Long-term memory lifecycle, journal events, and ranked SQLite FTS5 retrieval

Date: 2026-09-03 · Status: accepted · Scope: P2.1 long-term memory lifecycle
and retrieval index (`docs/IMPLEMENTATION_PLAN.md`), feeding P2.2 agent memory
injection and P3 multi-device CRDT sync.

## Context

`docs/ARCHITECTURE.md` defines the durable ownership and state hierarchy:
**the event journal is authoritative for syncable knowledge lifecycle; SQLite
is authoritative for device-local projections, settings, and runtime state, and
all syncable projections can always be rebuilt from the journal.**

P1.1 (ADR 0013) established the `domain_journal` in `altior-storage` as the
canonical event log for domain entities (`thread`, `turn`, `permission`),
guaranteeing forward-only migrations and deterministic projection rebuilds.

P2.1 introduces long-term memory: an agent capability to preserve, update, and
retrieve cross-conversation knowledge, user preferences, project facts, and
inferred context across sessions. To satisfy Altior's privacy, safety, and sync
invariants:
1. **Journal-authoritative lifecycle**: Memory mutations must append durable,
   immutable domain events (`memory.proposed`, `memory.confirmed`,
   `memory.rejected`, `memory.superseded`, `memory.forgotten`, `memory.expired`)
   to `domain_journal`.
2. **Rebuildable projection & standalone FTS5**: The `memory` table and
   `memory_fts` full-text search index must be purely derived projections
   rebuildable from the event log via `rebuild_domain_projections`.
3. **Secret-shaped filtering (fail-closed)**: Candidate memories containing API keys,
   private keys, passwords, or credential tokens must be rejected before touching
   the journal or projection tables.
4. **Deterministic composite ranking**: Search must evaluate BM25 relevance, scope
   affinity, confidence, recency decay, and explicitness bonus with deterministic
   tie-breakers and explainable scoring.
5. **Caller-passed clocks**: All mutations and searches receive explicit timestamps
   for total testability and deterministic replay.
6. **Forward compatibility with P3 sync**: Forgetting preserves tombstone records;
   events are directly reusable as the sync wire format.

## Decision

### 1. Pure domain entity contracts in `altior-domain`

All memory types are pure Rust domain models in `altior_domain::entity`:
- `MemoryRecord`: Complete entity representation with `MemoryId`, `MemoryContent`,
  `MemoryScope`, `MemoryKind`, `MemoryState`, `confidence` (0..=100), `MemorySensitivity`,
  `MemorySource`, `MemoryProvenance`, `created_at`, `updated_at`, optional `expires_at`,
  and optional `superseded_by`.
- `MemoryDraft`: Construction-stage input payload used to propose, create, or correct
  memories before event appending.
- `MemoryScope`: Scoping hierarchy: `Global`, `Person(BoundedLabel)`, `Project(BoundedLabel)`,
  or `Thread(BoundedLabel)`. Supports kind and target string serialization (`global`, `person`,
  `project`, `thread`) and hierarchical matching (`matches_scope`).
- `MemoryKind`: Knowledge category: `fact`, `preference`, `project_context`,
  `decision`, `system_directive`.
- `MemoryState`: Lifecycle state: `candidate`, `confirmed`, `rejected`, `superseded`,
  `forgotten`, `expired`.
- `MemorySensitivity`: Data sensitivity level: `normal`, `sensitive`.
- `MemorySource`: Provenance origin: `explicit`, `inferred`.
- `MemoryProvenance`: Causality tracking containing optional `thread_id`, `turn_id`,
  and bounded `excerpt` (`MemoryExcerpt`, max 1024 bytes).
- `MemoryContent`: Bounded value object (1 to 4096 bytes) validated against empty strings.
- `MemoryId`: Typed identifier newtype with `mem_` prefix (minimum 16 chars body).
- `MemoryListLimit`: Validated unsigned pagination limit (1..=200, default 50).
- `MemorySearchLimit`: Validated unsigned pagination limit (1..=64, default 8).
- `MemoryCursor`: Composite cursor `(updated_at, memory_id)` for stable pagination.
- `MemoryHit` & `MemoryMatchExplanation`: Search hit container carrying the record
  alongside transparent explainability metrics (`matched_terms`, `fts_rank`, `scope_weight`,
  `confidence_score`, `recency_score`, `explicitness_bonus`, `total_score`, `why_selected`).

### 2. Six domain event kinds and payload JSON schemas

Memory lifecycle is driven by six authoritative `DomainEventKind` variants recorded
in `domain_journal`:

1. `memory.proposed`: Inferred candidate memory awaiting confirmation.
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_...",
     "content": "string (1..=4096 bytes)",
     "scope_kind": "global|person|project|thread",
     "scope_target": "string|null",
     "kind": "fact|preference|project_context|decision|system_directive",
     "state": "candidate",
     "confidence": 0-100,
     "sensitivity": "normal|sensitive",
     "source": "explicit|inferred",
     "explicit": true|false,
     "provenance_thread_id": "thr_...|null",
     "provenance_turn_id": "trn_...|null",
     "excerpt": "string|null (<=1024 bytes)",
     "expires_at": 1704067200000|null
   }
   ```
2. `memory.confirmed`: Direct creation or promotion of candidate to confirmed state.
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_...",
     "content": "string (optional if confirming candidate)",
     "scope_kind": "string (optional if confirming candidate)",
     "scope_target": "string|null",
     "kind": "string (optional)",
     "state": "confirmed",
     "confidence": 0-100,
     "sensitivity": "string (optional)",
     "source": "string (optional)",
     "explicit": true|false,
     "provenance_thread_id": "thr_...|null",
     "provenance_turn_id": "trn_...|null",
     "excerpt": "string|null",
     "expires_at": 1704067200000|null
   }
   ```
3. `memory.rejected`: Rejection of candidate memory with optional reason.
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_...",
     "reason": "string|null"
   }
   ```
4. `memory.superseded`: Atomic replacement during memory correction.
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_... (original)",
     "superseded_by": "mem_... (replacement)"
   }
   ```
5. `memory.forgotten`: User or policy requested deletion (durable tombstone).
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_..."
   }
   ```
6. `memory.expired`: Scheduled or lazy sweep transition to expired state.
   Payload JSON schema:
   ```json
   {
     "memory_id": "mem_..."
   }
   ```

### 3. Rebuildable projection schema and standalone FTS5 index (SCHEMA_V6)

Schema migration v6 introduces two SQLite tables:
1. `memory`: Strongly-typed relational projection of all historical memory records.
2. `memory_fts`: Dedicated standalone FTS5 full-text search virtual table indexed
   on `(memory_id UNINDEXED, content)`.

**Why standalone FTS5 instead of external-content (`content='memory'`)**:
External-content FTS5 tables in SQLite do not support selective indexing of subsets
of rows without maintaining shadow index consistency for all rows. In Altior's
memory architecture, only active, confirmed, non-superseded, non-forgotten memories
should populate the full-text search index. A standalone FTS5 table allows explicit,
transactionally controlled insertion and deletion of index terms matching the exact
lifecycle state of each memory, while avoiding desynchronization risks during row
updates or projection rebuilds.

**Projection Rebuild & Digest**:
`rebuild_domain_projections` wipes `memory` and `memory_fts` and replays the
`domain_journal` sequentially from sequence 1. `domain_projection_digest` includes
all `memory` table fields in deterministic primary key order (`memory_id ASC`),
guaranteeing bit-for-bit replay integrity across node restarts.

### 4. Deterministic composite ranking formula and weights

Search retrieval in `Store::search_memories` evaluates candidates using a multi-factor
scoring formula:

$$\text{total\_score} = (\text{text\_score} \times 0.40) + (\text{scope\_weight} \times 0.25) + (\text{confidence} \times 0.20) + (\text{recency} \times 0.10) + \text{explicit\_bonus}$$

Where:
- **`text_score`**: SQLite FTS5 `bm25(memory_fts)` rank transformed via $(-\text{raw\_bm25}).\text{max}(0.1)$.
- **`scope_weight`**:
  - Exact scope match: $1.5$
  - Global scope (fallback match): $1.0$
  - Other compatible scope: $0.8$
  - Unfiltered search: $1.0$
- **`confidence`**: Linear normalization $\frac{\text{confidence}}{100.0} \in [0.0, 1.0]$.
- **`recency`**: Half-life temporal decay $\frac{1.0}{1.0 + \text{age\_days} \times 0.05}$.
- **`explicit_bonus`**: $+0.2$ if `source.is_explicit()`, else $0.0$.
- **Tie-breakers**:
  1. `total_score DESC`
  2. `updated_at DESC`
  3. `memory_id DESC`

### 5. Secret-shaped content filtering (fail-closed policy)

To prevent accidental ingestion of credentials into persistent memory:
- `altior_domain::secret_shape::is_secret_shaped` scans candidate content and provenance
  excerpts against regex patterns covering:
  - PEM private keys (`BEGIN PRIVATE KEY`, `BEGIN RSA PRIVATE KEY`, etc.)
  - Provider API keys (OpenAI `sk-...`, Anthropic `sk-ant-...`, GitHub PATs/tokens `ghp_...`,
    `github_pat_...`, Slack tokens `xoxb-...`, AWS access keys `AKIA...`)
  - Password and bearer assignment heuristics (`password = ...`, `bearer ...`)
- **Fail-Closed Policy**: If secret shapes are detected in draft content or excerpt:
  - Return `StorageError::SecretShapedContent`.
  - Append zero events to `domain_journal`.
  - Write zero rows to `memory` or `memory_fts`.
- **False-Positive Tolerance**: Intentional false positives on token-like strings are
  acceptable because credentials must never enter persistent agent context.

### 6. Caller-passed clocks and atomic crash safety

- All mutation APIs (`propose_memory`, `create_memory`, `confirm_memory`, `reject_memory`,
  `correct_memory`, `forget_memory`, `sweep_expired_memories`) and search APIs
  (`search_memories`) accept an explicit `UnixMillis` timestamp. No wall-clock
  or non-deterministic system time is sampled inside the storage engine.
- Every state transition executes inside an atomic SQLite `IMMEDIATE` transaction,
  writing to `domain_journal`, updating `memory`, and synchronizing `memory_fts`
  simultaneously. Crash at any point leaves the database consistent.

## Alternatives Considered

1. **Embedding-based vector search only**:
   - *Rejected*: Pure vector search requires heavyweight runtime dependencies (ONNX/embedding
     models) and lacks deterministic exact-phrase keyword matching. FTS5 with BM25 and
     composite scoring provides lightweight, embedded, instant retrieval. Hybrid vector + FTS
     can be layered on top in future phases.
2. **Hard-deleting forgotten memories**:
   - *Rejected*: Hard deletion violates append-only event sourcing and breaks multi-device
     CRDT replication. Forgotten memories must append `memory.forgotten` tombstones to
     propagate deletions across nodes.
3. **External content FTS5 table**:
   - *Rejected*: External content FTS5 does not allow indexing a filtered subset (only
     confirmed active memories) without indexing superseded/forgotten records.

## Failure Modes & Resilience

- **Secret Leak Attempt**: Rejected in validation prior to transaction start. Database
  and journal remain pristine.
- **Out-of-Order Lifecycle State Transition**: Attempting to confirm/reject a non-candidate
  or correct an already superseded memory returns `StorageError::InvalidDomainEvent`
  and aborts.
- **FTS Query Syntax Injection**: User search terms containing quotes, parentheses, colons,
  or boolean keywords are parsed into alphanumeric tokens and quoted as literal phrases
  `"term"`, preventing syntax errors.
- **Lazy vs Eager Expiry**: Expired memories are instantly filtered out at query time by
  `expires_at > now`, preventing stale retrievals even if `sweep_expired_memories` has
  not yet been invoked.
- **Projection Corruption or Index Loss**: `Store::rebuild_domain_projections` replays the
  full `domain_journal` and restores complete memory relational and FTS state.

## Migration & Compatibility

- Stamped as schema version 6 (`PRAGMA user_version = 6`).
- Forward-only migration in `MIGRATIONS` array applying `SCHEMA_V6`.
- Downgrades from versions $>6$ are explicitly refused with `StorageError::SchemaTooNew`.
- Fully backward compatible with v1-v5 schemas.

## Sync Implications (P3)

- All memory lifecycle events in `domain_journal` are pure and self-contained.
- Memory events will serve directly as CRDT sync payload operations in P3 multi-device relay.
- Superseding and forgetting semantics map cleanly to distributed tombstone convergence.

## Exit Strategy

The memory table and FTS index are decoupled projections over `domain_journal`. If
the retrieval engine or ranking algorithm is replaced (e.g., embedding vector store or
BM25 weighting adjustment), the underlying event schema remains unchanged and projections
can be re-indexed arbitrarily.
