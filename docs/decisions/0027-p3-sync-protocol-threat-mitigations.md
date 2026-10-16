# ADR 0027: P3 Personal Vault Sync Protocol — Session Hardening, Device Lifecycle, and the Sync Engine

- **Date:** 2026-10-16
- **Status:** Accepted (design of record for P3 implementation; the ADR 0025
  production barrier stays closed until its acceptance evidence exists)
- **Scope:** `altior-crypto`, `altior-relay`, `altior-domain`,
  `altior-storage`, `altior-core` (sync module)
- **Builds on:** ADR 0010 (CRDT), 0011 (crypto envelope), 0012 (relay queue),
  0025 (production gate, threat model), 0026 (OS secret store)

## 1. Context

P3 turns the two P0.5 spikes into the Personal Vault sync implementation.
ADR 0011/0012 named, but deliberately deferred, every capability ADR 0025
later made mandatory:

- F31: reconstructed sessions reset `send_counter` to 0 → **nonce reuse under
  an unchanged key** (two-time-pad keystream leakage).
- No contributory check: a low-order peer key yields an all-zero shared
  secret that HKDF happily stretches.
- No counter persistence: any process restart with static keys is F31.
- No session epochs: a long-lived static-static key covers all history —
  no forward secrecy, no per-run key separation.
- No revocation: a stolen paired device reads the vault forever.
- No pull-before-push: a stale device can resurrect forgotten facts.
- Relay: unbounded dedupe id set, single-consumer cursors, retention constants
  not yet aligned with ADR 0025 numbers.

ADR 0025 requires the 11 threat mitigations plus four deterministic test
suites before `SYNC_ENABLED` may flip. This ADR is the design of record for
that work. It changes no released durable format: sync state is new storage,
and envelope **v2** is a new version, never a rewrite of v1.

## 2. Decision overview

Three layers, each independently testable, all behind
`altior_core::sync_gate::ensure_sync_allowed()`:

1. **Session hardening** (`altior-crypto`, envelope v2):
   session epoch + turn ratchet + persistent counter floors + zero-shared-
   secret rejection. Kills threats 1, 2, 3.
2. **Device lifecycle** (`altior-domain` + `altior-storage`):
   vault device registry, pairing wrap of the vault data key, revocation
   certificates, key rotation. Kills threat 4, grounds threat 10.
3. **Sync engine** (`altior-core::sync`):
   journal-event replication over the relay with pull-before-push, ack
   cursors, and a tombstone frontier that compaction cannot pass.
   Kills threats 9, 10; composes with 5–8, 11.

## 3. Threat → mechanism → evidence map

