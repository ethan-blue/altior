# ADR 0019: Identifier allocation boundaries for Core entities and Desktop operations

- Status: Accepted
- Date: 2026-09-05

## Context

ADR 0004 froze the identifier string contract (`<prefix>_<16..64 chars from
[0-9a-z]>`) and stated that `altior-domain` validates but never generates
identifiers, leaving generation to `altior-core`. Its "revisit when" clause
asked for the generation convention to be recorded once real minting began.

The 2026-09-05 review (F01) found the Desktop fabricating entity-shaped
strings (`agent-…`, `trn_<millis>_<counter>`, `op_start_turn_1`,
`bin_alpha_01`, `thread-<n>_<millis>`) that violate the domain contract and
are rejected by real Core serde; the in-memory fixture transport accepted
them because it skipped validation. Separately, Core minted entity IDs as
`<prefix>_<16 hex of unix-millis>`, which collides when two entities of the
same kind are created within the same millisecond, and `configure_agent` /
`test_harness_binding` / `start_turn` responses did not carry the Core-minted
IDs back, forcing clients to guess.

Constraints: domain stays platform-neutral with no randomness source; tests
stay deterministic; no durable-format change is allowed in this slice; the
same protocol version 1 remains in force (additive response fields only).

## Decision

1. **Ownership.** `altior-domain` validates only. `altior-core` owns entity
   ID generation (threads, turns, agent profiles, harness bindings, events).
   The Desktop owns client operation ID generation. Neither side mints the
   other's identities.

2. **Core entity allocation convention.** Core allocates entity bodies as 16
   lowercase hex characters: 12 hex digits of unix-millis concatenated with
   4 hex digits of per-process noise (a wrapping counter mixed with the
   process id). This keeps bodies inside the ADR 0004 format, distinguishes
   creations inside the same millisecond in one process, and separates
   processes that restart within the same millisecond. Generation is a
   process-local infrastructure concern (`altior-core`); no wall-clock
   ordering is implied by ID sort order.

3. **Desktop operation allocation convention.** Operation IDs are opaque
   correlation identities: `op_` + 32 lowercase hex characters (128 bits
   from the platform CSPRNG). They are unique across renderer restarts with
   no persistent allocation state. Operation IDs carry no semantic payload —
   the command `kind` names the operation type.

4. **Retry identity rule.** A retry of a command whose delivery is
   unconfirmed must reuse the original operation ID (and, for `start_turn`,
   the original turn ID). A client must never mint a fresh identity for the
   same logical operation, because Core's operation registry and
   delivery-state checks would then see a different operation and could
   execute it twice.

5. **Core-minted IDs travel in responses.** `create_thread` already returns
   the full `ThreadDto`. The `command_result.data` payloads of
   `configure_agent`, `test_harness_binding`, and `start_turn` gain additive
   optional fields so clients use only confirmed identities:
   - `configure_agent` → `{"agent_profile_id": "...", "harness_binding_id": "..." | null}`
   - `test_harness_binding` → `{"ok": bool, "diagnostics": ...,
     "probed_binding_id": "..." }` (now always present, including a minted id)
   - `start_turn` → `{"admission": "...", "turn_id": "..."}` (now always present)
   Clients that need a Core-owned identity pass `null` at request time and
   read the minted value from the response. Unknown result-data fields are
   ignored by older peers, so this is an additive change under protocol v1.

6. **Fixture-transport equivalence.** The in-memory Desktop transport must
   validate incoming commands with the same ID/bound rules as real Core serde
   and produce the same response shapes. A fixture transport that accepts
   what production rejects is a defect, not a convenience.

## Alternatives

- **Client mints entity IDs** (current broken behavior): rejected — the
  client cannot guarantee global uniqueness, cannot be trusted at the trust
  boundary, and would force the domain to accept client-chosen identities.
- **Sequential per-process counters** (`thr_0000000000000001`): rejected —
  IDs collide across Core restarts, and low-entropy guessable IDs invite
  cross-record confusion in diagnostics.
- **UUID crate / UUIDv7 bodies** (anticipated by ADR 0004): deferred — adds
  a dependency for no current need; the 16-hex scheme already satisfies the
  format and collision requirements for device-local single-process minting.
  Revisit if entities are ever minted concurrently in multiple processes on
  one device.
- **Persistent operation ID journals in the renderer**: rejected — 128-bit
  random operation IDs make persistence unnecessary; a journal would add a
  durable format with migration obligations for no observable benefit.

## Failure modes

- **Same-millisecond creation burst**: distinguished by the 16-bit noise
  field (65536 allocations per millisecond per process before wraparound;
  wraparound cannot repeat within one millisecond because the counter is
  monotonic across the whole process lifetime).
- **Core restart inside the same millisecond**: process-id mixing separates
  the two processes' outputs.
- **Renderer restart reusing an operation ID**: prevented by CSPRNG
  generation (collision probability negligible at 128 bits).
- **Duplicate delivery of a possibly-executed prompt**: Core's operation
  registry and turn delivery-state checks reject re-execution; the retry
  identity rule keeps retries deduplicable.
- **Client assuming an entity exists before the response returns**: clients
  that need a Core-owned identity must pass `null` and use the response; the
  fixture transport enforces the same shape so drift is caught in tests.

## Migration path

None required. No persisted record changes shape; result-data gains are
additive JSON fields that older peers ignore. The invalid legacy shapes were
never accepted by real Core, so no stored data can contain them (the review's
F01 migration note: if evidence of invalid IDs entering the durable layer
ever appears, a separate repair tool must be designed first; scanning or
rewriting user vaults is prohibited).

## Exit strategy

The allocator is a leaf module inside `altior-core` with no dependents
outside command handlers; replacing it (e.g. with UUIDv7 bodies) touches one
module plus this ADR. Response-data fields are optional; clients must treat
missing fields as absent, never as a hard schema.

## Revisit when

- Entities are minted from more than one process per device (switch to
  UUIDv7 or a coordinated allocator).
- A command response needs to return bulk-minted IDs beyond the three
  commands listed above.
- Prompt retry UX (A04) needs persisted operation intents, at which point
  the retry identity rule gains a durable representation.
