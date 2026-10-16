//! ADR 0025 suite 2 — `test_session_reconstruction_nonce_uniqueness`.
//!
//! Recreates 1 000 sequential sessions between the same device pair with
//! simulated process kills (a fresh `SessionV2` each time, keys static,
//! epoch unchanged) and asserts that across all generated ciphertexts no
//! counter — and therefore no nonce, which is counter-derived — repeats
//! under the same key. Before the ADR 0027 counter-lease fix this was
//! the F31 nonce-reuse catastrophe: every reconstructed session restarted
//! its counter at 0/1.

use std::collections::HashSet;

use altior_crypto::{
    CounterBlock, CounterLease, DeviceId, DeviceIdentity, DeviceKeys, SessionEpoch, SessionV2,
};

const ALICE_SEED: [u8; 64] = [1; 64];
const BOB_SEED: [u8; 64] = [2; 64];
const EPOCH_BYTES: [u8; 16] = [7; 16];
const SESSIONS: u64 = 1_000;
const ENVELOPES_PER_SESSION: u64 = 100;

/// A stand-in for the durable `SecretStore`-backed lease: process kills
/// destroy the session, never the reservation ledger.
#[derive(Default)]
struct DurableLease {
    next: u64,
}

impl DurableLease {
    fn new() -> Self {
        Self { next: 1 }
    }
}

impl CounterLease for DurableLease {
    fn reserve(&mut self, block_size: u64) -> std::io::Result<CounterBlock> {
        let block = CounterBlock {
            start: self.next,
            end: self.next + block_size,
        };
        self.next = block.end;
        Ok(block)
    }
}

#[test]
fn test_session_reconstruction_nonce_uniqueness() {
    let alice =
        DeviceKeys::from_seed(ALICE_SEED, DeviceId::new("alice-uniqueness").unwrap()).unwrap();
    let bob = bob_identity();
    let epoch = SessionEpoch::from_bytes(EPOCH_BYTES).unwrap();
    let mut lease = DurableLease::new();

    // Every (sender, receiver, epoch, counter) ever used — the identity
    // of a nonce under a key. Session reconstruction with static keys and
    // the same epoch reuses the key, so uniqueness here is exactly the
    // F31 property.
    let mut used_counters: HashSet<u64> =
        HashSet::with_capacity(usize::try_from(SESSIONS * ENVELOPES_PER_SESSION).unwrap());
    // And the raw 12-byte nonces off the wire, independent of any
    // reasoning about which key they rode under.
    let mut used_nonces: HashSet<[u8; 12]> = HashSet::new();

    for session_no in 0..SESSIONS {
        // Simulated kill: the previous session is dropped (its in-memory
        // counter state is lost); only the durable lease survives.
        drop(
            SessionV2::outbound(&alice, &bob, epoch.clone(), 1, &mut lease)
                .expect("session reconstruction"),
        );

        let mut session = SessionV2::outbound(&alice, &bob, epoch.clone(), 1, &mut lease)
            .expect("session reconstruction");
        assert_eq!(session.turn_no(), 0);

        for n in 0..ENVELOPES_PER_SESSION {
            let plaintext = format!("kill {session_no} envelope {n}").into_bytes();
            let envelope = session
                .seal(&plaintext)
                .expect("seal within a leased block");
            assert_eq!(envelope[0], 2, "v2 version byte");

            let counter = u64::from_be_bytes(envelope[29..37].try_into().unwrap());
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(&envelope[69..81]);
            assert!(used_counters.insert(counter), "counter {counter} reused");
            assert!(used_nonces.insert(nonce), "nonce repeated on the wire");
        }
    }

    assert_eq!(
        usize::try_from(SESSIONS * ENVELOPES_PER_SESSION).unwrap(),
        used_counters.len(),
        "every envelope carried a fresh counter"
    );
    assert_eq!(
        usize::try_from(SESSIONS * ENVELOPES_PER_SESSION).unwrap(),
        used_nonces.len()
    );
}

fn bob_identity() -> DeviceIdentity {
    DeviceKeys::from_seed(BOB_SEED, DeviceId::new("bob-uniqueness").unwrap())
        .unwrap()
        .public_identity()
}
