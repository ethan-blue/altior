//! The v2 encrypted sync envelope (ADR 0027 §4, on ADR 0011 roots).
//!
//! Hardens the ADR 0011 spike against the ADR 0025 threat tree:
//!
//! - **Session epochs** (128-bit, one per pairing) mix into every key, so
//!   a re-pairing derives keys unrelated to all previous envelopes.
//! - **Counter leases**: the sender draws send counters from durable
//!   reservation blocks supplied by the caller, so a process restart can
//!   never rewind into an already-used counter (the F31 nonce-reuse fix).
//! - **Turn ratchet**: every outbound sync batch ("turn") derives its key
//!   through a fresh X25519 ephemeral chained onto the previous turn's
//!   extracted secret. Compromising the static key reveals no earlier
//!   turn, and keys change every round trip.
//! - **Epoch sequence**: a per-direction monotonic counter, durable on
//!   both ends; inbound envelopes carrying a foreign epoch or an older
//!   sequence fail closed before any cryptography — the disk-rollback
//!   defense.
//! - **Contributory check**: an all-zero shared secret (the x25519 result
//!   of a low-order peer key) is rejected at every derive point.
//!
//! Wire format v2 (big-endian):
//! `version(0x02) ‖ epoch_seq(u32) ‖ epoch_id(16) ‖ turn_no(u64) ‖
//!  counter(u64) ‖ eph_pub(32) ‖ nonce(12) ‖ ct+tag`
//!
//! The ephemeral public key rides on every envelope (zero-padded on turn
//! 0, whose key is the static-static root): it authenticates through the
//! derived key itself — a swapped key fails AEAD closed — so it is
//! deliberately not part of the associated data. Everything else a relay
//! could rewrite is bound into the AEAD tag. Nonces reuse v1's
//! 4-zero-bytes-plus-counter shape; uniqueness under a key comes from
//! monotonic counters within a turn and from the turn/epoch chaining
//! between keys.
//!
//! Turns are strictly ordered per direction: the receiver accepts only
//! the current or the immediately next turn number; a skipped turn fails
//! closed (`TurnMismatch`). Sync batches travel over the relay in order,
//! so a skipped turn means tampering or a broken peer — both warrant a
//! re-pair, never a best-effort open.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::device::{DeviceId, DeviceIdentity, DeviceKeys};
use crate::error::CryptoError;

/// The v2 wire version byte.
const ENVELOPE_VERSION: u8 = 2;

/// HKDF domain separation for the turn key.
const KDF_DOMAIN: &[u8] = b"altior/crypto/v2-turn-key";

/// HKDF domain separation for chaining one turn onto the previous one.
const CHAIN_DOMAIN: &[u8] = b"altior/crypto/v2-turn-chain";

const EPOCH_LEN: usize = 16;
const EPOCH_SEQ_LEN: usize = 4;
const TURN_LEN: usize = 8;
const COUNTER_LEN: usize = 8;
const EPH_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// version byte, then `epoch_seq`, `epoch`, `turn`, `counter`, `eph`, `nonce`, tag.
const MIN_ENVELOPE_LEN: usize =
    1 + EPOCH_SEQ_LEN + EPOCH_LEN + TURN_LEN + COUNTER_LEN + EPH_LEN + NONCE_LEN + 16;

/// A 128-bit session epoch, established once per pairing (ADR 0027 §4.1).
///
/// Production draws it from the OS CSPRNG inside `SecretStore`; tests
/// inject fixed bytes. The all-zero value is rejected as the canonical
/// "uninitialized" marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEpoch([u8; EPOCH_LEN]);

impl SessionEpoch {
    /// An epoch from raw bytes.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EpochAllZero`] for the reserved all-zero value.
    pub fn from_bytes(bytes: [u8; EPOCH_LEN]) -> Result<Self, CryptoError> {
        if bytes == [0; EPOCH_LEN] {
            return Err(CryptoError::EpochAllZero);
        }
        Ok(Self(bytes))
    }

    /// Raw epoch bytes (wire form).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; EPOCH_LEN] {
        &self.0
    }
}

