# Security model

## Protected assets

- personality and long-term memory
- conversation and provenance excerpts
- project knowledge and attachments
- Vault/device private keys
- provider credentials and agent auth state
- local project files and terminal authority

## Trust boundaries

- Desktop UI is less privileged than Core.
- Agent subprocesses and MCP servers are untrusted.
- Relay data is hostile until authenticated, decrypted, and schema-validated.
- A paired device may become compromised and must be revocable.
- Retrieved documents and memories are data, not trusted instructions.
- Context prompt assembly enforces a strict 3-tier trust hierarchy: Identity Documents (Authoritative) > Current User Prompt > Retrieved Memories (Untrusted Passive Reference). Memory content is sanitized to prevent header breakout or authority spoofing.

## Required controls

- OS secret-store integration for private keys and provider secrets
- authenticated local IPC with per-launch capability token
- explicit project roots and path-containment checks
- cross-project memory scope containment: project memories are strictly prohibited from leaking into other projects or projectless threads
- four-tier retention semantics distinguishing logical tombstones from historical audit logs and cryptographic erasure
- command permissions bound to exact active turns
- encrypted, authenticated, replay-protected sync envelopes
- bounded message, document, attachment, and decompression sizes
- sensitive-value redaction before logs and crash reports
- device revocation, data-key rotation, and recovery-key export
- dependency audit, signed releases, and reproducible migration tests

## Prohibited shortcuts

- plaintext secrets in config files or environment snapshots
- relay-side decryption
- trusting filenames, MIME types, ACP metadata, or MCP schemas without validation
- shell command construction from concatenated untrusted strings
- automatic prompt resend after indeterminate delivery
- silent security fallback when keychain, signature, or permission checks fail

## Production Sync Barrier and Threat Model (ADR 0025 / F31)

Multi-device synchronization is **explicitly disabled** in current desktop releases
(`altior_core::SYNC_ENABLED = false` in `crates/altior-core/src/sync_gate.rs`). The P0.5 crypto and relay crates are pre-production reference
spikes. Before synchronization is opened to users, the 11 critical threat models specified in
`docs/decisions/0025-sync-production-gate-threat-model-and-safety-barriers.md` must be
completely satisfied:

1. **Static-Session Nonce Reuse Prevention**: Ephemeral random epoch generation + monotonic persistent counter reservations or ratcheting to prevent two-time pad keystream exposure.
2. **Contributory Key Exchange**: Enforce rejection of low-order Curve25519 points (RFC 7748 §6).
3. **Rollback Resilience**: Epoch salt validation preventing counter replay after disk/VM rollback.
4. **Revocation & Key Rotation**: Signed revocation certificates with immediate pairwise session termination and data-key re-encryption.
5. **Zero-Knowledge Relay Integrity**: Plaintext memories and conversation texts never reach the relay; 100% of body payloads and recovery keys remain encrypted.
6. **AEAD Additional Authenticated Data**: Protocol version, sender, receiver, epoch, and sequence bound into ChaCha20-Poly1305 AAD to prevent routing tampering.
7. **Relay Quota & DoS Bounds**: Hard physical limits on frame size (1 MiB), metadata (4 KiB), and queue depths (50 MiB/vault).
8. **Decompression & CRDT Bounds**: Hard limits on decompression ratios (max 10 MiB) and framed Automerge imports to prevent memory exhaustion attacks.
9. **Pull-Before-Push Stale Device Rejoin**: Offline devices returning after long dormancy must apply tombstone and revocation frontiers before publishing local edits.
10. **Tombstone Immortality**: Compaction is mathematically prohibited from deleting memory tombstones before all paired devices cross the tombstone frontier.
11. **Subprocess Privilege Isolation**: ACP agent subprocesses have zero access to sync keys, local IPC discovery tokens, or network transports.

See `docs/decisions/0025-sync-production-gate-threat-model-and-safety-barriers.md` for the
complete threat tree, mathematical failure modes, and required regression tests.

