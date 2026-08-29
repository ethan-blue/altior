# Architecture

## Runtime topology

Altior Desktop is a Tauri/React client. `altior-core` is a separately supervised
Rust process that owns durable state, agent processes, terminal sessions, memory,
sync, and scheduling. Closing or reloading the UI must not terminate active work.

```text
Desktop UI
    | versioned local IPC
Altior Core
    |-- Context Runtime
    |-- Aggregated Agent Runtime and Operation Coordinator
    |-- Agent Harness Registry
    |-- Memory and Search
    |-- Project/Git/Terminal services
    |-- Sync Engine and Crypto
    `-- SQLite projections
```

## Dependency rule

Domain types and use cases sit at the center. Protocol, database, ACP, CRDT,
Tauri, operating-system, and network code implement outward-facing ports. No
infrastructure type is stored directly in a domain record.

## Core ports

- `AgentHarness`: capabilities, create/resume thread, prompt, steer, cancel, close
- `AgentCoordinator`: deterministic binding, bounded child operations, result
  collection, cancellation, and crash reconciliation
- `MemoryRepository`: lifecycle writes and provenance reads
- `MemoryRetriever`: bounded explainable retrieval
- `MemoryExtractor`: post-turn candidate production
- `SyncEngine`: publish, receive, acknowledge, compact, diagnose
- `SyncDocumentEngine`: concurrent knowledge-document edits
- `SecretStore`: opaque device-local secret references
- `SkillProvider` and `ToolProvider`
- `ProjectService`, `TerminalService`, `BlobStore`, `Scheduler`

## Durable ownership

- Altior owns Thread and Turn identities.
- A harness session ID is an optional replaceable binding on a Thread.
- The event journal is authoritative for syncable knowledge lifecycle.
- CRDT documents are authoritative for concurrently editable knowledge documents.
- SQLite is authoritative only for device-local settings and runtime state; its
  syncable projections can always be rebuilt.

## Context path

```text
User input
 -> policy and secret filtering
 -> identity documents
 -> scoped memory retrieval
 -> project knowledge
 -> selected skills
 -> token budgeter
 -> AgentHarness
```

Retrieved memory is reference material, not executable instruction. Context
sections carry type, scope, provenance, and trust markers.

## Agent aggregation

An Altior Agent is an `AgentProfile`, not a provider process. The profile selects
identity, memory, skills, permission defaults, and preferred device-local harness
bindings. ACP, Codex app-server, terminal, and future native execution are
adapters below that stable profile.

Delegation creates ordinary child threads joined by durable `Operation` records.
This keeps multi-agent work inspectable and recoverable without introducing
companies, memberships, or shared-workspace semantics. See ADR 0002 and
`REFERENCE_ARCHITECTURES.md`.