/// A durable reservation of the send-counter space: `[start, end)`.
///
/// The caller commits the reservation *before* sealing anything with it
/// (that ordering is what makes crash recovery safe), and hands the next
/// block over once the current one is exhausted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CounterBlock {
    /// First counter this block may hand out.
    pub start: u64,
    /// One past the last counter this block may hand out.
    pub end: u64,
}

/// The durable counter-reservation port (ADR 0027 §4.2).
///
/// `altior-crypto` stays dependency-free: `altior-storage` provides the
/// SQLite-backed implementation, tests an in-memory one. Implementations
/// must be crash-safe — a block returned here is committed.
pub trait CounterLease {
    /// Reserves the next block of `block_size` counters.
    ///
    /// # Errors
    ///
    /// Implementation-defined I/O failure. The session fails closed; it
    /// never seals with an unreserved counter.
    fn reserve(&mut self, block_size: u64) -> std::io::Result<CounterBlock>;
}

/// Default reservation block: large enough that production leases
/// rarely, small enough that tests exercise block rotation cheaply.
pub const COUNTER_BLOCK_SIZE: u64 = 10_000;

/// The resumable state of one session direction (ADR 0027 §4.3): the
/// chained intermediate and the turn it belongs to. Key material -
/// production persists it only through the OS secret store (ADR 0026),
/// never in SQLite or logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainSnapshot {
    /// The chained intermediate at the current turn.
    pub chain_ikm: [u8; 32],
    /// The turn the intermediate belongs to.
    pub turn_no: u64,
}

/// One direction of a v2 encrypted channel between two paired devices.
///
/// As in v1, the key is directional: the HKDF info binds the wire sender
/// and receiver, and two devices hold one outbound and one inbound
/// session per peer.
pub struct SessionV2 {
    direction: Direction,
    /// The wire sender of the envelopes this session handles.
    sender: DeviceId,
    /// The wire receiver of the envelopes this session handles.
    receiver: DeviceId,
    epoch: SessionEpoch,
    epoch_seq: u32,
    // Outbound state:
    counter_block: Option<CounterBlock>,
    next_counter: u64,
    /// Public half of the current turn's ephemeral; turn 0 seals zeros.
    turn_eph_pub: [u8; EPH_LEN],
    /// The peer's static X25519 key, for chaining new turns.
    peer_x25519: Option<[u8; EPH_LEN]>,
    // Inbound state:
    /// Our static secret, for chaining the peer's turn ephemerals.
    local_static: Option<StaticSecret>,
    // Both directions:
    chain_ikm: Option<Zeroizing<[u8; 32]>>,
    turn_no: u64,
    // Set only on a restored sending session whose turn was
    // interrupted: the ephemeral secret of that turn is gone, so a
    // fresh open_turn is required before any seal (fail closed, never
    // wrong-ciphertext).
    needs_fresh_turn: bool,
    replay: crate::session::ReplayWindow,
}

#[derive(Debug, Eq, PartialEq)]
enum Direction {
    Outbound,
    Inbound,
}

impl fmt::Debug for SessionV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionV2")
            .field("direction", &self.direction)
            .field("sender", &self.sender)
            .field("receiver", &self.receiver)
            .field("epoch", &self.epoch)
            .field("epoch_seq", &self.epoch_seq)
            .field("counter_block", &self.counter_block)
            .field("turn_eph_pub", &"<redacted>")
            .field("peer_x25519", &"<redacted>")
            .field("local_static", &"<redacted>")
            .field("chain_ikm", &"<redacted>")
            .field("replay", &self.replay)
            .field("turn_no", &self.turn_no)
            .field("needs_fresh_turn", &self.needs_fresh_turn)
            .field("next_counter", &self.next_counter)
            .field("key material", &"<redacted>")
            .finish()
    }
}

