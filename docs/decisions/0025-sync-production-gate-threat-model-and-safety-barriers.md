# ADR 0025: Sync Production Gate, Threat Model, and Safety Barriers

- **Date:** 2026-09-06
- **Status:** Accepted (P0 Architectural Barrier)
- **Scope:** Cross-crate (`altior-crypto`, `altior-relay`, `altior-crdt`, `altior-storage`, `altior-domain`)
- **Review Finding:** F31 (Sync crypto and relay remains a pre-production spike)
- **Task Reference:** A20 (docs/reviews/2026-09-05/TASKS.md §A20)

---

## 1. Context & Problem Statement

Altior invariants (AGENTS.md) require that:
1. One person owns one Personal Vault, paired cryptographically across devices without central servers, accounts, or corporate infrastructure.
2. The sync relay is an **untrusted, encrypted mailbox**. It must never observe plaintext memories, conversations, credentials, recovery keys, or document contents.
3. Forgotten memories must durably propagate as signed tombstones; compaction or long-offline devices must never resurrect forgotten facts.

In P0.5, reference spikes were created:
- `altior-crypto`: Two-device static-static X25519 ECDH + HKDF-SHA256 + ChaCha20-Poly1305 with 64-bit counter nonces and sliding replay windows (ADR 0011).
- `altior-relay`: In-memory content-agnostic WebSocket/HTTP message queue (ADR 0012).
- `altior-crdt`: Automerge document synchronization with bounded framed imports (ADR 0010).

### The Finding (F31)
In the P0.5 spike implementation:
1. When a pairwise `Session` is reconstructed from static keys (e.g. after a process restart or app reopen), `send_counter` resets to `0`, and the replay sliding window resets.
2. If two sessions are instantiated with the same static keys, **nonce reuse occurs under the same ChaCha20-Poly1305 key**, leading to catastrophic loss of confidentiality and integrity (two-time pad keystream XOR leakage).
3. The relay is an in-memory queue without persistent cursors, durable acks, client quotas, or malicious payload bounds.
4. If sync were prematurely enabled from the Desktop UI without resolving these invariants, user data would be exposed to silent key compromise and synchronization races.

---

## 2. Decision: Absolute Production Sync Barrier

1. **Explicit Compile-Time & Runtime Barrier**:
   - Synchronization is **strictly disabled** for production releases (`sync_enabled = false`).
   - The desktop client and core daemon must NOT expose active sync configuration or network synchronization to end users until every gate in this document is satisfied.
   - Code in `altior-crypto`, `altior-relay`, and `altior-crdt` remains isolated as tested reference spikes.

2. **No Custom Cryptography**:
   - Altior will never invent proprietary encryption schemes. All cryptographic mechanisms must use audited, standard algorithms (RFC 7748 X25519, RFC 5869 HKDF, RFC 8439 ChaCha20-Poly1305, RFC 8032 Ed25519).

3. **Mandatory Production Prerequisites**:
   - Production synchronization cannot be released until the 11 threat mitigations, 4 executable test suites, and independent cryptographic review checklist below are completed and verified.

---

## 3. Threat Tree & Mitigation Specifications

### Threat 1: Session Counter Reset & Nonce Reuse (F31)
- **Attack Vector**: Core restarts; a new `Session` is instantiated with static device keys; `send_counter` starts at `0`. An attacker collecting encrypted envelopes observes two ciphertexts with identical nonce `N` and key `K`:
  $$C_1 \oplus C_2 = P_1 \oplus P_2$$
  The keystream cancels out, completely breaking confidentiality.
- **Mitigation**:
  1. **Persistent Counter Reservation**: Counters must be monotonically allocated in persistent storage in reservation blocks (e.g., reserving chunks of 10,000) before any ciphertext is transmitted.
  2. **Random Epoch Salt**: Every session negotiation or restart must generate an ephemeral, non-repeating `epoch_id` (128-bit CSPRNG) exchanged via authenticated handshake. Nonces must be derived from `HKDF(epoch_id || counter)`.
  3. **Ephemeral Ratchet (Double Ratchet)**: Transition from static-static ECDH to an ephemeral Diffie-Hellman ratchet, guaranteeing that even with counter corruption, new keys are derived every round trip.

### Threat 2: Low-Order Public Key & Contributory ECDH Attacks
- **Attack Vector**: A malicious peer or compromised relay supplies a Curve25519 point of small order (order 1, 2, 4, or 8). The shared secret becomes a trivial, known point, allowing the attacker to decrypt all traffic.
- **Mitigation**:
  - Enforce strict contributory behavior and reject low-order points using `x25519_dalek::PublicKey` validation or Edwards25519 clamping and non-zero check on the scalar multiplication result per RFC 7748 §6.

### Threat 3: Disk Rollback & Virtual Machine Snapshot Cloning
- **Attack Vector**: A user restores their system from a disk backup or VM snapshot. The persistent counter rolls backward, causing the node to emit previously used counters under an existing session.
- **Mitigation**:
  - Store session states alongside a monotonic hardware-backed epoch token (e.g., TPM / OS Secure Enclave monotonic counter, or cloud-free epoch pairing where the peer rejects any inbound envelope with `epoch_seq <= max_observed_epoch_seq`).

