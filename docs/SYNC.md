# Personal Vault synchronization

## Goals

- local writes never wait for a network
- authorized devices converge after arbitrary offline periods
- relay compromise reveals no plaintext
- duplicate, delayed, replayed, and out-of-order envelopes are safe
- forgotten data is not resurrected by stale devices
- transports and CRDT engines can be replaced without changing domain records

## Device identity and pairing

The first device creates a Personal Vault and recovery material. Each installation
has a unique signing/encryption identity. Pairing requires an already authorized
device or recovery flow, displays a human-verifiable fingerprint, and wraps the
Vault data key for the new device. Private keys stay in the OS secret store.

## Data planes

1. **Knowledge journal**: immutable signed lifecycle events for memories, devices,
   settings families, tasks, and summaries.
2. **Knowledge documents**: CRDT updates for SOUL, USER, curated MEMORY, and project
   documents.
3. **Blob plane**: content-addressed encrypted chunks for opt-in attachments.
4. **Ephemeral plane**: presence and progress; never required for convergence.

## Relay

The relay authenticates devices, stores bounded encrypted envelopes, tracks
acknowledgement cursors, and supports WebSocket delivery plus catch-up. It has no
decryption keys and no product-domain write API. Self-hosted and official relays
implement the same versioned transport contract.

## Conflict policy

- Immutable events merge by ID and validate causal references.
- Memory correction and forgetting are explicit lifecycle events.
- CRDT documents resolve concurrent textual/structured edits.
- Device-local settings never enter conflict resolution.
- Conflicting security events fail closed and surface a recovery workflow.

## Compaction

Snapshots are optimization only. A snapshot includes the covered event frontier,
tombstone frontier, schema version, and signer. A long-offline device must first
apply revocation and tombstone history before contributing new writes.

## P0 engine bake-off (Complete)

The `SyncDocumentEngine` bake-off evaluated Loro and Automerge across offline editing,
serialization performance, and memory bounds (ADR 0010). Automerge was selected as the
primary CRDT engine for Altior structured documents.

## Production Safety Barrier & Release Prerequisite (ADR 0025)

**Status: Synchronization is DISABLED in production desktop builds (`altior_core::SYNC_ENABLED = false`).**

The runtime hard latch lives in `crates/altior-core/src/sync_gate.rs`: call `altior_core::ensure_sync_allowed()` before any network sync, pairing-over-network, or relay connect path. Flipping the constant requires ADR 0025 acceptance evidence - never a Desktop toggle or env var.

Per ADR 0025 and review finding F31, the existing `altior-crypto` and `altior-relay` crates
serve as hermetic reference spikes. Enabling network synchronization in commercial or production
builds is blocked pending completion of the four required acceptance gates:

1. **`test_three_device_offline_convergence`**: Three devices concurrently edit documents, propose memories, and emit tombstones, achieving 100% convergence via the relay.
2. **`test_session_reconstruction_nonce_uniqueness`**: 1,000 session restarts with 100,000 generated ciphertexts prove zero duplicate nonces.
3. **`test_relay_zero_knowledge_audit`**: Frame byte audits prove zero plaintext memory, prompt, or credential leakage to the relay.
4. **`test_stale_device_tombstone_non_resurrection`**: 30-day simulated offline device re-sync proves forgotten facts are not resurrected.

See `docs/decisions/0025-sync-production-gate-threat-model-and-safety-barriers.md` for the
detailed threat tree, attack vector analysis, and production checklist.