impl SessionV2 {
    /// The sending session from `local` to `peer`, bound to `epoch` at
    /// `epoch_seq`.
    ///
    /// The first counter block is reserved from `lease` immediately; the
    /// session never seals with an unreserved counter and stops at the
    /// block end until [`SessionV2::set_counter_block`] installs the next
    /// one. Turn 0 (the static-static root key) is open on construction.
    ///
    /// # Errors
    ///
    /// [`CryptoError::DeviceIdCollision`] for equal ids;
    /// [`CryptoError::DegenerateSharedSecret`] for a low-order peer key;
    /// lease failures propagate as [`CryptoError::CounterLeaseFailed`].
    pub fn outbound(
        local: &DeviceKeys,
        peer: &DeviceIdentity,
        epoch: SessionEpoch,
        epoch_seq: u32,
        lease: &mut dyn CounterLease,
    ) -> Result<Self, CryptoError> {
        if local.id() == peer.id() {
            return Err(CryptoError::DeviceIdCollision {
                id: local.id().to_string(),
            });
        }
        let block = lease
            .reserve(COUNTER_BLOCK_SIZE)
            .map_err(|source| CryptoError::CounterLeaseFailed { source })?;
        if block.start == 0 {
            return Err(CryptoError::CounterBlockRewind {
                block_start: block.start,
                next_counter: 0,
            });
        }
        let static_shared = shared_secret_from(local.static_secret(), peer.x25519_public())?;
        Ok(Self {
            direction: Direction::Outbound,
            sender: local.id().clone(),
            receiver: peer.id().clone(),
            epoch,
            epoch_seq,
            counter_block: Some(block),
            next_counter: block.start,
            turn_eph_pub: [0; EPH_LEN],
            peer_x25519: Some(peer.x25519_public()),
            local_static: None,
            chain_ikm: Some(static_shared),
            turn_no: 0,
            needs_fresh_turn: false,
            replay: crate::session::ReplayWindow::default(),
        })
    }

    /// The receiving session for what `peer` sends to `local`.
    ///
    /// # Errors
    ///
    /// Same derivation failures as [`SessionV2::outbound`].
    pub fn inbound(
        local: &DeviceKeys,
        peer: &DeviceIdentity,
        epoch: SessionEpoch,
        epoch_seq: u32,
    ) -> Result<Self, CryptoError> {
        if local.id() == peer.id() {
            return Err(CryptoError::DeviceIdCollision {
                id: local.id().to_string(),
            });
        }
        let static_shared = shared_secret_from(local.static_secret(), peer.x25519_public())?;
        Ok(Self {
            direction: Direction::Inbound,
            sender: peer.id().clone(),
            receiver: local.id().clone(),
            epoch,
            epoch_seq,
            counter_block: None,
            next_counter: 0,
            turn_eph_pub: [0; EPH_LEN],
            peer_x25519: None,
            local_static: Some(local.static_secret().clone()),
            chain_ikm: Some(static_shared),
            turn_no: 0,
            needs_fresh_turn: false,
            replay: crate::session::ReplayWindow::default(),
        })
    }

    /// Reconstructs the sending session from a durable chain snapshot
    /// (OS secret store, ADR 0027 §4.3). The counter space continues
    /// from `lease`; [`SessionV2::open_turn`] must run before the first
    /// seal - the interrupted turn's ephemeral is gone by design, and
    /// sealing a restored turn fails closed.
    ///
    /// # Errors
    ///
    /// Same derivation failures as [`SessionV2::outbound`].
    pub fn restore_outbound(
        local: &DeviceKeys,
        peer: &DeviceIdentity,
        epoch: SessionEpoch,
        epoch_seq: u32,
        snapshot: &ChainSnapshot,
        lease: &mut dyn CounterLease,
    ) -> Result<Self, CryptoError> {
        let restored = Self::outbound(local, peer, epoch, epoch_seq, lease)?;
        Ok(Self {
            chain_ikm: Some(Zeroizing::new(snapshot.chain_ikm)),
            turn_no: snapshot.turn_no,
            needs_fresh_turn: snapshot.turn_no > 0,
            ..restored
        })
    }

    /// Reconstructs the receiving session from a durable chain snapshot.
    /// Late envelopes of already-passed turns stay rejected; the current
    /// turn continues to open with the restored intermediate.
    ///
    /// # Errors
    ///
    /// Same derivation failures as [`SessionV2::inbound`].
    pub fn restore_inbound(
        local: &DeviceKeys,
        peer: &DeviceIdentity,
        epoch: SessionEpoch,
        epoch_seq: u32,
        snapshot: &ChainSnapshot,
    ) -> Result<Self, CryptoError> {
        let restored = Self::inbound(local, peer, epoch, epoch_seq)?;
        Ok(Self {
            chain_ikm: Some(Zeroizing::new(snapshot.chain_ikm)),
            turn_no: snapshot.turn_no,
            ..restored
        })
    }

