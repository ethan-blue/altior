//! Pairing transcript tests (ADR 0011): canonical bytes, signature
//! round trip, substitution rejection. Fixed seeds only.

use altior_crypto::{CryptoError, DeviceId, DeviceIdentity, DeviceKeys, PairingTranscript};

fn alice() -> DeviceKeys {
    keys(0xa1, "alice-laptop")
}

fn bob() -> DeviceKeys {
    keys(0x5b, "bob-phone")
}

fn keys(seed: u8, id: &str) -> DeviceKeys {
    DeviceKeys::from_seed([seed; 64], DeviceId::new(id).expect("valid device id"))
        .expect("valid seed")
}

#[test]
fn transcript_is_order_independent() {
    let (alice, bob) = (alice(), bob());
    let (a, b) = (alice.public_identity(), bob.public_identity());
    assert_eq!(
        PairingTranscript::new(&a, &b)
            .expect("transcript")
            .as_bytes(),
        PairingTranscript::new(&b, &a)
            .expect("transcript")
            .as_bytes(),
        "both devices must canonicalize the same transcript"
    );
}

#[test]
fn signatures_round_trip_between_devices() {
    let (alice, bob) = (alice(), bob());
    let (a, b) = (alice.public_identity(), bob.public_identity());
    let transcript = PairingTranscript::new(&a, &b).expect("transcript");

    let alice_sig = transcript.sign(&alice);
    let bob_sig = transcript.sign(&bob);
    transcript.verify(&a, &alice_sig).expect("alice verifies");
    transcript.verify(&b, &bob_sig).expect("bob verifies");
}

#[test]
fn substitution_is_rejected() {
    let (alice, bob) = (alice(), bob());
    let (a, b) = (alice.public_identity(), bob.public_identity());
    let transcript = PairingTranscript::new(&a, &b).expect("transcript");
    let alice_sig = transcript.sign(&alice);

    // Mallory presents her own identity but alice's signature.
    let mallory = DeviceIdentity::new(
        DeviceId::new("mallory-phone").expect("valid id"),
        a.x25519_public(),
        b.ed25519_public(),
    );
    assert!(matches!(
        transcript.verify(&mallory, &alice_sig),
        Err(CryptoError::SignatureInvalid)
    ));

    // A signature over a different transcript does not verify here.
    let mallory_real = keys(0x3d, "mallory-real");
    let other = PairingTranscript::new(&a, &mallory_real.public_identity()).expect("transcript");
    let other_sig = other.sign(&mallory_real);
    assert!(matches!(
        transcript.verify(&mallory_real.public_identity(), &other_sig),
        Err(CryptoError::SignatureInvalid)
    ));

    // Garbage bytes are not signatures.
    assert!(matches!(
        transcript.verify(&a, &[0u8; 64]),
        Err(CryptoError::SignatureInvalid)
    ));
}

#[test]
fn tampered_transcript_fails_verification() {
    let (alice, bob) = (alice(), bob());
    let (a, b) = (alice.public_identity(), bob.public_identity());
    let transcript = PairingTranscript::new(&a, &b).expect("transcript");
    let alice_sig = transcript.sign(&alice);

    // An id renamed after signing: the device rebuilds the transcript
    // from what it now sees, and alice's signature — made over the
    // original id — no longer covers those bytes.
    let renamed = DeviceIdentity::new(
        DeviceId::new("alice-laptop-2").expect("valid id"),
        a.x25519_public(),
        a.ed25519_public(),
    );
    let tampered = PairingTranscript::new(&renamed, &b).expect("transcript");
    assert!(matches!(
        tampered.verify(&renamed, &alice_sig),
        Err(CryptoError::SignatureInvalid)
    ));
}

#[test]
fn same_device_id_is_rejected_instead_of_ambiguously_ordered() {
    let first = keys(0x11, "same-device").public_identity();
    let second = keys(0x22, "same-device").public_identity();
    assert!(matches!(
        PairingTranscript::new(&first, &second),
        Err(CryptoError::DeviceIdCollision { .. })
    ));
}
