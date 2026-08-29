# Altior contributor rules

`AGENTS.md` is the canonical instruction file for coding agents and contributors.
Read every `AGENTS.md` from the repository root to the file being changed.
Keep durable product and architecture decisions in `docs/`; do not hide load-bearing
behavior in prompts, issue comments, or private notes.

## Product invariants

- Altior is a local-first, multi-device personal knowledge runtime. It is not a
  team workspace, organization SaaS, or hosted chat product.
- One person owns one Personal Vault. Devices join through cryptographic pairing;
  there are no companies, memberships, invitations, roles, billing, or email login.
- Knowledge remains useful offline. Network loss must never block local reads,
  local writes, ACP conversations, or memory retrieval.
- The relay is an untrusted encrypted mailbox. It must not receive plaintext
  memories, conversations, document contents, credentials, or recovery keys.
- ACP is one replaceable agent harness, not the domain model. Terminal and future
  native harnesses must use the same thread, turn, permission, and event contracts.
- Personality, memory, skills, and scheduling belong to Altior Context Runtime,
  above every harness. Never implement them inside an ACP adapter.

## Architecture boundaries

- `altior-domain` contains platform-neutral domain types and rules. It must not
  depend on Tauri, ACP, a CRDT engine, SQLite, networking, or OS APIs.
- `altior-protocol` owns versioned Desktop/Core IPC DTOs and event envelopes.
- Infrastructure crates implement domain ports. Dependencies point inward.
- Desktop is a client of `altior-core`; it must not open the database, spawn
  agents, read secrets, or assemble model context directly.
- SQLite is a local projection and query index, never the synchronization wire
  format or conflict-resolution authority.
- Synchronized facts use immutable signed events. Concurrently editable documents
  use the selected CRDT engine behind `SyncDocumentEngine`.
- Large blobs are content-addressed, chunked, encrypted, and synchronized outside
  CRDT documents.
- No source dependency on Lody. Apache-2.0 code may be reused only deliberately,
  with required attribution and a provenance note in `THIRD_PARTY_NOTICES.md`.

## Sync and security

- Device private keys and provider credentials live in the OS secret store.
  Never persist them in SQLite, Markdown, logs, crash reports, IPC payload dumps,
  fixtures, or sync envelopes.
- Use reviewed libraries and standard primitives; do not invent cryptography.
- Every remote envelope is authenticated before decryption and validated again
  before projection. Treat relay data and paired-device payloads as untrusted.
- Device revocation, key rotation, replay protection, protocol versioning, and
  bounded resource use are required parts of the first sync protocol.
- Sync defaults are explicit by data family. Runtime paths, environment variables,
  permissions, process state, logs, and secrets are always device-local.
- A forgotten memory must propagate as a durable tombstone/event. Compaction must
  not resurrect it on a device returning after a long offline period.

## Memory

- Every durable memory records scope, kind, confidence, provenance, timestamps,
  lifecycle state, and optional expiry.
- Model inference is a candidate, never an authoritative fact. Explicit user
  statements may be promoted according to documented policy.
- Secret-shaped content is rejected before durable write and before sync.
- Retrieval is bounded and explainable. Results must expose why they matched and
  where they came from.
- Synced source evidence is minimal and encrypted. Do not require full transcript
  sync merely to make a memory explainable.

## Agent harnesses

- Stable domain events are translated to and from provider/protocol-specific
  messages only inside a harness implementation.
- Start with stable ACP v1. Experimental protocol versions stay behind feature
  flags and cannot change stored domain records.
- Capability support is negotiated and recorded; never infer it from an agent
  version string.
- Agent authentication and billing belong to the external provider. Altior stores
  only device-local launch configuration and secret references.
- A prompt retry is allowed only when delivery is provably absent. Never duplicate
  a possibly executed turn.

## Engineering discipline

- All AI-authored work follows `docs/AI_DEVELOPMENT.md`; prompts and agent summaries
  are not substitutes for checked-in contracts, fixtures, tests, or decisions.
- Rust stable, edition 2024. `cargo fmt`, `cargo clippy --all-targets --all-features
  -- -D warnings`, and `cargo test --workspace` are required before merge.
- Prefer explicit typed errors, cancellation-safe async code, bounded channels,
  structured concurrency, and idempotent operations.
- Tests must not depend on real sleeps, public networks, machine load, or scheduler
  luck. Use fake clocks, deterministic identities, in-memory transports, and
  synthetic transcripts.
- Add migrations; never edit an already released migration. Crash safety and
  downgrade behavior must be documented for durable format changes.
- Preserve unrelated work. Keep changes narrow, reviewable, and tied to an
  acceptance criterion or architecture decision.
- Commit subjects use Conventional Commits. Security-sensitive changes call out
  threat-model impact in the commit body or pull request.

## Decision process

- Before introducing a framework or protocol, write or update an ADR under
  `docs/decisions/` with alternatives, failure modes, migration path, and exit
  strategy.
- Temporary compatibility code must state its removal condition.
- If a request conflicts with these invariants, stop and surface the conflict
  instead of silently weakening the design.
- The first production harness is ACP. Terminal, Codex app-server, multi-agent
  coordination, and an Altior-native loop remain deferred until the ACP
  continuity acceptance journey passes or a later ADR changes the release scope.