### Threat 4: Compromised Device Revocation & Key Rotation
- **Attack Vector**: A paired phone or laptop is stolen. The attacker attempts to read subsequent vault updates or inject malicious edits.
- **Mitigation**:
  - The Vault owner signs a **Revocation Certificate** using their offline Personal Vault Recovery Key.
  - Revocation certificates are broadcast as high-priority domain journal events.
  - Devices immediately wipe pairwise session keys for the revoked device ID and rotate document symmetric encryption keys.

### Threat 5: Relay Eavesdropping & Data Infiltration
- **Attack Vector**: The sync relay is compromised or operated by a hostile third party attempting to read personal memories and conversation history.
- **Mitigation**:
  - Zero-knowledge relay architecture: Envelopes are encrypted with AEAD before reaching the transport. The relay receives only `recipient_device_token`, `envelope_id`, `payload_bytes`, and `timestamp`.
  - Content, thread titles, turn text, memory excerpts, and recovery phrases are 100% opaque ciphertext.

### Threat 6: Relay Envelope Tampering & Downgrade
- **Attack Vector**: Hostile relay re-orders, truncates, or modifies routing metadata (e.g., swapping sender ID or altering payload).
- **Mitigation**:
  - All routing headers (protocol version, sender device ID, recipient device ID, epoch, sequence counter) are authenticated as AEAD Additional Authenticated Data (AAD).
  - Any single-bit mutation in transit fails Poly1305 authentication closed with zero side effects.

### Threat 7: Relay Resource Exhaustion & Denial of Service
- **Attack Vector**: Malicious client floods the relay with massive envelopes or endless streams.
- **Mitigation**:
  - Hard physical bounds: Max envelope size = 1 MiB; max metadata = 4 KiB.
  - Per-vault quota = 50 MiB queue limit; unacknowledged messages expire after 14 days.
  - Client rate limiting (max 50 requests/sec).

### Threat 8: Decompression Bombs & Malicious CRDT Documents
- **Attack Vector**: Attacker sends a tiny compressed envelope that expands into gigabytes, exhausting device memory (zip bomb / CRDT expansion attack).
- **Mitigation**:
  - Streaming decompression limits: Max decompressed buffer ratio capped at 10x with absolute hard ceiling of 10 MiB.
  - Automerge document chunks are parsed inside isolated memory bounds; malformed or cyclically bloated graphs fail closed with typed `CrdtDecodeError`.

### Threat 9: 30-Day Offline Stale Device Resurrection
- **Attack Vector**: Device C has been offline for 60 days. Meanwhile, Device A deleted/forgot sensitive memories and rotated keys. Device C comes online and tries to push its old database state, resurrecting forgotten memories.
- **Mitigation**:
  - Mandatory **Pull-Before-Push Handshake**: A rejoining device must pull and apply all pending tombstone frontiers and revocation events *before* its local pending writes are admitted into the sync stream.

### Threat 10: Forgotten Memory Durability (Tombstone Immortality)
- **Attack Vector**: A user issues `forget_memory`. Local and remote compaction sweeps discard old records. If tombstones are prematurely compacted, an unsynced replica can re-propose the memory.
- **Mitigation**:
  - A forgotten memory generates a permanent tombstone tuple: `(memory_id, forgotten_at, tombstone_digest)`.
  - Compaction algorithms are mathematically prohibited from garbage-collecting a tombstone until the global vector clock confirms all paired devices have acknowledged the tombstone beyond the revocation frontier.

### Threat 11: Malicious Agent Subprocess Compromise
- **Attack Vector**: An external ACP agent tries to access sync keys or trigger remote synchronization.
- **Mitigation**:
  - Strict IPC privilege separation: Sync operations and device keys are exclusively owned by `altior-core`. ACP agent subprocesses run with zero network access and zero access to Vault keys or discovery tokens.

---

## 4. Executable Test Specifications (Pre-Release Acceptance)

Before sync is enabled, the following four deterministic test suites must be implemented and passing:

1. **`test_three_device_offline_convergence`**:
   - Devices A, B, and C pair.
   - Device C goes offline.
   - Device A edits Document 1; Device B edits Document 1 concurrently.
   - Device A forgets Memory 1; Device B adds Memory 2.
   - All devices reconnect via Relay; all three must converge to 100% identical state and digest with zero loss.

2. **`test_session_reconstruction_nonce_uniqueness`**:
   - Recreate 1,000 sequential `Session` instances between the same device pair with intentional simulated process kills.
   - Assert with a bloom filter / hash set that across all 100,000 generated ciphertexts, **zero duplicate nonces** occur under the same key.

3. **`test_relay_zero_knowledge_audit`**:
   - Capture all byte frames received by `altior-relay` during an active multi-turn session with memory recall.
   - Run entropy checks and string scanning; assert that zero plaintext substrings (user prompts, memories, agent outputs, secret refs) appear in relay memory or network frames.

4. **`test_stale_device_tombstone_non_resurrection`**:
   - Simulate a device returning after 30 days of offline dormancy.
   - Ensure that forgotten facts remain deleted, and that the stale device does not resurrect forgotten records upon re-sync.

---

## 5. Decision Consequences

- **Positive**: Eliminates any risk of shipping broken or insecure encryption; ensures Altior personal knowledge stays strictly private and offline-first; prevents keystream reuse vulnerabilities.
- **Trade-off**: Multi-device synchronization remains disabled in the current desktop release; users operate in single-vault, local-first mode until the full security audit and ratcheting protocol land in P3.
