//! The composition the P0.5 spikes exist for (ADR 0010/0011/0012):
//! an encrypted envelope crosses a content-agnostic relay between
//! two devices. The relay carries it without learning anything, and
//! the receiver's replay window absorbs the relay's at-least-once
//! re-delivery.

use altior_crypto::{CryptoError, DeviceId, DeviceKeys, Session};
use altior_relay::{BucketId, FetchPage, PushOutcome, Relay, RelayPolicy};

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
fn encrypted_envelopes_cross_the_relay_blind() {
    let (alice, bob) = (alice(), bob());
    let mut alice_out = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut bob_in = Session::inbound(&bob, &alice.public_identity()).expect("session");
    let mut relay = Relay::new(RelayPolicy::permissive());
    // The bucket is the receiver's; production derives it from the
    // receiver's public identity so the relay needs no account
    // directory either.
    let bob_inbox = BucketId::new(format!("inbox:{}", bob.public_identity().id()));

    let plaintext = b"knowledge document delta";
    let envelope = alice_out.seal(plaintext).expect("seal");
    assert_ne!(
        envelope.as_slice(),
        &plaintext[..],
        "the relay never sees plaintext"
    );
    assert!(
        !envelope
            .windows(plaintext.len())
            .any(|window| window == plaintext),
        "no plaintext substring rides inside the envelope bytes"
    );

    // Retry semantics: an at-least-once transport may re-push.
    assert!(matches!(
        relay.push(&bob_inbox, "env-1", &envelope),
        Ok(PushOutcome::Pushed { .. })
    ));
    assert_eq!(
        relay.push(&bob_inbox, "env-1", &envelope).expect("re-push"),
        PushOutcome::Duplicate { seq: 1 }
    );

    // The receiver pages its inbox and opens what arrives.
    let FetchPage::Entries { items, .. } = relay.fetch(&bob_inbox, 0, 10) else {
        panic!("expected entries");
    };
    assert_eq!(items.len(), 1, "the duplicate push queued nothing");
    let opened = bob_in.open(&items[0].payload).expect("open");
    assert_eq!(opened, plaintext);

    // The relay's repeatable fetch re-delivers; the replay window
    // absorbs it. The two layers compose instead of duplicating work.
    let FetchPage::Entries { items, .. } = relay.fetch(&bob_inbox, 0, 10) else {
        panic!("expected entries");
    };
    assert_eq!(items.len(), 1, "fetch is non-destructive");
    assert!(matches!(
        bob_in.open(&items[0].payload),
        Err(CryptoError::ReplayRejected { counter: 1 })
    ));

    // A third device cannot read the mail even with full relay
    // access: no key, no plaintext.
    let carol = keys(0xc3, "carol-tablet");
    let mut carol_in = Session::inbound(&carol, &alice.public_identity()).expect("session");
    assert!(matches!(
        carol_in.open(&items[0].payload),
        Err(CryptoError::EnvelopeOpen { .. })
    ));
}

#[test]
fn a_fell_behind_receiver_is_told_to_resync_not_lied_to() {
    let (alice, bob) = (alice(), bob());
    let mut alice_out = Session::outbound(&alice, &bob.public_identity()).expect("session");
    let mut bob_in = Session::inbound(&bob, &alice.public_identity()).expect("session");
    let mut relay = Relay::new(RelayPolicy {
        max_age_ticks: 5,
        ..RelayPolicy::permissive()
    });
    let bob_inbox = BucketId::new("inbox:bob");

    for i in 0..3 {
        let envelope = alice_out
            .seal(format!("delta {i}").as_bytes())
            .expect("seal");
        relay
            .push(&bob_inbox, &format!("env-{i}"), &envelope)
            .expect("push");
    }
    // Bob is offline; retention expires the mail.
    for _ in 0..6 {
        relay.tick();
    }
    relay.sweep_expired();

    // Bob reconnects with a stale cursor: the relay answers with the
    // explicit Compacted page, never with a silent gap.
    assert_eq!(
        relay.fetch(&bob_inbox, 0, 10),
        FetchPage::Compacted { compacted_up_to: 3 }
    );
    // The resync path in production is a fresh CRDT snapshot push
    // (ADR 0010); here a new envelope after the boundary still flows.
    let envelope = alice_out.seal(b"resync snapshot").expect("seal");
    relay.push(&bob_inbox, "env-snap", &envelope).expect("push");
    let FetchPage::Entries {
        items, next_cursor, ..
    } = relay.fetch(&bob_inbox, 3, 10)
    else {
        panic!("expected entries");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(next_cursor, 4);
    assert_eq!(
        bob_in.open(&items[0].payload).expect("open"),
        b"resync snapshot"
    );
}