| # | Threat (ADR 0025) | Mechanism | Test |
|---|---|---|---|
| 1 | Nonce reuse on restart | Epoch-mixed keys (HKDF info) + persisted counter reservation blocks | `test_session_reconstruction_nonce_uniqueness` (suite 2) |
| 2 | Low-order / non-contributory ECDH | Reject all-zero shared secret at every derive point; pairing signatures remain the authentication of record | unit test with degenerate peer key |
| 3 | Disk rollback / VM snapshot | Per-direction monotonic `epoch_seq`; inbound envelope with `epoch_seq <= max_seen` is rejected pre-crypto | unit tests on epoch state machine |
| 4 | Stolen device | Revocation certificate (Ed25519, offline recovery key) as journal event; sessions wiped, document keys rotated | suite 1 + revocation unit tests |
| 5 | Relay eavesdropping | Unchanged: sealed sender, opaque bytes; now with zero-knowledge audit on v2 frames | `test_relay_zero_knowledge_audit` (suite 3) |
| 6 | Relay tampering | AAD extends with epoch and turn header; any mutation fails Poly1305 closed | unit tests (bit-flip table) |
| 7 | Relay exhaustion | Constants: 1 MiB envelope, 4 KiB addressing, 50 MiB per-bucket quota, 14-day tick expiry, bounded dedupe LRU | relay unit tests |
| 8 | Decompression / CRDT bombs | No compression on the sync plane; Automerge imports stay under ADR 0010 bounded decode (10 MiB ceiling, typed `CrdtDecodeError`) | existing crdt tests; engine rejects oversized imports |
| 9 | Stale-device resurrection | Pull-before-push: a rejoining device applies the tombstone/revocation frontier before its writes are admitted | `test_stale_device_tombstone_non_resurrection` (suite 4) |
| 10 | Tombstone immortality | Compaction may reclaim tombstones only below the global ack frontier (every paired device's durable cursor) | suite 1 + engine unit tests |
| 11 | Agent subprocess compromise | Structural: sync keys and relay access live only in `altior-core`; ACP subprocess path gains no sync capability (capability set never includes it) | capability negotiation tests (existing) |

## 4. Session hardening (envelope v2)

### 4.1 Session epochs

Every pairing (or re-pairing) between two devices establishes a 128-bit
`epoch_id` (CSPRNG in production, injected deterministic RNG in tests). The
epoch is mixed into the HKDF info of the directional session key, next to
version, purpose, and the two device ids. Consequences:

- A restarted process derives the **same** key for the same epoch (static
  keys, same epoch) — so the epoch alone does not fix F31.
- A **re-pairing** (new epoch) yields an unrelated key even if every static
  key leaked into the old epoch's envelopes.

Epochs therefore pair with counter floors, not replace them.

### 4.2 Persistent counter floors (F31 fix)

Before any v2 envelope is sealed, the sender reserves a counter block from
durable storage through the `CounterLease` port:

```text
trait CounterLease { fn reserve(&mut self, block: u64) -> io::Result<(u64, u64)>; }
```

Blocks of 10 000. `Session::outbound` gains a counter floor parameter; the
session starts at `floor` instead of 0 and errors when it reaches the block
end (caller reserves the next block — crash-safe because reservation is a
single durable write that happens *before* any use). Replay of an old block
after disk rollback is caught by §4.4's epoch-sequence check on the peer.

`altior-crypto` stays dependency-free: the port is owned by the caller
(`altior-storage` provides the SQLite-backed implementation; tests use an
in-memory one).

### 4.3 Turn ratchet (forward secrecy)

Full Double Ratchet deferred; ADR 0025's requirement — *new keys every round
trip* — is met by a **turn ratchet**:

- A *turn* is one outbound sync batch. The sender generates a fresh X25519
  ephemeral per turn; the envelope v2 header carries `epoch_id`, `epoch_seq`,
  `turn_no`, and the ephemeral public key (all inside the AEAD's reach —
  the header is AAD-adjacent: `epoch_id`, ids, counters authenticate; the
  ephemeral key authenticates via the derived key itself).
- The v2 session key = HKDF(static shared, info = domain ‖ epoch ‖ turn_no ‖
  sender ‖ receiver), where *static shared* for turn *n > 0* is
  `ECDH(sender_ephemeral_n, receiver_static)` chained with the previous
  turn key: `ikm_n = HKDF-Extract(ikm_{n-1}, ECDH(eph_n, peer_static))`.
  Compromise of the static key reveals future turns only until the next
  ratchet step and never reveals earlier turns (ephemeral secrets are
  zeroized).
- The receiver keeps per-sender turn state; late turns within the replay
  window are opened with retained turn keys, older ones are rejected exactly
  like today's replay window.

This is deliberately not Signal's full per-message ratchet: sync traffic is
batch-shaped, and the batch boundary is the natural ratchet granularity.
A full Double Ratchet remains the revisit trigger if per-message forward
secrecy is ever required.

### 4.4 Epoch sequence (rollback detection)

Each direction tracks `epoch_seq`: a monotonic counter incremented on every
re-pairing, durable on both ends. Inbound envelopes carry `epoch_seq`; a
value `<= max_observed` for that sender is rejected before any crypto. A
rolled-back disk emits stale `epoch_seq` values and fails closed; recovery
is re-pairing (human-verified fingerprint), never automatic.

### 4.5 Contributory check

After every `diffie_hellman`, reject an all-zero shared secret with a typed
error (`CryptoError::DegenerateSharedSecret`). x25519-dalek 2.x removed
`was_contributory`; the all-zero check is the RFC 7748 §6 conservative
subset. Pairing signatures (ADR 0011) remain the actual authentication of
peer keys; this check bounds the damage of a hostile *unauthenticated* key
reaching a derive path (e.g. future three-device code paths).

### 4.6 Envelope v2 format

`version(0x02) ‖ epoch_seq(u32) ‖ epoch_id(16) ‖ turn_no(u64) ‖ send_counter(u64) ‖
eph_pub(32, turn 0 only) ‖ nonce(12) ‖ ct+tag`. Version byte checked before
any crypto. v1 envelopes remain openable only by v1 sessions (spike code);
no cross-version downgrade is possible because the version byte is part of
the authenticated context and the key derivation domains differ.

## 5. Device lifecycle

Domain (`altior-domain`, no crypto dependency — bytes and rules only):

- `VaultDeviceRecord { device_id, added_at, added_by, status: Active | Revoked { revoked_at, cert_ref }, epoch_seq }`.
- `RevocationCertificate { device_id, revoked_at, recovery_signature }` —
  Ed25519 signature by the vault recovery key over the canonical tuple.
  Enters the journal as a signed event with highest replication priority.
- Rotation: on revocation, the vault document key is re-wrapped for every
  still-`Active` device; the revoked device's pairwise sessions are dropped
  at both ends on applying the event.

Pairing reuses ADR 0011's `PairingTranscript` (fingerprint display is the
existing human check) and wraps the vault data key to the new device's
X25519 static key via an AEAD seal whose AAD binds both device ids and the
transcript digest. Private keys never leave the OS secret store (ADR 0026);
the wrapped vault key is ordinary ciphertext at rest.

