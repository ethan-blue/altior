# ADR 0012: Relay transport — a content-agnostic queue machine

Date: 2026-08-30 · Status: accepted · Scope: P0.5 relay spike
(docs/IMPLEMENTATION_PLAN.md), feeding P1 multi-device sync.

## Context

`docs/ARCHITECTURE.md` lets an untrusted relay carry sync traffic
between the user's devices only if the relay can neither read nor
forge it. P0.5 must prove the relay's *shape* — its queue, quota,
retention, and compaction semantics — before any real server
exists. The spike is therefore a pure in-memory state machine with
**zero dependencies**, no network, and no timers; a network server
later wraps this exact contract.

What others do, and what we adopt or reject:

- **Content-agnostic queues with cursors** — every mailbox-service
  pattern (LMTP delivery queues, MQTT persistent sessions, AWS SQS
  approximate cursors): opaque payloads, monotonic sequence numbers,
  `fetch(after, limit)` paging. We adopt it with strict (not
  approximate) cursors.
- **Sealed sender** — Signal's discovery that even *metadata* (who
  sent to whom) is worth hiding: our push API takes a destination
  bucket and bytes, and has no sender field at all. There is nothing
  to log and nothing to subpoena; the property is structural, not
  policy. Adopted.
- **At-least-once delivery + receiver dedupe** — the standard
  queue-composition discipline: fetch never consumes (repeatable),
  and the ADR 0011 replay window makes re-delivery harmless. The two
  layers compose instead of duplicating work; the spike's
  integration test proves the composition directly.
- **Retention measured against an explicit logical clock** — the
  local-first testing discipline (no wall clock, no sleeps): the
  relay ages entries against a caller-advanced `tick()` counter.
  Production maps ticks to its server clock; the semantics are
  identical.
- **Quotas as backpressure, not eviction** — mailbox quotas: an
  over-full bucket refuses new pushes (`BucketFull`) instead of
  dropping old mail, so a runaway sender cannot evict a slow
  receiver's undelivered envelopes. Oversized single payloads are
  refused whole (`PushTooLarge`).

## Decision

### 1. New crate `altior-relay`, zero dependencies

Same placement pattern as ADR 0009–0011: the contract crates gain
nothing. `altior-relay` has an **empty dependency list** — it is
bytes, counters, and a HashMap — pinned by the dependency-boundary
test asserting the empty list. The only dependency is a dev one:
`altior-crypto`, for the end-to-end two-device integration test.

### 2. Model: buckets, cursored fetch, idempotent push

A `BucketId` names one device's inbox (production derives it from
the receiver's public identity, so the relay needs no account
directory). `push(bucket, id, payload) -> Pushed{seq} |
Duplicate{seq}` assigns one global monotonically increasing sequence
space — no reuse across buckets, so a cursor is unambiguous. A
repeated push id inside the retained window returns the original
receipt only when the opaque payload is byte-identical. Reusing an id
for different bytes returns typed `PushIdCollision`.

`fetch(bucket, after, limit)` returns every retained entry with
`seq > after`, ascending, and is **repeatable**: fetch never
consumes. The receiver's next cursor is the last returned `seq`.
An `after` beyond the bucket's actual frontier returns typed
`FetchPage::InvalidCursor`; it never mutates acknowledgement state.

### 3. The receiver's cursor is a possession claim

The subtle rule: a validated `fetch(after = N)` at or below the
bucket's actual frontier is the receiver *asserting possession of
everything through N*. The relay records the bucket's high-water
cursor from the valid request and last returned sequence; compaction
reclaims exactly the prefix the receiver has certainly seen. This is
what makes aggressive compaction safe — and it is honest about
trust: a receiver that lies about its cursor loses the mail it
claimed to have. Its loss, its lie.

### 4. Compaction with fetch equivalence, and the Compacted page

`compact(bucket)` folds the prefix below the receiver's cursor into
a checkpoint (counts and boundary; entries are dropped). The
equivalence contract, tested by snapshotting pages before and after:
**for every cursor at or after the checkpoint boundary, fetch
results are byte-identical**. A cursor that fell behind the boundary
gets `FetchPage::Compacted { compacted_up_to }` — an explicit
instruction to resync (in production: a fresh CRDT snapshot push,
ADR 0010) rather than a silent gap in the mail.

