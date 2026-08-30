//! Two-device envelope tests (ADR 0011): round trip, tampering,
//! context binding, replay window, determinism. Fixed seeds only —
//! no RNG, no sleeps, no network.

use altior_crypto::{CryptoError, DeviceId, DeviceKeys, Session};

const ENVELOPE_BODY_OFFSET: usize = 1 + 8 + 12;
const POLY1305_TAG_LEN: usize = 16;

fn alice() -> DeviceKeys {
    keys(0xa1, "alice-laptop")
}

fn bob() -> DeviceKeys {
    keys(0x5b, "bob-phone")
}

fn carol() -> DeviceKeys {
    keys(0xc3, "carol-tablet")
}

fn keys(seed: u8, id: &str) -> DeviceKeys {
    DeviceKeys::from_seed([seed; 64], DeviceId::new(id).expect("valid device id"))
        .expect("valid seed")
}

#[test]
fn round_trip_between_devices() {
    let (alice, bob) = (alice(), bob());
    let bob_identity = bob.public_identity();
    let mut outbound = Session::outbound(&alice, &bob_identity).expect("distinct devices");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("distinct devices");

    for (i, plaintext) in ["hello", "", "multi\nline \u{4e2d}\u{6587} body"]
        .iter()
        .enumerate()
    {
        let envelope = outbound.seal(plaintext.as_bytes()).expect("seal");
        assert_eq!(
            u64::from_be_bytes(envelope[1..9].try_into().expect("counter")),
            (i as u64) + 1,
            "counters advance from 1"
        );
        let opened = inbound.open(&envelope).expect("open");
        assert_eq!(opened, plaintext.as_bytes(), "plaintext survives");
    }
}

#[test]
fn both_directions_work_between_the_same_devices() {
    let (alice, bob) = (alice(), bob());
    let mut alice_out = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut bob_in = Session::inbound(&bob, &alice.public_identity()).expect("session");
    let mut bob_out = Session::outbound(&bob, &alice.public_identity()).expect("session");
    let mut alice_in = Session::inbound(&alice, &bob.public_identity()).expect("session");

    let to_bob = alice_out.seal(b"to bob").expect("seal");
    let to_alice = bob_out.seal(b"to alice").expect("seal");
    // Cross-direction: a session for the wrong direction must reject
    // (sender/receiver in the AAD differ).
    assert!(matches!(
        alice_in.open(&to_bob),
        Err(CryptoError::EnvelopeOpen { .. })
    ));
    assert_eq!(bob_in.open(&to_bob).expect("open"), b"to bob");
    assert_eq!(alice_in.open(&to_alice).expect("open"), b"to alice");
}

#[test]
fn wire_bytes_are_deterministic_for_the_same_seeds() {
    let mut first = Session::outbound(&alice(), &bob().public_identity()).expect("session");
    let mut second = Session::outbound(&alice(), &bob().public_identity()).expect("session");
    // Both sessions are on counter 1, so the counter-nonces match and
    // identical plaintexts must produce identical bytes.
    let a = first.seal(b"deterministic").expect("seal");
    let b = second.seal(b"deterministic").expect("seal");
    assert_eq!(a, b, "same seeds and counters give same wire bytes");

    let different = first.seal(b"deterministic").expect("seal");
    assert_ne!(a, different, "counters advance, so bytes differ");
}

#[test]
fn tampered_envelopes_fail_authentication() {
    let (alice, bob) = (alice(), bob());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("session");
    let envelope = outbound.seal(b"tamper target").expect("seal");

    // Byte 0 is the version byte and fails the version check, not
    // authentication — it has its own case below.
    for index in [5usize, 12, envelope.len() - 1] {
        let mut tampered = envelope.clone();
        tampered[index] ^= 0x01;
        assert!(
            matches!(
                inbound.open(&tampered),
                Err(CryptoError::EnvelopeOpen { .. })
            ),
            "flipping byte {index} must break authentication"
        );
    }
    let mut truncated = envelope.clone();
    truncated.truncate(36);
    assert!(matches!(
        inbound.open(&truncated),
        Err(CryptoError::EnvelopeTruncated { size: 36 })
    ));
    let mut wrong_version = envelope.clone();
    wrong_version[0] = 9;
    assert!(matches!(
        inbound.open(&wrong_version),
        Err(CryptoError::EnvelopeVersion { found: 9 })
    ));
}

#[test]
fn wrong_peer_or_wrong_ids_reject() {
    let (alice, bob, carol) = (alice(), bob(), carol());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");

    // Keys from a different pairing: carol never shared a session
    // with alice, so her inbound cannot open alice's envelope.
    let mut carol_inbound = Session::inbound(&carol, &alice.public_identity()).expect("session");
    let envelope = outbound.seal(b"secret").expect("seal");
    assert!(matches!(
        carol_inbound.open(&envelope),
        Err(CryptoError::EnvelopeOpen { .. })
    ));

    // Same key material, but the receiver believes the sender has a
    // different id: ids are bound into the HKDF salt and the AAD, so
    // the envelope must not open.
    let bob_as_mallory = keys(0x5b, "mallory-phone");
    let alice_as_seen = {
        let real = alice.public_identity();
        altior_crypto::DeviceIdentity::new(
            DeviceId::new("someone-else").expect("valid device id"),
            real.x25519_public(),
            real.ed25519_public(),
        )
    };
    let mut bob_inbound = Session::inbound(&bob_as_mallory, &alice_as_seen).expect("session");
    assert!(
        matches!(
            bob_inbound.open(&envelope),
            Err(CryptoError::EnvelopeOpen { .. })
        ),
        "renaming the sender must break salt/AAD binding"
    );
}

