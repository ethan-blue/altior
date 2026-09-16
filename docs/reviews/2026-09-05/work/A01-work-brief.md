# A01 work brief — freeze real ID boundaries and fix IDs

- **Task ID / subslice:** A01 (single slice; F01 + F32 command-validation half)
- **Baseline:** `f945ef153eb3344abd3801a3c2e4002fd25d80be`

## Outcome (user-visible)

Every command the Desktop emits decodes against the real Rust protocol DTOs.
Entity identities (threads, turns, agent profiles, harness bindings) are minted
by Core and returned to the client in command responses; the client never
fabricates one. Client operation IDs are valid, restart-unique, and retried
operations keep their identity. Same-millisecond creations do not collide.
Historically emitted invalid shapes are proven rejected by checked-in fixtures.

## Acceptance

1. Desktop-emission fixtures for list_threads, runtime_status, diagnostics,
   open_thread, get_history, search_threads, create_thread, configure_agent,
   test_harness_binding, start_turn, cancel_turn, respond_permission,
   get_context_snapshot, list_identity_documents, put_identity_document,
   delete_identity_document decode + validate in Rust (`altior-protocol`
   fixture suite), including Chinese titles/prompts and null optional IDs.
2. Checked-in invalid fixtures (legacy `op_start_turn_1`, `agent-alpha`,
   `trn_<ts>_<n>`, `bin_alpha_01` shapes) are rejected with typed errors.
3. `InMemoryTransport` validates every incoming command with an equivalent
   TS mirror and rejects invalid commands (`InvalidCommandError`); it mirrors
   real Core response shapes (configure/test/start_turn carry Core-minted IDs).
4. Store: start_turn sends `turn_id: null` and adopts the Core-returned turn
   id; onboard/test use Core-returned profile/binding ids; createThread uses
   only the Core-returned thread id (failure adds no local thread); operation
   IDs come from a documented allocator and are unique across store instances.
5. Core: entity ID allocator produces same-millisecond-distinct, format-valid
   ids (12 hex millis + 16-bit process-mixed counter); configure/test/start_turn
   responses carry the minted IDs.
6. dto-export re-run shows no TS drift; all repository gates pass.

## In scope

`altior-domain` (read-only), `altior-protocol` fixtures/tests,
`altior-core` ID allocation + response payloads, desktop `ipc/` modules,
`applicationStore`, `fixtures/timeline.ts`, App.tsx literal fallback,
existing test updates, ADR 0019, CHECKPOINT evidence.

## Out of scope

Honest create/cancel/permission failure UX (A04), per-thread active-turn map
(A04), demo-data removal and search-generation guards (A03), secret-ref
semantics incl. `sanitizeSecretRef` fabrication (A07), history DTO redesign
(A05), any domain ID format relaxation, any migration (no durable format
change; command_result `data` gains additive optional JSON fields only).

## Contracts touched

ADR 0004 (identifiers, envelopes — unchanged, extended by ADR 0019 for
allocation boundaries); `CommandEnvelope` payload schemas (unchanged);
command_result `data` contents for create/configure/test/start_turn
(additive). New ADR 0019 records: Core owns entity ID generation (allocation
convention), Desktop owns operation ID generation, retry identity rule.

## Failure / cancellation / offline / restart

Invalid commands are rejected before transport dispatch (fake) or by Core
serde (real); nothing is half-applied. No new cancellation paths. Renderer
restart mints fresh operation IDs (128-bit random); Core restart keeps ID
uniqueness via millis + process-mixed counter.

## Security / synchronization impact

No secrets touched; `sanitizeSecretRef` intentionally unchanged (A07). No
sync surface affected. Error strings carry no prompt content (unchanged).

## Migration / downgrade

None required: no durable format change, no stored record shape change.
Older Cores ignore unknown JSON keys in result data; older Desktops ignore
unknown result-data fields (additive rule, ADR 0004).

## Tests / evidence

Rust: `altior-protocol` desktop fixture suite (valid + invalid shapes);
entity allocator unit tests (same-ms distinct, prefix/format); workspace gates.
TS: fake validation rejections; store emits only contract-valid commands;
turn-id adoption; create-failure adds no thread; op-id restart uniqueness.
