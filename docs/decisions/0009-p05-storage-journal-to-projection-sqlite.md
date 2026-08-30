# ADR 0009: SQLite journal-to-projection storage spike

Date: 2026-08-30 · Status: accepted · Scope: P0.5 storage spike
(docs/IMPLEMENTATION_PLAN.md), feeding P1.1 persistence.

## Context

`docs/ARCHITECTURE.md` fixes the durable-ownership rule this spike must
honor: **the event journal is authoritative for syncable knowledge
lifecycle; SQLite is authoritative only for device-local settings and
runtime state, and its syncable projections can always be rebuilt.**
P1.1 then needs append-safe local turn/event persistence and SQLite
projections with migrations, recovery markers, and projection rebuild.

P0.5 selects the storage foundation with a runnable spike instead of a
paper decision. What others do, and what we adopt or reject:

- **Forward-only migrations keyed by `PRAGMA user_version`** — the
  rusqlite_migration pattern. One atomic integer survives vacuums and
  backups; no bookkeeping table that can drift from the schema it
  describes. We adopt it.
- **Refuse, never migrate down** — when an older binary opens a newer
  schema (Android SQLite's classic hard failure, and every local-first
  app that ships downgrade paths eventually regrets them), we fail with
  a typed `SchemaTooNew` error instead of guessing. We adopt it.
- **Append-only enforcement at the database layer** — ledger-style
  tables guarded by `BEFORE UPDATE/DELETE` triggers, so a bug in caller
  code cannot mutate history. We adopt it.
- **WAL journal mode + `synchronous=NORMAL`** — the standard advice for
  local-first desktop apps (Linear, Figma-style local clients): the
  writer never blocks readers and a crash loses at most the last
  transaction, not the file.
- **Projections as derived state** — event-sourcing practice: the
  journal is truth, projections are caches with a recorded high-water
  mark, and any detected staleness triggers a rebuild by replay.

## Decision

### 1. New crate `altior-storage`, outside the contract crates

Same placement pattern as `altior-ipc` and `altior-acp` (ADR 0006,
0007): `altior-domain` and `altior-protocol` gain no new dependencies.
`altior-storage` depends on exactly `[altior-domain, altior-protocol,
rusqlite]` and is pinned by the
dependency-boundary test. SQLite is vendored via rusqlite's `bundled`
feature so gates need no system SQLite and stay hermetic.

### 2. Migrations: forward-only, `user_version`-keyed, one transaction each

`MIGRATIONS` is a slice of `(version, sql)` applied in order inside a
transaction; each step also stamps `PRAGMA user_version`. Reopening an
already-migrated database applies nothing. A database whose
`user_version` exceeds the highest known version fails
`StorageError::SchemaTooNew { found, supported }` — the file is left
untouched. There is no down migration, by design.

### 3. Journal: append-only, idempotent, bounded

`journal(seq AUTOINCREMENT, event_id UNIQUE, thread_id?, turn_id?,
sequence, kind, payload BLOB, occurred_at)` with `BEFORE UPDATE` and
`BEFORE DELETE` triggers raising `journal is append-only`. The payload
is the JSON encoding of the protocol `EventEnvelope` (round-trip is
tested), and `kind` is `EventBody::kind_name()` including `acp.*`
unknown kinds — unknown events journal exactly as they arrived
(ADR 0004 preservation). A defense-in-depth cap
(`JOURNAL_PAYLOAD_MAX`, 1 MiB, mirroring the ACP line cap) rejects
oversized payloads before insert with a typed error.

`append_event` is idempotent only when both `event_id` and encoded
payload are byte-identical: that case returns
`AppendOutcome::Duplicate { seq }`. Reusing an `event_id` for different
content returns typed `EventIdCollision`; it never aliases two facts.

### 4. Projections: replayed aggregates with a high-water mark

