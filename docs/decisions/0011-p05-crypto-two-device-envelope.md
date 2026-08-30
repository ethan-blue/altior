# ADR 0011: Standard-library crypto spike — the two-device envelope

Date: 2026-08-30 · Status: accepted · Scope: P0.5 crypto spike
(docs/IMPLEMENTATION_PLAN.md), feeding P1 device-to-device sync.

## Context

`docs/ARCHITECTURE.md` requires end-to-end encryption between the
user's own devices with the transport unable to read payloads. P0.5
must prove the cryptographic core with standard, boring primitives
before any relay or sync protocol builds on it.

The spike follows the Signal lineage, scaled down to two devices and
no ratchet, and borrows the specific practices that lineage
validated:

- **X25519 static-static ECDH** — the initial key agreement of every
  Signal-style session, minus the ratchet. RustCrypto's `x25519-dalek`
  (the curve25519-dalek audit lineage) with `StaticSecret`
  clamping per RFC 7748.
- **HKDF-SHA256 domain separation** — the KDF chain root of the Double
  Ratchet; here `info` names the protocol version, algorithm purpose,
  sender, and receiver in an ordered length-delimited context. This
  is the required domain separation between opposite directions.
- **ChaCha20-Poly1305** — Signal's and age's AEAD: constant-time
  software implementation, no AES hardware dependency, robust on the
  heterogeneous devices a multi-device personal system runs on.
- **Fully bound associated data** — Signal's associated-data practice:
  version, sender id, receiver id, and counter all feed the AEAD tag,
  so every routing claim an attacker could rewrite is authenticated.
- **Counter nonces** — the deterministic-nonce discipline (RFC 5116
  style): counters never repeat within a session, so nonce reuse
  under one key is structurally impossible and no RNG sits in the
  message path.
