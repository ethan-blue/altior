# Memory system

## Modes

- `Off`: no long-term retrieval or extraction; thread history remains durable. Passthrough is byte-for-byte exact.
- `Session`: strictly thread-local context (`MemoryScope::Thread(current_thread)`). Excludes global, person, project, and other thread memories.
- `LongTerm`: scoped retrieval before a turn and candidate extraction afterward. Scopes permitted: Global, Person (vault owner), current Thread, and current Project (when the thread belongs to an assigned project).

## Memory model & Scopes

A memory contains an ID, content, scope, kind, confidence, lifecycle state,
provenance, creation/update times, optional expiry, and sensitivity classification.
Scopes include:
- `global`: universally applicable across all conversations.
- `person`: personal details of the vault owner.
- `project`: strictly confined to threads associated with the named project. Projectless threads never recall project memories.
- `thread`: strictly confined to the origin thread.

Lifecycle states include candidate, confirmed, rejected, superseded, forgotten, and expired.

## Write policy

- Explicit user requests to remember may create confirmed memories.
- Clear stable user statements may be auto-confirmed only under a documented rule.
- Model interpretations and summaries begin as candidates.
- Contradictions create a correction event and retain history; they do not mutate
  provenance invisibly.
- Secret-shaped content is rejected fail-closed before database and journal writes.

## Retrieval & Prompt Trust Hierarchy

The initial engine uses SQLite FTS plus deterministic composite ranking:

```text
text relevance + scope weight + confidence + recency/expiry + explicitness
```

Context Runtime applies an explicit three-tier prompt trust hierarchy:
1. **Tier 1 (Authoritative)**: User Identity & Profile Documents (`# Identity`).
2. **Tier 2 (Current Request)**: User prompt for the active turn.
3. **Tier 3 (Untrusted Reference Data)**: Passively retrieved memories (`# Relevant Memories`).
   Framed with explicit untrusted reference warnings; all leading markdown headings are neutralized so memory items cannot impersonate system instructions or authorize tool execution.

### Token Estimation & Budget Defaults

- Estimator version: `v1_bytes_div_ceil_4` (`ceil(bytes / 4)`), a pure deterministic heuristic.
- Default identity budget: 1,024 estimated tokens.
- Default memory budget: 2,048 estimated tokens.
- Snapshot JSON cap: 64 KiB (`CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES`). Exceeding entries are recorded in `dropped` with their rank and reason (`BudgetExhausted` or `ScopeDisallowed`).

## Retention, Forgetting & Cryptographic Purge

Altior distinguishes four levels of lifecycle retention:
1. **Future Retrieval Exclusion**: Calling `forget_memory` appends a `memory.forgotten` domain event, updates the record state to `forgotten`, and deletes it from `memory_fts`. It is never injected into any future turns.
2. **Historical Audit Preservation**: Past `ContextSnapshot` rows remain immutable so users can audit what context was provided to an agent during historical turns.
3. **Sync Tombstone**: The `memory.forgotten` event propagates across paired devices to ensure offline devices do not resurrect the forgotten memory upon sync.
4. **Cryptographic Purge**: Logical forgetting is not cryptographic sector wiping. SQLite journal files retain tombstoned events until explicit vault reset or vacuum.