`thread_projection(thread_id PRIMARY KEY, event_count, first_seq,
last_seq, last_event_id, last_kind, updated_at)` is maintained
incrementally on append **and** rebuildable wholesale. The
`projection_state` records `journal_max_seq` and an independent
`projection_version` for fold semantics. SQLite `user_version`
describes physical DB schema only. `Store::open` compares both marker
fields against `MAX(journal.seq)` and the current fold version: any
mismatch (or a missing marker with a non-empty journal) triggers
`rebuild_projections()`,
which wipes and re-derives the projection tables from the journal in
one transaction. A stale or damaged projection therefore heals on the
next open without operator action.

Events without a `thread_id` (connection-scoped events) journal but do
not project; thread summaries are per-thread aggregates only.

### 5. Reader path for catch-up

`journal_records(after_seq, JournalLimit)` returns journal rows in seq
order. `JournalLimit` is unsigned and capped at 10,000, so SQLite never
receives a negative/unbounded `LIMIT`. This is the storage-side hook the IPC
retained-window replay (ADR 0006) and later snapshot paths can call;
the spike proves order and round-trip fidelity only.

### 6. Testing strategy: hermetic and deterministic

In-memory SQLite for logic tests; `tempfile` only where reopen
persistence matters. Timestamps come from fixture constants; no
sleeps, no wall clock assertions, no network. Rebuild equivalence is
proven by comparing full thread-summary snapshots before and after a
forced wipe + rebuild + reopen.

## Alternatives considered

- **`schema_migrations` table instead of `user_version`** — visible in
  tooling, but it can drift from reality and needs its own guarding.
  Rejected for the single-integer invariant.
- **sled / redb (pure-Rust embedded KV)** — avoids the C build, but no
  SQL projection layer and a smaller recovery/inspection ecosystem;
  SQLite's single-file copyability is operationally valuable. Rejected
  for now; revisit if build pain dominates.
- **An event-sourcing framework crate** — imports opinions we do not
  need for two tables and would blur the boundary contract. Rejected.
- **Chunked rebuild with persistent markers** — for very large
  journals a single-transaction replay holds the write lock too long.
  The spike keeps the simple atomic rebuild and names the chunked
  variant as the scaling path (see failure modes); journal sizes in P1
  remain small enough that this is acceptable.

## Failure modes

- **Older binary, newer schema**: hard `SchemaTooNew` refusal; the
  operator upgrades. Data is never silently mangled.
- **Trigger bypass** (`DROP TRIGGER` then `UPDATE`): possible for
  local code with the file; the threat model is accidental mutation by
  our own code, not an adversary with database access.
- **Projection drift not visible in the marker** (for example a
  partially deleted aggregate row with an intact high-water mark):
  undetected until an explicit rebuild. Mitigation in P1: projection
  checksums; named here so the gap is on record.
- **Rebuild cost**: `O(journal)` under one write transaction; grows
  with history. Acceptable at P1 scale; the chunked rebuild is the
  named escape hatch.
- **Payload bloat**: every envelope stored as JSON text bytes; compact
  binary framing is a later, schema-versioned change.
- **SQLite file corruption**: recover by copying the single file
  (backups are one `cp`); projections are re-derivable by definition.

## Migration

P1.1 adopts this crate as the persistence port implementation and adds
the real domain tables (turns, events, permissions, projects) as
additional forward-only migrations on top of version 1. The v1 schema
here is additive: nothing in P0 depends on it yet.

## Exit strategy

Swapping rusqlite for sqlx (async) or moving to another store touches
only `altior-storage`; the public API (`append_event`,
`thread_summaries`, `rebuild_projections`, `journal_records`) is the
seam. If the journal ever moves into a CRDT-backed store, the fold in
`rebuild_projections` is the single place that changes.

## Revisit triggers

- Journal length makes single-transaction rebuilds observable
  (multi-second) → chunked rebuild with persistent markers.
- Projection drift reports from the field → add projection checksums.
- Windows CI cannot build bundled SQLite → evaluate system-SQLite or a
  pure-Rust store.