Storage: new tables (`vault_device`, `sync_counter_lease`, `sync_ack_cursor`,
`tombstone_frontier`) in a fresh migration; no existing table changes, no
released-format impact (sync is not enabled).

## 6. Sync engine

`altior-core::sync`, constructed only behind the sync gate:

- **Wire unit** = signed journal event batch, sealed in a v2 envelope,
  pushed to the recipient's relay bucket. Order = journal order; the relay's
  global sequence gives an unambiguous per-bucket cursor.
- **Pull-before-push (threat 9)**: a device's outbound queue is gated on
  having applied the peer's current tombstone/revocation frontier (fetched
  via a frontier digest exchange). Until then, local writes stay pending.
  Forgetting a memory while a peer is offline works; the peer's *stale*
  writes wait until it has pulled the tombstone.
- **Ack cursors (threat 10)**: each device durably records the frontier it
  has applied, per peer. Compaction (local journal pruning, tombstone
  GC) may only reclaim below `min(all peers' acked frontier)`. A peer with
  no durable ack pins the frontier forever — conservative, correct, and
  visible in sync diagnostics.
- **Fan-out**: per-device sub-cursors keyed by device id inside one vault
  bucket (the ADR 0012 named extension), so one user's three devices each
  track their own progress.

## 7. Relay alignment (threats 5–8)

Constants adopted from ADR 0025: envelope payload ≤ 1 MiB, addressing
metadata ≤ 4 KiB, per-bucket quota 50 MiB, expiry 14 days (logical ticks;
production maps ticks to server time), and a bounded dedupe-id LRU (the
ADR 0012 named hardening) so the seen-set cannot grow without bound.
Retention stays strictly larger than the crypto replay window; the engine
maps `FetchPage::Compacted` to snapshot resync, never to data loss.

## 8. Determinism and test plan

All four ADR 0025 suites run on deterministic fixtures: injected RNG
(`DeviceKeys::from_seed` + a `Rng` trait for epochs/ephemerals), the
in-memory relay with logical ticks, fake clocks, no network, no sleeps.

1. `test_three_device_offline_convergence` — engine test (suite 1).
2. `test_session_reconstruction_nonce_uniqueness` — 1 000 session
   reconstructions across simulated kills; hash-set asserts zero duplicate
   (key, nonce) pairs over ≥ 100 000 envelopes (suite 2).
3. `test_relay_zero_knowledge_audit` — byte-audit every relayed frame and
   relay-internal state for plaintext markers (suite 3).
4. `test_stale_device_tombstone_non_resurrection` — 30-tick-dormant device
   rejoins; forgotten facts stay forgotten (suite 4).

## 9. Alternatives considered

- **Full Double Ratchet now** — maximum forward secrecy, but per-message
  header machinery and out-of-order key soup that batch-shaped sync traffic
  does not need; the turn ratchet meets ADR 0025's stated bar. Revisit
  trigger recorded (§4.3).
- **TPM-backed monotonic counters (threat 3)** — hardware-dependent,
  platform-fragmented; the peer-side epoch-sequence check achieves the same
  fail-closed property cross-platform. Named for hardening later.
- **Putting the sync engine in a new crate** — rejected: the engine is the
  application coordinator's peer (ADR 0002), needs `altior-storage` +
  `altior-crypto` + the runtime, and gains nothing from crate isolation
  that a module boundary under the sync gate doesn't provide.
- **Persisting nonces instead of counters** — equivalent hardness but
  unbounded storage and no crash-safe block semantics; counter floors are
  the cheaper invariant.

## 10. Failure modes and consequences

- Counter-lease storage corruption → sessions refuse to seal (fail closed);
  recovery is re-pairing, never counter reuse.
- A lost relay cannot break convergence (at-least-once + replay windows +
  cursored catch-up), but pins compaction until acked — disk usage grows
  with an offline peer; surfaced in diagnostics.
- Revocation requires the offline recovery key; losing it means no revocation
  path (documented product behavior, recovery-key ADR to follow).
- Turn-key retention grows with the replay window only (bounded, 64 turns).

## 11. Migration / downgrade

All sync state is new and behind a disabled gate; no migration of released
data. Envelope v1 → v2 is a version split, not a rewrite; v1 code remains
as spike/tests only. If sync ships and must be withdrawn, `SYNC_ENABLED`
returns to false — devices simply stop syncing; local data is unaffected.

## 12. Exit strategy

The crypto seam is unchanged (ADR 0011): swapping the ratchet schedule or
AEAD is a new KDF info + envelope version. The relay seam is unchanged
(ADR 0012): the in-memory machine remains the test double for any server
implementation. The engine's ports (transport, counter lease, clock, RNG)
are the boundaries for every future replacement.
