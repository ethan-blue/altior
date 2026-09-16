# ADR 0021: Epoch-Scoped Stream Deduplication, Replay Recovery, and Bounded UI Cache

- **Status:** Accepted
- **Date:** 2026-09-06
- **Authors:** Ethan, Altior Core & Desktop Teams
- **Deciders:** Architecture Review (A06 / Finding F09)
- **Consulted:** Desktop Team, Security Team
- **Informed:** Core Runtime

## Context

Altior's Desktop connects to `altior-core` via local IPC (`altior-ipc`). As defined in ADR 0004, ADR 0006, and ADR 0015:
1. Every Core daemon instance mints a unique `CoreInstanceId` (`cor_<16..64 alnum>`) upon initialization.
2. Stream events carry a monotonic sequence number `Sequence` (1-based `u64`), which starts from 1 per Core instance.
3. The greeting handshake (`CoreGreeting`) advertises `instance_id` and an optional `RetainedWindow` (`from..=through`).

Prior to A06, the Desktop client (`applicationStore.ts`) tracked event deduplication using an unbounded `Set<string>` (`seenEventIds`) and an unbounded `Set<number>` (`processedSequences`), without binding sequence numbers to the Core instance ID:
- **Core restart bug:** When Core restarted and emitted sequence 1 in a new epoch, Desktop silently dropped it because sequence 1 had already been recorded in the previous Core epoch.
- **Memory leak:** On long-running desktop sessions handling tens of thousands of streaming chunks, the deduplication sets and `timelineStores` grew without bound.
- **Cache unpinning danger:** Naive cache eviction could evict a conversation with an active background turn or unsubmitted draft, losing in-flight state.

## Decision

### 1. Epoch-Scoped Sequence Cursor

Every sequence number is interpreted strictly within its owning `CoreInstanceId` (the **epoch**):

$$\text{StreamCursor} = (\text{coreInstanceId}: \text{string}, \text{sequence}: \text{number})$$

- When `CoreGreeting` is received, the client compares `greeting.instance_id` with its current epoch:
  - If identical: continue current sequence tracking.
  - If different (new Core instance or reconnect to restarted daemon): transition to the new epoch.
- Upon epoch transition:
  - The sequence window is reset for the new epoch.
  - Stale sequence numbers from the superseded epoch are discarded.
  - Replay requests never cross epoch boundaries.

### 2. Bounded Sequence Window & Event Deduplication

To guarantee $O(1)$ memory consumption regardless of stream lifetime:
- `seenEventIds`: Fixed-capacity FIFO ring buffer (cap: 4,096 entries). Event IDs older than 4,096 are dropped from memory; their ordering is protected by the sequence window.
- `processedSequences`: Bounded window tracking sequences within $[\text{highWater} - W, \text{highWater}]$, where $W = \max(2048, \text{retainedWindow.len})$. Sequences below $\text{highWater} - W$ are rejected as expired duplicates.

### 3. Bounded Timeline Store Cache with Pinned Working Set

`timelineStores` in `applicationStore` is constrained to an LRU cache with a configurable capacity (default: 20 conversations):
- **Pinned Working Set:**
  1. The currently selected conversation (`selectedThreadId`).
  2. Any conversation with an active turn (`activeTurns[].threadId`).
  Pinned conversations are **never evicted**.
- **Eviction Invariant:** Evicting an unpinned, idle conversation releases only derived UI DOM and VirtualWindow measurements. The durable source of truth resides in SQLite Journal (ADR 0009, ADR 0020). Reopening an evicted conversation re-queries `get_history` seamlessly.
- **Draft & Pending Protection:** User draft inputs and pending permissions are decoupled from timeline store rows and preserved across timeline store recycling.

## Alternatives Considered

1. **Global Unbounded Map with Periodic GC:**
   - *Rejected:* GC sweeps are prone to jank and unpredictable retention under load.
2. **Server-Side Session Persistence Across Restarts:**
   - *Rejected:* Violates the invariant that Core restarts are authoritative clean slates. Replay windows are bounded in-memory buffers; long-term continuity belongs to the journal history query (ADR 0020).

## Consequences

- Core restarts cleanly resume stream subscriptions with sequence 1 without event loss or manual client refreshes.
- Long-running sessions with over 1,000,000 synthetic events maintain a stable, bounded memory footprint.
- All historical content remains instantly available on demand through the journal-backed A05 pagination path.
