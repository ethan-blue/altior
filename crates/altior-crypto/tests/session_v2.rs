//! `SessionV2` behaviour: the ADR 0027 §4 hardening contract.

use altior_crypto::{
    CounterBlock, CounterLease, DeviceId, DeviceIdentity, DeviceKeys, SessionEpoch, SessionV2,
};

const ALICE_SEED: [u8; 64] = [1; 64];
const BOB_SEED: [u8; 64] = [2; 64];
const ALICE_ID: &str = "alice-test";
const BOB_ID: &str = "bob-test";
const EPOCH_BYTES: [u8; 16] = [7; 16];
const TURN_SEED_1: [u8; 32] = [11; 32];
const TURN_SEED_2: [u8; 32] = [22; 32];

fn alice_keys() -> DeviceKeys {
    DeviceKeys::from_seed(ALICE_SEED, DeviceId::new(ALICE_ID).unwrap()).unwrap()
}

fn bob_keys() -> DeviceKeys {
    DeviceKeys::from_seed(BOB_SEED, DeviceId::new(BOB_ID).unwrap()).unwrap()
}

fn bob_identity() -> DeviceIdentity {
    bob_keys().public_identity()
}

fn test_epoch() -> SessionEpoch {
    SessionEpoch::from_bytes(EPOCH_BYTES).unwrap()
}

/// Deterministic lease: blocks of 64 so the tests exercise block rotation
/// without relying on the 10 000 production default.
/// Counters start at 1: the replay window (like v1) never accepts 0.
struct FixedLease {
    next: u64,
}

impl FixedLease {
    fn new() -> Self {
        Self { next: 1 }
    }
}

impl CounterLease for FixedLease {
    /// Deliberately hands out 64-wide blocks regardless of the requested
    /// size — the port does not promise the session preferred width, and
    /// narrow blocks exercise exhaustion cheaply.
    fn reserve(&mut self, _block_size: u64) -> std::io::Result<CounterBlock> {
        let block = CounterBlock {
            start: self.next,
            end: self.next + 64,
        };
        self.next = block.end;
        Ok(block)
    }
}

#[test]
fn roundtrips_turn_zero_and_a_ratcheted_turn() {
    let mut out = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    let mut inb = SessionV2::inbound(
        &bob_keys(),
        &alice_keys().public_identity(),
        test_epoch(),
        1,
    )
    .unwrap();

    // Turn 0: the static-static root key.
    let m0 = out.seal(b"turn zero payload").unwrap();
    assert_eq!(inb.open(&m0).unwrap(), b"turn zero payload");

    // Turn 1: a fresh ephemeral chains onto the root.
    out.open_turn(TURN_SEED_1).unwrap();
    assert_eq!(out.turn_no(), 1);
    let m1 = out.seal(b"turn one payload").unwrap();
    assert_eq!(inb.open(&m1).unwrap(), b"turn one payload");

    // Turn 2: chaining is repeatable.
    out.open_turn(TURN_SEED_2).unwrap();
    let m2 = out.seal(b"turn two payload").unwrap();
    assert_eq!(inb.open(&m2).unwrap(), b"turn two payload");

    // The three turns sealed under different keys: same plaintext,
    // same counter position, different envelope bytes.
    let mut out2 = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    let t0 = out2.seal(b"turn zero payload").unwrap();
    out2.open_turn(TURN_SEED_1).unwrap();
    let t1 = out2.seal(b"turn zero payload").unwrap();
    assert_ne!(t0, t1);
}

#[test]
fn tampered_or_foreign_envelopes_fail_closed() {
    let mut out = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    let mut inb = SessionV2::inbound(
        &bob_keys(),
        &alice_keys().public_identity(),
        test_epoch(),
        1,
    )
    .unwrap();
    out.open_turn(TURN_SEED_1).unwrap();
    let envelope = out.seal(b"secret batch").unwrap();

    // Single-bit flips anywhere in the ciphertext or header break
    // authentication closed.
    for flip_at in [5, 21, 37, 90, envelope.len() - 1] {
        let mut damaged = envelope.clone();
        damaged[flip_at] ^= 0x01;
        let err = inb
            .open(&damaged)
            .err()
            .unwrap_or_else(|| panic!("envelope accepted with byte {flip_at} flipped"));
        assert!(
            matches!(
                err,
                altior_crypto::CryptoError::EnvelopeOpen { .. }
                    | altior_crypto::CryptoError::EpochMismatch { .. }
                    | altior_crypto::CryptoError::TurnMismatch { .. }
            ),
            "byte {flip_at}: unexpected {err:?}"
        );
    }

    // A duplicate delivery of the same envelope is a replay.
    let replayed = out.seal(b"second envelope").unwrap();
    assert!(inb.open(&replayed).is_ok());
    assert!(matches!(
        inb.open(&replayed),
        Err(altior_crypto::CryptoError::ReplayRejected { .. })
    ));

    // A foreign epoch sequence is rejected before any cryptography.
    let mut foreign = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        2,
        &mut FixedLease::new(),
    )
    .unwrap();
    let foreign_envelope = foreign.seal(b"other epoch").unwrap();
    assert!(matches!(
        inb.open(&foreign_envelope),
        Err(altior_crypto::CryptoError::EpochMismatch {
            expected_seq: 1,
            found_seq: 2
        })
    ));

    // A skipped turn fails closed (no blind key-chain jumps).
    out.open_turn(TURN_SEED_2).unwrap();
    out.open_turn(TURN_SEED_1).unwrap();
    let ahead = out.seal(b"turn three").unwrap();
    assert!(matches!(
        inb.open(&ahead),
        Err(altior_crypto::CryptoError::TurnMismatch { .. })
    ));
}