    /// The resumable chain state of this (sending) session.
    #[must_use]
    pub fn outbound_chain(&self) -> Option<ChainSnapshot> {
        self.chain_ikm.as_ref().map(|ikm| ChainSnapshot {
            chain_ikm: **ikm,
            turn_no: self.turn_no,
        })
    }

    /// The resumable chain state of this (receiving) session.
    #[must_use]
    pub fn inbound_chain(&self) -> Option<ChainSnapshot> {
        self.chain_ikm.as_ref().map(|ikm| ChainSnapshot {
            chain_ikm: **ikm,
            turn_no: self.turn_no,
        })
    }

    /// Installs the next counter block, after the caller durably
    /// committed it.
    ///
    /// # Errors
    ///
    /// [`CryptoError::CounterBlockRewind`] if the block starts below the
    /// next counter this session would hand out — a caller bug that
    /// would otherwise reuse counters. Fails closed, never seals.
    pub fn set_counter_block(&mut self, block: CounterBlock) -> Result<(), CryptoError> {
        if block.end <= block.start || block.start == 0 || block.start < self.next_counter {
            return Err(CryptoError::CounterBlockRewind {
                block_start: block.start,
                next_counter: self.next_counter,
            });
        }
        self.counter_block = Some(block);
        Ok(())
    }

    /// Opens a new ratchet turn from a caller-supplied ephemeral seed
    /// (production: OS CSPRNG via `SecretStore`; tests: fixed bytes).
    /// Every envelope sealed after this belongs to the new turn and
    /// carries its ephemeral public key. The ephemeral secret is dropped
    /// at the end of the call (zeroized by the dalek type) — only the
    /// chained intermediate survives.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EphemeralSeedAllZero`] for the reserved zero seed;
    /// [`CryptoError::DegenerateSharedSecret`] for a degenerate ECDH.
    ///
    /// # Panics
    ///
    /// Panics if the internal chain or peer key is `None` on an outbound
    /// session — structurally impossible after the constructor, so the
    /// panic marks a programming error, not input.
    pub fn open_turn(&mut self, ephemeral_seed: [u8; EPH_LEN]) -> Result<(), CryptoError> {
        if self.direction != Direction::Outbound {
            return Err(CryptoError::TurnNotOpen);
        }
        if ephemeral_seed == [0; EPH_LEN] {
            return Err(CryptoError::EphemeralSeedAllZero);
        }
        let peer_x25519 = self.peer_x25519.expect("outbound carries the peer key");
        let eph_secret = StaticSecret::from(ephemeral_seed);
        let eph_pub = PublicKey::from(&eph_secret).to_bytes();
        let shared = shared_secret_from(&eph_secret, peer_x25519)?;
        let previous = self.chain_ikm.take().expect("outbound always chains");
        self.chain_ikm = Some(chain_turn(&previous, &shared));
        self.turn_eph_pub = eph_pub;
        self.turn_no += 1;
        self.needs_fresh_turn = false;
        Ok(())
    }

