# ADR 0010: CRDT `SyncDocumentEngine` bake-off — Automerge selected, Loro retained

Date: 2026-08-30 · Status: accepted · Scope: P0.5 CRDT spike
(docs/IMPLEMENTATION_PLAN.md), feeding P1 concurrent knowledge-document
edits.

## Context

`docs/ARCHITECTURE.md` names `SyncDocumentEngine` as the port for
concurrent knowledge-document edits synced between devices. P0.5 must
pick the backing CRDT with a runnable bake-off, not a paper comparison.
Both candidates implement the same text-sequence + LWW-scalar model;
the spike races them under identical deterministic schedules:

- **Loro 1.13.9** — name-addressed root containers, active development,
  text-editing-optimized; its own snapshot/updates export modes.
- **Automerge 0.11.0** — the established choice (used in production by
  several collaborative editors), columnar-compressed state, and a
  mature purpose-built delta-sync protocol for unreliable transports.

What the spike measures and how others' experience shaped it:

- **Adversarial convergence, not happy paths** — the property-test
  practice of both libraries' own test suites: same-offset concurrent
  inserts, delete-vs-insert races, merge idempotence and order
  independence, star topologies under a seeded schedule, stale forks
  catching up. Both libraries' histories include exactly these bugs.
- **Encoded state size on a fixed workload** — the number a relay-era
  sync budget actually sees; Automerge's RLE-columnar format vs Loro's
  snapshot encoding is the documented size difference.
- **Opaque state exchange** — the relay (ADR 0012 direction) moves
  bytes it cannot read; sync therefore must work through
  engine-opaque blobs (`export_state`/`import_state`), which is also
  the E2E-encryption-friendly shape.
- **Deterministic replica identities** — both libraries' defaults
  derive replica ids from randomness, which leaks into encoded state
  and tie-breaks. The spike fixes peer/actor ids explicitly
  (`set_peer_id`, `set_actor`) so every schedule is reproducible.

### Measured results (fixed 1000-op script, seed `0x000b_0ca1`)

4 text fields, 8 LWW scalars, ~60% insert / 20% delete / 20% set.
The hardened figures include Altior framing and collision-safe names:

| metric                       | loro 1.13.9 | automerge 0.11.0 |
|------------------------------|-------------|------------------|
| encoded state after 1000 ops | 14804 bytes | 10150 bytes      |
| view (all fields)            | identical   | identical        |
| encoded bytes, rerun         | identical   | identical        |

All 7 adversarial scenarios pass on **both** engines (identical view
digests across all replicas in every topology), and the sequential
script produces byte-identical views across engines.

## Decision

### 1. New crate `altior-crdt`, outside the contract crates

Same placement pattern as ADR 0009: `altior-domain` and
`altior-protocol` gain no dependencies. `altior-crdt` depends on
exactly `[automerge, loro]` and is pinned by the dependency-boundary
test. Neither contract crate learns a CRDT exists.

### 2. The port: object-safe `SyncDocumentEngine`, engines behind `AnyEngine`

The trait exposes only object-safe operations (typed-result
insert/delete text at char indices with clamping, typed-result LWW
scalar set, canonical sorted `view()`, opaque framed
`export_state`/`import_state`). Constructors and fork semantics
(`with_schema`, `fork_with_peer`) return concrete or enum types because
`Self`-returning methods break dyn compatibility — they live on the
engines and on the `AnyEngine` enum, which forwards the trait. The
canonical view (`Vec<(String, FieldView)>`, sorted) is what
convergence means: two replicas converge iff their views are equal.

### 3. Automerge is the selected engine for P1

- **~31% smaller encoded state on the measured workload** (10150 vs
  14804 bytes), and columnar compression scales better with history.
- **A mature, wire-level delta sync protocol** (`automerge-sync`'s
  `SyncMessage`) designed for lossy, unordered transports — the exact
  P1 relay model. The spike proves state-based exchange; the protocol
  adoption is a contained, additive step.
- **Passes every adversarial case** the spike throws at it, matching
  Loro.
- The encoding determinism holds (fixed actor ids), so CI can assert
  exact sizes without flakiness.

Loro stays in-tree as the retained second implementation: the
adversarial suite keeps racing both engines, so any library-level
regression in either shows up as a failing invariant rather than as a
field report. The port, not the library, is the contract.