#[test]
fn replayed_envelopes_are_rejected() {
    let (alice, bob) = (alice(), bob());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("session");

    let envelope = outbound.seal(b"once").expect("seal");
    assert_eq!(inbound.open(&envelope).expect("open"), b"once");
    assert!(
        matches!(
            inbound.open(&envelope),
            Err(CryptoError::ReplayRejected { counter: 1 })
        ),
        "the exact same envelope must be refused a second time"
    );
}

#[test]
fn reordering_within_the_window_is_accepted() {
    let (alice, bob) = (alice(), bob());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("session");

    let envelopes: Vec<Vec<u8>> = (0..3)
        .map(|i| outbound.seal(format!("msg{i}").as_bytes()).expect("seal"))
        .collect();
    // Deliver 3, 1, 2 — a relay may reorder within its retention.
    for i in [2usize, 0, 1] {
        let opened = inbound.open(&envelopes[i]).expect("open");
        assert_eq!(opened, format!("msg{i}").as_bytes());
    }
    assert!(matches!(
        inbound.open(&envelopes[0]),
        Err(CryptoError::ReplayRejected { .. })
    ));
}

#[test]
fn stale_envelopes_beyond_the_window_are_rejected() {
    let (alice, bob) = (alice(), bob());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("session");

    // 80 envelopes; deliver 1..=70 so the window's highest is 70.
    let envelopes: Vec<Vec<u8>> = (0..80)
        .map(|i| {
            outbound
                .seal(&[u8::try_from(i).expect("i below 80")])
                .expect("seal")
        })
        .collect();
    for envelope in &envelopes[..70] {
        inbound.open(envelope).expect("open within window span");
    }
    // Counter 1 is 69 behind the highest: outside the 64-delivery
    // window, refused even though never delivered.
    assert!(matches!(
        inbound.open(&envelopes[0]),
        Err(CryptoError::ReplayRejected { counter: 1 })
    ));
    // Fresh mail still flows after the stale rejection.
    assert_eq!(
        inbound.open(&envelopes[79]).expect("open"),
        [79u8],
        "fresh envelopes still accepted"
    );
}

#[test]
fn large_plaintext_round_trips() {
    let (alice, bob) = (alice(), bob());
    let mut outbound = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut inbound = Session::inbound(&bob, &alice.public_identity()).expect("session");
    // A realistic document-state payload for the relay era. Chosen by
    // hand, not measured: the point is that size is not special.
    let plaintext: Vec<u8> = (0..64 * 1024u32)
        .map(|i| u8::try_from(i % 251).expect("below 251"))
        .collect();
    let envelope = outbound.seal(&plaintext).expect("seal");
    assert_eq!(inbound.open(&envelope).expect("open"), plaintext);
}

#[test]
fn zero_seed_is_refused() {
    assert!(matches!(
        DeviceKeys::from_seed([0; 64], DeviceId::new("void").expect("valid id")),
        Err(CryptoError::SeedAllZero)
    ));
}

#[test]
fn opposite_directions_at_counter_one_do_not_reuse_the_key_stream() {
    let (alice, bob) = (alice(), bob());
    let mut alice_out = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut bob_out = Session::outbound(&bob, &alice.public_identity()).expect("session");
    let a_plain = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let b_plain = b"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    let a = alice_out.seal(a_plain).expect("seal A to B");
    let b = bob_out.seal(b_plain).expect("seal B to A");

    let a_ciphertext = &a[ENVELOPE_BODY_OFFSET..a.len() - POLY1305_TAG_LEN];
    let b_ciphertext = &b[ENVELOPE_BODY_OFFSET..b.len() - POLY1305_TAG_LEN];
    let ciphertext_xor: Vec<u8> = a_ciphertext
        .iter()
        .zip(b_ciphertext)
        .map(|(left, right)| left ^ right)
        .collect();
    let plaintext_xor: Vec<u8> = a_plain
        .iter()
        .zip(b_plain)
        .map(|(left, right)| left ^ right)
        .collect();

    assert_ne!(
        ciphertext_xor, plaintext_xor,
        "direction-separated HKDF keys prevent two-time-pad XOR leakage"
    );
}

#[test]
fn device_identity_inputs_are_typed_and_bounded() {
    assert!(matches!(
        DeviceId::new(""),
        Err(CryptoError::DeviceIdInvalid { length: 0, .. })
    ));
    assert!(matches!(
        DeviceId::new("x".repeat(DeviceId::MAX_LEN + 1)),
        Err(CryptoError::DeviceIdInvalid { .. })
    ));

    let same_a = keys(0x11, "same-device");
    let same_b = keys(0x22, "same-device");
    assert!(matches!(
        Session::outbound(&same_a, &same_b.public_identity()),
        Err(CryptoError::DeviceIdCollision { .. })
    ));
}