    /// Seals `plaintext` into a v2 envelope under the current turn key.
    ///
    /// # Errors
    ///
    /// [`CryptoError::CounterBlockExhausted`] when the leased block ran
    /// out — lease the next block and continue; never reuse a counter.
    ///
    /// # Panics
    ///
    /// Panics if the internal chain is `None` — structurally impossible
    /// after the constructor, so the panic marks a programming error.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if self.needs_fresh_turn {
            return Err(CryptoError::TurnNotOpen);
        }
        let block = self
            .counter_block
            .ok_or(CryptoError::CounterBlockExhausted)?;
        let counter = self.next_counter;
        if counter >= block.end {
            return Err(CryptoError::CounterBlockExhausted);
        }
        self.next_counter += 1;
        if self.next_counter >= block.end {
            self.counter_block = None;
        }

        let chain = self.chain_ikm.as_ref().expect("constructed with a chain");
        let turn_key = turn_key(
            chain,
            &self.epoch,
            self.epoch_seq,
            self.turn_no,
            &self.sender,
            &self.receiver,
        );

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[NONCE_LEN - COUNTER_LEN..].copy_from_slice(&counter.to_be_bytes());
        let aad = associated_data(
            self.epoch_seq,
            &self.epoch,
            self.turn_no,
            counter,
            &self.sender,
            &self.receiver,
        );
        let cipher = ChaCha20Poly1305::new(Key::from_slice(turn_key.as_ref()));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|source| CryptoError::EnvelopeSeal { source })?;

        let mut envelope = Vec::with_capacity(MIN_ENVELOPE_LEN + ciphertext.len());
        envelope.push(ENVELOPE_VERSION);
        envelope.extend_from_slice(&self.epoch_seq.to_be_bytes());
        envelope.extend_from_slice(self.epoch.as_bytes());
        envelope.extend_from_slice(&self.turn_no.to_be_bytes());
        envelope.extend_from_slice(&counter.to_be_bytes());
        envelope.extend_from_slice(&self.turn_eph_pub);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    /// Authenticates and decrypts an inbound v2 envelope.
    ///
    /// Check order: shape, then epoch binding (cheap gate on which key
    /// applies), then turn ordering, then AEAD, then the replay window.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EnvelopeTruncated`] / `EnvelopeVersion` for
    /// malformed input; `EpochMismatch` for a foreign epoch or sequence;
    /// `TurnMismatch` for a skipped or already-passed turn;
    /// `EnvelopeOpen` on any authentication failure; `ReplayRejected`
    /// for a duplicate counter within the current turn.
    ///
    /// # Panics
    ///
    /// Panics when a fixed-size slice misaligns after the length check —
    /// structurally impossible, so the panic marks a programming error.
    pub fn open(&mut self, envelope: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if envelope.len() < MIN_ENVELOPE_LEN {
            return Err(CryptoError::EnvelopeTruncated {
                size: envelope.len(),
            });
        }
        if envelope[0] != ENVELOPE_VERSION {
            return Err(CryptoError::EnvelopeVersion { found: envelope[0] });
        }
        let mut cursor = 1;
        let mut take = |buf: &[u8], len: usize| -> Vec<u8> {
            let chunk = buf[cursor..cursor + len].to_vec();
            cursor += len;
            chunk
        };
        let wire_epoch_seq =
            u32::from_be_bytes(take(envelope, EPOCH_SEQ_LEN).try_into().expect("4 bytes"));
        let mut wire_epoch = [0; EPOCH_LEN];
        wire_epoch.copy_from_slice(&take(envelope, EPOCH_LEN));
        let turn_no = u64::from_be_bytes(take(envelope, TURN_LEN).try_into().expect("8 bytes"));
        let counter = u64::from_be_bytes(take(envelope, COUNTER_LEN).try_into().expect("8 bytes"));
        let mut wire_eph = [0; EPH_LEN];
        wire_eph.copy_from_slice(&take(envelope, EPH_LEN));
        let nonce_bytes: [u8; NONCE_LEN] = take(envelope, NONCE_LEN)
            .try_into()
            .expect("12 nonce bytes");

        if wire_epoch_seq != self.epoch_seq || wire_epoch != *self.epoch.as_bytes() {
            return Err(CryptoError::EpochMismatch {
                expected_seq: self.epoch_seq,
                found_seq: wire_epoch_seq,
            });
        }

        // Turn ordering: accept the current turn, or prepare exactly one
        // chaining step. Anything else fails closed. The candidate chain
        // commits only after the AEAD accepts the envelope — a forged or
        // corrupted turn header must never poison the receiver's state
        // (ADR 0011: never act on unauthenticated counters).
        if turn_no != self.turn_no && turn_no != self.turn_no + 1 {
            return Err(CryptoError::TurnMismatch {
                current: self.turn_no,
                found: turn_no,
            });
        }
        let candidate_chain = if turn_no == self.turn_no + 1 {
            let local_static = self.local_static.as_ref().expect("inbound");
            let shared = shared_secret_from(local_static, wire_eph)?;
            let previous = self.chain_ikm.as_ref().expect("inbound chains");
            Some(chain_turn(previous, &shared))
        } else {
            None
        };

        let chain = candidate_chain
            .as_ref()
            .unwrap_or_else(|| self.chain_ikm.as_ref().expect("turn established a chain"));
        let turn_key = turn_key(
            chain,
            &self.epoch,
            self.epoch_seq,
            turn_no,
            &self.sender,
            &self.receiver,
        );
        let aad = associated_data(
            self.epoch_seq,
            &self.epoch,
            turn_no,
            counter,
            &self.sender,
            &self.receiver,
        );
        let cipher = ChaCha20Poly1305::new(Key::from_slice(turn_key.as_ref()));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &envelope[cursor..],
                    aad: &aad,
                },
            )
            .map_err(|source| CryptoError::EnvelopeOpen { source })?;

        // Authenticated: only now may the turn advance commit.
        if let Some(committed) = candidate_chain {
            self.chain_ikm = Some(committed);
            self.turn_no = turn_no;
            // A fresh turn starts a fresh counter space under a fresh
            // key; the old window's state must not leak forward.
            self.replay = crate::session::ReplayWindow::default();
        }
        if !self.replay.accept(counter) {
            return Err(CryptoError::ReplayRejected { counter });
        }
        Ok(plaintext)
    }

    /// The next counter `seal` will hand out.
    #[must_use]
    pub fn next_counter(&self) -> u64 {
        self.next_counter
    }

    /// The currently open turn number.
    #[must_use]
    pub fn turn_no(&self) -> u64 {
        self.turn_no
    }
}