### 4. Schema-first container creation (the Automerge footgun we found)

Creating a container under a map key is itself a map write: two
replicas that concurrently create the *same* text field resolve one
side away — that field's writes silently vanish. Loro's name-addressed
root containers are immune. The port therefore encodes the discipline:
`DocumentSchema` declares text and scalar fields before construction;
`with_schema` pre-creates every text container *before* any replica
forks. Empty, duplicate, conflicting, undeclared, and wrong-kind field
access returns `CrdtError`. Logical names are encoded into private
engine namespaces, so Loro's former `fields` root and Automerge's
former `s:` prefix cannot collide with user fields.

Exported state has an Altior magic/version, engine tag, schema digest,
and payload length. Import is capped at 8 MiB, validates framing and a
temporary engine document, and returns typed errors for malformed,
cross-engine, or cross-schema bytes instead of panicking.

### 5. Deterministic replica identities

`LoroEngine::new` fixes `peer_id(1)`; `fork_with_peer(peer)` assigns
distinct fixed ids; `AutomergeEngine::fork_with_peer` clones and sets
`ActorId` from the peer integer (never `Automerge::fork`, whose random
actor makes tie-breaks and encoded bytes irreproducible). Production
may use random ids — the port keeps them deterministic so gates stay
reproducible.

### 6. Version pins and MSRV

`automerge 0.7.x` does not currently compile against its transitive
`hexane` (upstream API skew), which is why 0.11.0 is pinned and
`Cargo.lock` is committed — the lockfile is load-bearing for supply
chain stability, not a hygiene nicety. Automerge 0.11 requires Rust
1.90; the workspace `rust-version` moves 1.85 → 1.90 to state that
honestly (the toolchain is current stable). Loro is pinned at 1.13.9
for the same reason.

## Alternatives considered

- **Selecting Loro instead** — immune to the container-creation race
  and very fast in public benchmarks, but larger encoded state on the
  measured workload and a younger sync ecosystem. Retained as the
  always-tested second engine instead; the port makes the swap a
  one-place change.
- **Yjs/Yrs** — excellent text editing (YATA), but this is a Rust
  workspace and the ecosystem's center of gravity for Rust delta sync
  is automerge-sync. Not raced; name recorded for completeness.
- **Operation-based sync instead of state-based** — smaller messages,
  but requires exactly-once ordered delivery and per-replica
  send-queues; the relay model is at-least-once opaque payloads.
  State-based (and automerge's delta protocol later) matches.
- **Keeping one engine only** — halves the test surface but loses the
  regression tripwire and the credible exit path. Rejected.

## Failure modes

- **Concurrent same-field creation despite the discipline** (a future
  caller forking before declaring fields): the adversarial suite's
  factories make it structurally impossible today; P1 schema
  versioning must keep it that way. This is the sharpest edge on
  record.
- **Library major-version churn** (automerge 0.7/hexane already
  demonstrated it): mitigated by the committed lockfile and by the
  port, which absorbs API drift in exactly two files
  (`loro_engine.rs`, `automerge_engine.rs`).
- **Snapshot growth over long histories**: both encodings grow;
  automerge's shrink-on-save and the sync protocol's delta mode are
  the scaling paths. Revisit if measured growth becomes a relay
  budget problem.
- **Trait drift toward engine specifics**: any method smelling of
  automerge or loro internals on `SyncDocumentEngine` is a design
  smell — the port stays engine-neutral by review.

## Migration

P1.1 adopts `AutomergeEngine` wherever `SyncDocumentEngine` is stored
(today: nothing in P0 consumes it — the spike is additive). Knowledge
documents get a declared field set created via `with_schema` before
replication. The Loro engine remains compiled and tested.

## Exit strategy

Swap to Loro (or a successor) by changing which engine P1 constructs;
every consumer holds `dyn SyncDocumentEngine` or `AnyEngine`, so no
call site changes. The adversarial suite already validates the
replacement continuously — the bake-off never ends, it just stops
being a decision.

## Revisit triggers

- Loro's encoded state decisively smaller at realistic document sizes
  (the 1000-op spike is a smoke measurement, not a benchmark suite).
- automerge-sync's `SyncMessage` proves awkward over the relay →
  re-evaluate Loro's sync protocol.
- A container-creation data-loss bug reaches production → the schema
  discipline failed; revisit whether name-addressed containers (Loro)
  should be the default.
