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
survives UI reload, and two-sided duplicate-command prevention. The OS
transport (tokio named pipes / Unix domain sockets) is a named later slice.
Next is the P0 technical spike work described in
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