fn shared_secret_from(
    secret: &StaticSecret,
    peer_public_bytes: [u8; EPH_LEN],
) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let shared = secret.diffie_hellman(&PublicKey::from(peer_public_bytes));
    if shared.as_bytes() == &[0; 32] {
        return Err(CryptoError::DegenerateSharedSecret);
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(shared.as_bytes());
    Ok(out)
}

/// Chains turn `n` onto turn `n-1`: `ikm_n = Extract(salt = ikm_{n-1},
/// ikm = ECDH(eph_n, peer_static))` (ADR 0027 §4.3).
fn chain_turn(previous_ikm: &[u8; 32], turn_shared: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(previous_ikm), turn_shared);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(CHAIN_DOMAIN, out.as_mut()).expect("32-byte prk");
    out
}

/// The per-turn AEAD key, bound to the epoch, sequence, turn, and both
/// directions.
fn turn_key(
    chain_ikm: &[u8; 32],
    epoch: &SessionEpoch,
    epoch_seq: u32,
    turn_no: u64,
    sender: &DeviceId,
    receiver: &DeviceId,
) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, chain_ikm);
    let mut info = Vec::from(KDF_DOMAIN);
    info.push(ENVELOPE_VERSION);
    info.extend_from_slice(epoch.as_bytes());
    info.extend_from_slice(&epoch_seq.to_be_bytes());
    info.extend_from_slice(&turn_no.to_be_bytes());
    push_id(&mut info, sender);
    push_id(&mut info, receiver);
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(&info, key.as_mut()).expect("32-byte okm");
    key
}

/// Associated data: every routing claim a relay could rewrite.
fn associated_data(
    epoch_seq: u32,
    epoch: &SessionEpoch,
    turn_no: u64,
    counter: u64,
    sender: &DeviceId,
    receiver: &DeviceId,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(
        1 + EPOCH_SEQ_LEN + EPOCH_LEN + TURN_LEN + COUNTER_LEN + 2 * (8 + DeviceId::MAX_LEN),
    );
    aad.push(ENVELOPE_VERSION);
    aad.extend_from_slice(&epoch_seq.to_be_bytes());
    aad.extend_from_slice(epoch.as_bytes());
    aad.extend_from_slice(&turn_no.to_be_bytes());
    aad.extend_from_slice(&counter.to_be_bytes());
    push_id(&mut aad, sender);
    push_id(&mut aad, receiver);
    aad
}

/// Length-prefixed id bytes (same shape as v1), so id pairs never alias.
fn push_id(buf: &mut Vec<u8>, id: &DeviceId) {
    buf.extend_from_slice(&(id.as_str().len() as u64).to_be_bytes());
    buf.extend_from_slice(id.as_str().as_bytes());
}
