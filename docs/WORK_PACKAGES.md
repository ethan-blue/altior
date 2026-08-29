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

Exit: a new thread recalls a confirmed fact, explains its source, accepts a
correction, and excludes forgotten content.

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