### 5. Retention: logical ticks, explicit sweep, reported loss

`sweep_expired()` folds entries older than `max_age_ticks` into the
checkpoint — *fetched or not* — and returns per-bucket summaries.
"Older" is strict: `age > max_age_ticks`; equality remains retained,
and `max_age_ticks = 0` expires an entry after the clock advances.
This is the only path where the relay deliberately loses undelivered
mail: by explicit policy age, reported to the caller, surfaced to
the receiver as a Compacted page. Nothing is lost silently.

### 6. Evidence composition

The integration test (`tests/two_device_flow.rs`) runs the full
spike stack: a sealed envelope (ADR 0011) crosses the relay; the
relay sees no plaintext (asserted down to substring level); a
duplicate re-push queues nothing; re-delivery is rejected by the
receiver's replay window; a third device with full relay access
still cannot open the mail; and a receiver that fell past retention
is told to resync, then receives fresh mail after the boundary.

## Alternatives considered

- **Destructive (at-most-once) fetch** — halves the queue cost, but
  makes any fetch crash a mail loss and pushes dedupe into the
  untrusted relay. Rejected; receiver-side dedupe (ADR 0011) is
  already bought.
- **Per-bucket sequence numbers** — saves a u64 here and there, but
  cursors then need bucket qualification everywhere and an
  at-least-once push retry across a bucket recreate could reuse a
  seq. One global space, no reuse.
- **Evict-oldest on quota** — classic bounded mailbox, but it lets
  a fast sender starve a slow receiver *silently*. Backpressure
  (`BucketFull`) makes the pressure visible at the push call site.
  Rejected for the spike; production may add per-sender sub-quotas
  with the same visible-failure property.
- **A real server now (axum/tokio + SQLite)** — out of spike scope
  and premature: the semantics live in this state machine, and the
  server is a transport adapter over the same contract (ADR 0006's
  adapter pattern).

## Failure modes

- **Forged future cursor**: rejected as `InvalidCursor`; it cannot
  move the checkpoint or hide later pushes. A valid cursor within the
  existing frontier remains a possession claim, so durable production
  acknowledgement authentication is still required.
- **Retention drops undelivered mail** after `max_age_ticks`: by
  policy, reported, and surfaced as a Compacted page. The resync
  path (CRDT snapshot) must exist before any production retention is
  shorter than realistic offline periods.
- **Unbounded `seen` ids**: dedupe checks only currently-retained
  entries; after compaction a re-push re-queues. Harmless for
  state-based CRDT payloads (import is idempotent) and for envelope
  traffic (replay window). A bounded seen-id LRU is the named
  production hardening.
- **Single consumer per bucket**: the cursor model assumes one
  receiver per inbox. Multi-device fan-out (one user, three devices
  reading the same bucket) needs per-device cursors — named as the
  P1 extension (per-device sub-cursors keyed by device id).
- **Memory bounds**: the state machine holds payloads in memory; the
  production server backs buckets with the ADR 0009 storage
  patterns instead. Same contract, different medium.

## Migration

P1.2 wraps this contract in a real transport (HTTP or WebSocket
adapter behind the ADR 0006 IPC/adapter pattern) and backs it with
persistent storage; `Relay` itself becomes the reference
implementation and the test double for offline simulation. The
`Compacted` page maps directly to the sync engine's
"snapshot-resync" trigger.

## Exit strategy

The public surface (`push`/`fetch`/`compact`/`sweep_expired`/
`stats`, `FetchPage`, `PushOutcome`) is the seam. Swapping the
in-memory machine for a server-backed client, or SQLite-backed
persistence, touches one implementation; every consumer and test
keeps calling the same contract.

## Revisit triggers

- Multi-device fan-out per bucket lands → per-device sub-cursors.
- Production retention tuning → align `max_age_ticks` with offline
  expectations and the ADR 0011 replay-window width (retention must
  exceed the window or re-delivery turns into refusal).
- Server deployment → persistence-backed implementation of this
  contract, new ADR for the wire encoding.
