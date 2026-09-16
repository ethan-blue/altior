# Work packages

Each package ends in a reviewable artifact and acceptance evidence. Parallel work
may begin only after its referenced contract is frozen.

## P0 — Technical proof, two weeks

- Core/Desktop IPC handshake and process supervision
- ACP v1 connection to two real external agents
- normalized synthetic event fixtures
- encrypted two-device relay spike
- Loro/Automerge bake-off using the shared adversarial test suite
- SQLite projection rebuild from a journal fixture

Exit: ADRs select IPC, CRDT, storage, cryptographic library, and process model.

## P1 — Local thread runtime, three to four weeks

- thread/turn/event domain implementation
- ACP harness only; keep other harnesses behind unimplemented stable ports
- streaming, permissions, cancellation, resume, and process cleanup
- SQLite migrations and thread search
- minimal Desktop Chat, Threads, Agents, and Settings views

Exit: two agents complete and resume real threads without sync.

## P2 — Identity and memory, three weeks

- identity documents and Context Runtime
- memory lifecycle journal, FTS retrieval, extraction, correction, forgetting
- provenance UI and context-budget diagnostics
- secret-shaped content filter

### Subpackages & Architectural Decisions:

- **P2.1 Long-term memory lifecycle and ranked retrieval (ADR 0017)**:
  Authoritative `domain_journal` events, `memory` projection, fail-closed `is_secret_shaped` filtering, composite scoring, and forget tombstones.
  *Status*: Complete in local storage/domain crates.
- **P2.2 Identity documents and explainable context assembly (ADR 0018)**:
  Device-local `identity_document` table, deterministic token budgeting, per-turn `context_snapshot` audit rows, and Desktop Context panel.
  *Status*: Complete in local Core/Desktop crates. (Note: default token budgets updated to 1024/2048 in ADR 0022).
- **P2.3 Review hardening, boundaries, and performance (ADR 0019–0024)**:
  - ADR 0019: Identifier allocation boundaries (Core allocates entity IDs; Desktop allocates CSPRNG operation IDs; idempotent deduplication).
  - ADR 0020: Bounded timeline history records over durable journal events.
  - ADR 0021: Epoch-scoped stream deduplication and replay window bounds.
  - ADR 0022: Multi-scope trust boundaries (`Global`, `Personal`, `Project`, `Thread`) and versioned token heuristics.
  - ADR 0023: SQLite FTS5 trigram hybrid retrieval for Chinese CJK and code tokens.
  - ADR 0024: Projection digest checkpointing on turn settlement with crash healing.
  *Status*: Complete in local Core/Storage/Desktop crates.

Exit: a new thread recalls a confirmed fact, explains its source, accepts a
correction, and excludes forgotten content. Verified by `p22_context_injection`,
`p24_memory_product`, and `p25_memory_retrieval_relevance`.

### Explicit Non-Claims & Deferred Boundaries:
- **Real Third-Party ACP**: While `mock_acp_agent` passes all 8-step continuity journeys, live external agent smoke testing against third-party models (A18) remains opt-in and blocked on external credentials/environment.
- **OS Secret Store**: Windows Credential Manager is integrated as the production resolver behind `SecretResolver` (ADR 0026; `AcpHarnessAdapter` defaults to `OsSecretStore`, hermetic tests keep explicit `NoSecretsResolver`/`with_no_secrets`). Verified on Windows with a live Credential Manager round-trip test including guaranteed cleanup. macOS Keychain / Linux Secret Service drivers remain deferred and fail closed.
- **Production Synchronization**: Under ADR 0025, multi-device synchronization is explicitly disabled (`sync_enabled = false`). P0.5 crypto and relay spikes remain pre-production reference code until all 11 ADR 0025 threat mitigations and four acceptance test suites are satisfied in P3.
- **Release Signing**: Windows installer signing and production distribution packaging (P5) remain unexecuted.

## P3 — Personal Vault sync, four to six weeks

- device identity, pairing, recovery, revocation, and key rotation
- relay service and self-host configuration
- knowledge journal, CRDT document, and optional blob synchronization
- compaction, offline catch-up, diagnostics, and corruption handling

Exit: three devices pass the offline/concurrency/revocation acceptance matrix.

## P4 — Project workbench and extensions, three to four weeks

- project registration, file tree, Git status/diff, PTY, attachments
- MCP and skills registries
- scheduler foundation
- permission profiles and path containment

Exit: an agent works inside an approved project while the user reviews terminal
activity and file changes; denied paths remain inaccessible.

Terminal Harness, Codex app-server, multi-agent coordination, and the
Altior-native harness are post-ACP extensions. They do not block P1 through P5
unless a later ADR deliberately promotes one into a release slice.

## P5 — Release, two to three weeks

- Windows installer, signing, updater, backup/restore, support bundle
- migration, crash recovery, soak, resource, and security tests
- license and third-party notices

Exit: release acceptance targets in `ACCEPTANCE.md` pass on a clean Windows host.
