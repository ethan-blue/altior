# Memory system

## Modes

- `Off`: no long-term retrieval or extraction; thread history remains durable.
- `Session`: active harness context plus Altior thread summary, no cross-thread recall.
- `LongTerm`: scoped retrieval before a turn and candidate extraction afterward.

## Memory model

A memory contains an ID, content, scope, kind, confidence, lifecycle state,
provenance, creation/update times, optional expiry, and sensitivity classification.
Scopes include global, person, project, and thread. Lifecycle states include
candidate, confirmed, rejected, forgotten, and expired.

## Write policy

- Explicit user requests to remember may create confirmed memories.
- Clear stable user statements may be auto-confirmed only under a documented rule.
- Model interpretations and summaries begin as candidates.
- Contradictions create a correction event and retain history; they do not mutate
  provenance invisibly.
- Secret-shaped content is rejected before database and journal writes.

## Retrieval

The initial engine uses SQLite FTS plus deterministic ranking:

```text
text relevance + scope weight + confidence + recency/expiry + explicitness
```

Return at most eight memories by default. Context Runtime applies a hard token
budget and records selected memory IDs and ranking explanations for diagnostics.
Embedding retrieval is a later optional implementation behind `MemoryRetriever`.

## Cross-device provenance

Confirmed memories and their minimal supporting excerpts synchronize by default.
Full transcript synchronization remains opt-in. A device must be able to answer
"why do you remember this?" without downloading unrelated conversation history.

