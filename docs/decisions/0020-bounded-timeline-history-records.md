# ADR 0020: Bounded timeline history records (journal-backed, real content)

- Status: Accepted
- Date: 2026-09-06

## Context

Review finding F07 (task A05): the Desktop history path returns `TurnDto`
records that carry no message content, no tool/permission order, and no
truncation policy, so the renderer displays the placeholder
`Turn trn_<id>` after a restart. The facts already exist: Core persists
every runtime event — `TurnStarted` (with prompt content), `MessageDelta`
(streaming text), `PermissionRequested`/`PermissionDecided`,
`TurnCompleted`/`Cancelled`/`Failed` — as JSON-payload rows in the
append-only `domain_journal` table (schema v2, ADR 0013), indexed by
`(thread_id, seq)` and `(turn_id, seq)`. What is missing is a read-back
query, a versioned wire contract for ordered timeline entries, and a
Desktop reduction that treats live events and history pages identically.

Constraints: the journal must stay the single fact source (no second
copy); pages must be bounded (no unbounded snapshot back to the UI);
unknown or corrupt journal rows must degrade to a bounded preserved entry
instead of failing the page; local IPC DTO extensions must not become sync
wire format; no schema migration is allowed to change already-released
migrations.

## Decision

### 1. Timeline entries are journal projections

`get_history` returns `entries: Vec<HistoryEntryDto>` — one entry per
journal fact, ordered by journal `seq` ascending. The entry set is a
strict projection of the journal, never a parallel record. Kinds:

- `user_message { event_id, turn_id, seq, text, occurred_at }` — from
  `TurnStarted.payload.content` (the user prompt).
- `assistant_delta { event_id, turn_id, seq, text, occurred_at }` — one
  per `MessageDelta`; the client merges deltas by turn, exactly as it does
  for live streaming.
- `permission { event_id, turn_id, seq, permission_kind, description,
  decision, occurred_at }` — from `PermissionRequested`.
- `permission_decision { event_id, permission_event_id, turn_id, seq,
  decision, occurred_at }` — from `PermissionDecided`; the client applies
  it to the matching permission row (including rows that live on an earlier
  page).
- `turn_state { event_id, turn_id, seq, state, reason, delivery,
  occurred_at }` — from `TurnCompleted`/`TurnCancelled`/`TurnFailed`.
- `unknown { event_id, seq, kind, diagnostic }` — journal rows whose kind
  or payload cannot be projected; `diagnostic` is bounded (≤ 512 bytes)
  and the page never fails because of them.

### 2. Pagination is journal-seq-based and turns toward the past

`GetHistoryCommand` gains `before_seq: Option<u64>` (serde default;
additive under protocol v1). Core selects the ≤ `limit` journal rows for
the thread with `seq < before_seq` (or the newest page when absent) and
returns them oldest-first. The response carries
`next_seq_cursor: Option<HistoryCursorDto>` (`{ seq }` of the oldest entry
in the page) and `has_more`. `HistoryCursorDto { seq }` is a new type; the
existing `next_cursor`/`turns` fields keep their meaning for older clients
and turn-level summaries. The client requests older pages by passing the
last `next_seq_cursor.seq` as `before_seq` and **prepends** the reduced
rows.

### 3. Bounded pages and bounded entries

- Entry count per page: `limit` clamped to `[1, HISTORY_LIMIT_MAX = 500]`.
- Byte budget: Core accumulates encoded entries and stops before exceeding
  7/8 of `EnvelopeLimits.payload_bytes`, closing the page with
  `has_more = true` and a cursor. The wire envelope bound is therefore
  never the thing that breaks first.
- Text fields inside entries are already bounded at write time
  (`MessageText` cap, bounded permission descriptions); read-back clamps
  again defensively and flags nothing — the journal cannot contain
  oversized text by construction.

### 4. Live and history share one reduction

The Desktop maps both live IPC events and history entries onto the same
reduction: a prompt becomes a user row, deltas append to the turn's
assistant row (creating it on first sight), permissions become/patch
permission rows, `turn_state` finishes streaming (and appends the
failure/cancel notice). History is rendered by replaying entries through
this reduction; nothing on the history path constructs rows that the live
path could not produce.

### 5. Availability and restart

`get_history` reads only SQLite; it never launches or probes an agent
harness, so recorded content stays readable while the agent is absent,
crashed, or mid-restart. `ThreadHistoryResponseDto.high_water_seq` (the
thread's current journal max seq at query time) lets the client know where
live replay must catch up from; combining snapshot + subscribe stays under
ADR 0006's delivery rules.

### 6. Compatibility

- All response changes are additive optional fields; older clients ignore
  them and keep the turn-summary rendering. A client receiving a response
  without `entries` (older Core) falls back to turn placeholders.
- This is a local IPC DTO surface only; the sync wire format is unchanged,
  and full transcript sync remains opt-in per SECURITY.md.
- No schema change: `domain_journal` already stores what is needed; the
  new read path is SELECT-only. Downgrade behavior is therefore trivially
  safe (an older Core simply serves the old response shape).

## Alternatives

- **Merge deltas server-side into full assistant messages**: rejected —
  the client would need a second rendering path for live streaming, and
  full-text pages would re-send growing text on every older page.
- **Return raw journal payloads and let the client decode kinds**:
  rejected — the wire contract would leak journal internals and move
  bounded-decoding duty to every client.
- **Turn-level cursor (started_at, turn_id)**: kept for the existing
  `next_cursor`, but journal seq is the content cursor — it is total,
  monotonic, and stable under same-millisecond turns.
- **New IPC command** (`get_timeline`): rejected — `get_history` already
  owns "thread content pagination"; additive fields avoid a parallel
  surface.

## Failure modes

- **Corrupt/unknown journal row**: degraded to `unknown` entry with
  bounded diagnostic; the page still returns.
- **Page larger than the payload bound**: Core closes the page early with
  a cursor; no truncation of individual entries.
- **Client reconnect across page fetches**: cursors are journal seqs and
  remain valid across Core restarts (journal persists); a seq beyond the
  current high-water simply yields an empty page.
- **Mixed old/new Core**: additive fields keep both directions rendering.

## Migration path

None required (no stored-format change). The read path is additive SQL on
an existing table with existing indexes.

## Exit strategy

`HistoryEntryDto` is a plain DTO; if a future protocol v2 reshapes the
history surface, the entries/cursor types can be replaced behind the
version bump while the journal stays authoritative.

## Revisit when

- Tool-call facts become first-class domain events (the ACP runtime
  currently surfaces tool activity only inside agent text/streams); the
  `unknown` entry kind is the designated extension point until then.
- Full transcript sync (opt-in) needs the same entry vocabulary on the
  wire.