#[test]
fn counter_blocks_are_exhausted_then_renewed_never_rewound() {
    // A 64-counter block: 64 seals succeed, the 65th demands a new lease.
    let mut out = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    for _ in 0..64 {
        out.seal(b"x").unwrap();
    }
    assert!(matches!(
        out.seal(b"x"),
        Err(altior_crypto::CryptoError::CounterBlockExhausted)
    ));

    // Installing a block that starts below the next counter is a caller
    // bug that would reuse counters: refused.
    let rewind = CounterBlock { start: 1, end: 65 };
    assert!(matches!(
        out.set_counter_block(rewind),
        Err(altior_crypto::CryptoError::CounterBlockRewind { .. })
    ));

    let next = CounterBlock {
        start: 65,
        end: 129,
    };
    out.set_counter_block(next).unwrap();
    out.seal(b"renewed").unwrap();

    // An empty or inverted block is refused outright.
    assert!(
        out.set_counter_block(CounterBlock {
            start: 200,
            end: 200
        })
        .is_err()
    );
}

#[test]
fn low_order_peer_key_is_rejected_as_degenerate() {
    // An all-zero X25519 public key is the canonical low-order point; the
    // ECDH result would be the all-zero shared secret.
    let degenerate = DeviceIdentity::new(DeviceId::new("mallory").unwrap(), [0; 32], [9; 32]);
    let err = SessionV2::outbound(
        &alice_keys(),
        &degenerate,
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .expect_err("degenerate peer key accepted");
    assert!(matches!(
        err,
        altior_crypto::CryptoError::DegenerateSharedSecret
    ));

    let err = SessionV2::inbound(&alice_keys(), &degenerate, test_epoch(), 1)
        .expect_err("degenerate peer key accepted inbound");
    assert!(matches!(
        err,
        altior_crypto::CryptoError::DegenerateSharedSecret
    ));
}

#[test]
fn turn_hygiene_zero_seed_and_inbound_rejected() {
    let mut out = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    assert!(matches!(
        out.open_turn([0; 32]),
        Err(altior_crypto::CryptoError::EphemeralSeedAllZero)
    ));

    let mut inb = SessionV2::inbound(
        &bob_keys(),
        &alice_keys().public_identity(),
        test_epoch(),
        1,
    )
    .unwrap();
    assert!(matches!(
        inb.open_turn(TURN_SEED_1),
        Err(altior_crypto::CryptoError::TurnNotOpen)
    ));
}

#[test]
fn epoch_must_be_initialized() {
    assert!(matches!(
        SessionEpoch::from_bytes([0; 16]),
        Err(altior_crypto::CryptoError::EpochAllZero)
    ));
}

#[test]
fn sessions_resume_from_chain_snapshots_across_a_restart() {
    let mut out = SessionV2::outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &mut FixedLease::new(),
    )
    .unwrap();
    let mut inb = SessionV2::inbound(
        &bob_keys(),
        &alice_keys().public_identity(),
        test_epoch(),
        1,
    )
    .unwrap();

    out.open_turn(TURN_SEED_1).unwrap();
    let m1 = out.seal(b"before the restart").unwrap();
    assert_eq!(inb.open(&m1).unwrap(), b"before the restart");

    // Snapshots taken at turn 1, then the process dies.
    let out_snapshot = out.outbound_chain().expect("outbound chain");
    let in_snapshot = inb.inbound_chain().expect("inbound chain");
    assert_eq!(out_snapshot.turn_no, 1);
    assert_eq!(in_snapshot.turn_no, 1);

    // Restart: both directions restore from the snapshots (production:
    // OS secret store). Counters continue from the durable lease.
    let mut out2 = SessionV2::restore_outbound(
        &alice_keys(),
        &bob_identity(),
        test_epoch(),
        1,
        &out_snapshot,
        &mut FixedLease::new(),
    )
    .unwrap();
    let mut in2 = SessionV2::restore_inbound(
        &bob_keys(),
        &alice_keys().public_identity(),
        test_epoch(),
        1,
        &in_snapshot,
    )
    .unwrap();

    // A restored sending session whose turn was interrupted refuses to
    // seal before opening a fresh turn: the old ephemeral is gone, and
    // guessing would produce envelopes the peer cannot place.
    assert!(matches!(
        out2.seal(b"premature"),
        Err(altior_crypto::CryptoError::TurnNotOpen)
    ));

    // Fresh turn continues the chain; the restored peer follows.
    out2.open_turn(TURN_SEED_2).unwrap();
    assert_eq!(out2.turn_no(), 2);
    let m2 = out2.seal(b"after the restart").unwrap();
    assert_eq!(in2.open(&m2).unwrap(), b"after the restart");

    // Envelopes of the already-passed turn stay rejected after restore.
    assert!(matches!(
        in2.open(&m1),
        Err(altior_crypto::CryptoError::TurnMismatch { .. })
    ));
}