- **Sliding-window replay rejection** — the DTLS/DTLS-SRTP pattern
  (Signal's per-message counters are the same idea): accept anything
  above the highest seen counter, tolerate reordering within a
  64-delivery window, reject duplicates and anything older.
- **Signed pairing transcripts** — Signal's safety-number discipline
  reduced to software: both devices canonicalize identical transcript
  bytes (both ids, both public key pairs, canonical order) and sign
  them with Ed25519 identity keys (`ed25519-dalek`, `verify_strict`);
  substituted keys or renamed devices fail verification. Humans only
  decide to trust the channel the identities traveled over.

Two honesty notes that shaped the API: AEAD failures are one opaque
error on purpose (no oracle about which check tripped — callers must
not branch on *why* an envelope failed), and `aead::Error` implements
no `std::error::Error`, so the typed error keeps the failure for
`Debug` but chains no `source()`.

## Decision

### 1. New crate `altior-crypto`, outside the contract crates

Same placement pattern as ADR 0009/0010: `altior-domain` and
`altior-protocol` gain no dependencies. `altior-crypto` depends on
exactly `[chacha20poly1305, ed25519-dalek, hkdf, sha2, x25519-dalek,
zeroize]` — all RustCrypto / dalek first-party crates, no bespoke
crypto anywhere — and is pinned by the dependency-boundary test.

### 2. Device identity: one private, one public type

`DeviceKeys` (X25519 static secret + Ed25519 signing key, both
zeroizing on drop via the dalek `zeroize` feature, `Debug` redacted)
and `DeviceIdentity` (id + both public keys, freely shareable). A key
pair alone is not an identity: ids bind into the session HKDF
info/context, the envelope associated data, and the pairing transcript. Keys come from
caller-supplied 64-byte seeds (`DeviceKeys::from_seed`), so the
library has **no RNG anywhere** — the spike is fully deterministic,
and production OS randomness is `SecretStore`'s job in a later slice.
An all-zero seed is refused as the canonical "uninitialized" value.

### 3. Sessions are directional

`Session::outbound(local, peer)` and the peer's matching inbound
session derive the same directional key. HKDF info length-binds the
protocol version, session-key purpose, sender `DeviceId`, and receiver
`DeviceId`; reversing sender/receiver therefore derives a different
key even when both directions use counter nonce 1. Two
devices hold two sessions per side. An envelope sealed for one
direction does not open in the other — both key and AAD differ —
which the tests assert directly.

### 4. Envelope format v1: `version || counter || nonce || ct+tag`

One byte of version (forward-only, checked before any crypto), an
8-byte big-endian counter, a 12-byte nonce (4 zero bytes + the
counter, carried redundantly so the format stays self-describing),
then ciphertext and Poly1305 tag. Sealing is deterministic: same
seeds, same counter, same plaintext → identical bytes (asserted), and
counters advance monotonically from 1 so the counter-nonce can never
repeat under one key. Counter exhaustion is a typed error
(`CounterExhausted`) whose only correct response is rekeying.

`open` authenticates before it replays-checks — never act on an
unauthenticated counter — and then runs the 64-delivery sliding
window. Reordering within the window is accepted (a relay may deliver
out of order); duplicates and anything older than the window are
rejected with the counter attached.

### 5. Pairing: signatures over canonical transcript bytes

`DeviceId` rejects empty and over-128-byte labels, and pairing/session
construction rejects equal ids with typed errors. `DeviceKeys::from_seed`
returns a typed error for the reserved all-zero seed rather than
panicking. `PairingTranscript::new(a, b)` canonicalizes the two valid,
distinct identities by id order (byte-identical from either side), each device signs
those bytes, and `verify(signer, signature)` accepts only the exact
combination of transcript and presented identity. Malformed
signatures, wrong signers, mixed-and-matched key pairs from different
identities, and transcripts rebuilt after an id rename all fail as
the single `SignatureInvalid`.

## Alternatives considered

- **AES-256-GCM instead of ChaCha20-Poly1305** — equally sound with
  hardware support, but slower and harder to reason about
  constant-time-ness on devices without AES-NI. Rejected for
  ChaCha's software robustness; revisit never expected to be needed.
- **A full Double Ratchet now** — forward secrecy is the right
  long-term answer, but it needs message ordering machinery the spike
  does not otherwise require, and the P1 threat model (the user's own
  devices vs a curious relay) tolerates static keys for the first
  iteration. Named as the P1.2 follow-up, not skipped silently.
- **libsodium / dryoc wrappers** — a fine C lineage, but the RustCrypto
  + dalek stack is pure Rust, workspace-uniform, and has the audit
  history we need. Rejected to keep one crypto ecosystem.
- **Storing counters in a MAC'd header rather than the envelope** —
  equivalent here; keeping the counter in the clear part of the
  envelope makes replay debugging and P1 relay cursors simpler.

## Failure modes

- **Static-static without ratchet**: a leaked device key decrypts all
  history of sessions it participated in. Known, accepted for the
  spike, scheduled out via the ratchet follow-up.
- **All-zero shared secret** (low-order peer key): x25519-dalek 2.x
  removed the `was_contributory` probe; the spike does not check it.
  A maliciously chosen low-order public key yields an all-zero
  shared secret that HKDF stretches anyway. Pairing signatures are
  the actual defense (keys arrive authenticated), and the
  contributory check is named for the P1 hardening pass.
- **Replay window vs relay retention mismatch**: if a relay retains
  and re-delivers beyond 64 deliveries of backlog, legitimate stale
  mail is refused. The P1 relay design must keep fetch cursors and
  retention aligned with (larger than) the window; the constant is
  one line.
- **Counter persistence**: sessions are in-memory only in the spike;
  production must persist send counters with the key (`SecretStore`)
  or reuse nonces after restart — the classic AEAD restart failure.
  Named so P1 cannot miss it.
- **Device-id collision**: rejected before pairing or session
  derivation; length-prefixing still prevents concatenation ambiguity.

## Migration

P1.2 adopts this crate behind the `SecretStore` + sync-engine seam:
device keys persist in `SecretStore` (OS keychain or age-style file,
a separate ADR), `Session` gains persisted counters, and the relay
moves these opaque envelopes. The envelope format is
version-byte-guarded, so format changes are new versions, never
rewrites.

## Exit strategy

Swapping an algorithm (e.g., an AES-GCM variant or a PQ hybrid) is a
new KDF info string plus a new envelope version — old envelopes
remain openable by the version that wrote them. The public API
(`Session::seal/open`, `PairingTranscript`) is the seam; nothing
outside `altior-crypto` learns which AEAD runs inside.

## Revisit triggers

- P1.2 forward-secrecy work starts → ratchet design ADR, building on
  these sessions as the root.
- PQ hybrid standardization lands in the RustCrypto ecosystem →
  evaluate X25519+ML-KEM hybrid ECDH behind the same seam.
- A relay retention design exceeds the 64-delivery window → widen the
  window or move to persistent receive cursors.
