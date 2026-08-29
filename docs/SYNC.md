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

## P0 engine bake-off

Implement the same `SyncDocumentEngine` test suite for Loro and Automerge. Test
three-device offline edits, 100k memory objects, 30-day simulated absence,
duplicate/out-of-order frames, corruption, compaction, revocation, and key rotation.
Record the selection in an ADR; remove the losing production adapter after the
decision while retaining portable fixtures.

