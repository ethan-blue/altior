# Altior

Altior is a local-first, multi-device personal knowledge runtime for persistent
AI identity, memory, skills, and agent conversations.

It deliberately has no organization, company, team, billing, or traditional
product-login model. A person owns an encrypted Personal Vault and authorizes
their own devices through cryptographic pairing.

## Status

Architecture and executable-contract phase. The Rust workspace contains the
complete P0.1 contract skeleton: stable domain identifiers, clock and
delivery-state conventions, the Desktop/Core handshake and version
negotiation, versioned command/event/snapshot envelopes with bounded
payloads and unknown-event preservation, and deterministic protocol
fixtures (ADR 0004). `apps/desktop` runs the fixture shell on an in-memory
transport with TypeScript DTOs generated from the Rust contracts (ADR 0005).
P0.2 adds Core supervision and local IPC (ADR 0006): the `altior-ipc`
crate with bounded length-prefixed frames, per-launch capability tokens,
pure session machines (shared per-launch event log, sequence/catch-up with
retained-window replay or explicit gaps, epoch change on Core restart), and
`altior-core` with the spawn-or-attach supervisor, turn ownership that
survives UI reload, and two-sided duplicate-command prevention. P0.3 adds
the narrow ACP v1 adapter (`altior-acp`, ADR 0007): capability-only
negotiation, unknown-event-preserving mapping onto the protocol contracts,
prompt delivery classified onto the frozen `DeliveryState` vocabulary, a
pure process lifecycle machine, and an opt-in real-agent smoke harness
behind `ALTIOR_ACP_SMOKE_AGENTS`. P0.4 adds the Desktop workbench spike
(ADR 0008): the five-region shell with a zero-dependency virtualized
timeline (100,000 rows, prepend-stable history, per-row streaming
updates, scroll restoration), keyboard navigation with inline approval,
and a Tauri v2 shell outside the Rust workspace with pinned capability
minimums. The OS transport (tokio named pipes /
Unix domain sockets) is a named later slice. P0.5 hardens four runnable
foundation spikes (ADR 0009-0012): the SQLite journal-to-projection
store with forward-only migrations and self-healing rebuilds, the
Loro/Automerge `SyncDocumentEngine` bake-off (Automerge selected,
typed schema and framed/bounded imports, both engines raced in CI), the standard-primitives two-device
crypto envelope (direction-separated X25519/HKDF keys +
ChaCha20-Poly1305 with replay
windows and signed pairing transcripts), and the zero-dependency
content-agnostic relay with validated cursored fetch and checkpoint
compaction. These remain pre-production reference implementations:
counter persistence, contributory checks, ratcheting, durable relay
storage/acks, and per-device cursors are deferred to P1/P3. Next is the P1 ACP
desktop-continuity work described in
[Work Packages](docs/WORK_PACKAGES.md).

## Design anchors

- Rust core daemon with a Tauri/React desktop client
- local SQLite projections and search indexes
- signed event synchronization plus CRDT documents where concurrent editing is needed
- end-to-end encrypted relay, optional self-hosting, and offline-first behavior
- replaceable agent harnesses: ACP, terminal, and a future Altior-native harness
- personality and memory above every agent harness

Start with [Product](docs/PRODUCT.md), [Architecture](docs/ARCHITECTURE.md), and
[Contributor Rules](AGENTS.md). Implementation is ACP-first; see the
[Implementation Plan](docs/IMPLEMENTATION_PLAN.md),
[Desktop UI Architecture](docs/UI_ARCHITECTURE.md), and
[AI Development Discipline](docs/AI_DEVELOPMENT.md).
