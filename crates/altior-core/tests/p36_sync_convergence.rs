//! ADR 0025 suite 1 — `test_three_device_offline_convergence`, on the
//! ADR 0027 sync engine.
//!
//! Devices A, B, and C pair. C goes offline. A edits Document 1 and
//! forgets Memory 1 while B concurrently edits Document 1 and adds
//! Memory 2. A and B exchange; C rejoins, pulls, contributes a fresh
//! memory of its own, and all three devices must converge to byte-equal
//! state digests with zero loss: both document edits survive, Memory 1
//! is tombstoned everywhere, Memories 2 and 3 are everywhere.

use altior_core::sync::{SyncNode, vault_relay};
use altior_crypto::DeviceId;
use altior_crypto::{CounterBlock, CounterLease, DeviceKeys, SessionEpoch};

const EPOCH_BYTES: [u8; 16] = [12; 16];
const TURN_SEED_A: [u8; 32] = [41; 32];
const TURN_SEED_B: [u8; 32] = [42; 32];
const TURN_SEED_C: [u8; 32] = [43; 32];

/// In-memory durable lease: one ledger per direction (the production
/// implementation is `SqliteCounterLease`).
#[derive(Default)]
struct MemLease {
    next: u64,
}

impl MemLease {
    fn new() -> Self {
        Self { next: 1 }
    }
}

impl CounterLease for MemLease {
    fn reserve(&mut self, block_size: u64) -> std::io::Result<CounterBlock> {
        let block = CounterBlock {
            start: self.next,
            end: self.next + block_size,
        };
        self.next = block.end;
        Ok(block)
    }
}

fn device(seed: u8, label: &str) -> DeviceKeys {
    let mut key_seed = [seed; 64];
    key_seed[31] = seed.wrapping_add(7);
    DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap()
}

fn node(seed: u8, label: &str) -> SyncNode {
    SyncNode::new(device(seed, label), label)
}

#[test]
fn test_three_device_offline_convergence() {
    let mut a = node(1, "device-a");
    let mut b = node(2, "device-b");
    let mut c = node(3, "device-c");
    let epoch = SessionEpoch::from_bytes(EPOCH_BYTES).unwrap();
    let mut relay = vault_relay();

    // Pair the triangle: three edges, two directional leases each.
    {
        let mut ab = MemLease::new();
        let mut ba = MemLease::new();
        SyncNode::pair(&mut a, &mut b, &epoch, 1, &mut ab, &mut ba).unwrap();
    }
    {
        let mut ac = MemLease::new();
        let mut ca = MemLease::new();
        SyncNode::pair(&mut a, &mut c, &epoch, 1, &mut ac, &mut ca).unwrap();
    }
    {
        let mut bc = MemLease::new();
        let mut cb = MemLease::new();
        SyncNode::pair(&mut b, &mut c, &epoch, 1, &mut bc, &mut cb).unwrap();
    }

    // Warm-up round: Memory 1 exists on all three devices before C goes
    // dark. Every flush polls first, so one pass settles the triangle.
    a.propose_memory("mem_000000000001a", "the shared note")
        .expect("propose failed")
        .expect("warm-up proposal");
    a.flush(&mut relay, TURN_SEED_A).unwrap();
    b.poll(&mut relay).unwrap();
    c.poll(&mut relay).unwrap();
    b.flush(&mut relay, TURN_SEED_B).unwrap();
    c.flush(&mut relay, TURN_SEED_C).unwrap();
    a.poll(&mut relay).unwrap();
    for node in [&a, &b, &c] {
        assert!(
            node.memories().contains_key("mem_000000000001a"),
            "warm-up memory missing pre-dormancy"
        );
    }

    // ── C goes offline ──────────────────────────────────────────────
    // A: document edit + forget Memory 1. B: concurrent document edit +
    // Memory 2. The edits are concurrent — neither device has seen the
    // other when it writes.
    a.append_document("doc-1", "alpha edit by a").unwrap();
    a.forget_memory("mem_000000000001a", 1_700_000_050_000, [5; 32])
        .unwrap();
    b.append_document("doc-1", "beta edit by b").unwrap();
    b.propose_memory("mem_000000000002b", "memory two, added during dormancy")
        .expect("propose failed")
        .expect("M2 proposal");

    a.flush(&mut relay, TURN_SEED_A).unwrap();
    b.flush(&mut relay, TURN_SEED_B).unwrap();
    b.poll(&mut relay).unwrap();
    a.poll(&mut relay).unwrap();

    // A and B converge pairwise: both edits survive, M1 is gone.
    assert_eq!(a.state_digest(), b.state_digest(), "A and B diverged");
    assert!(!a.memories().contains_key("mem_000000000001a"));
    assert!(a.memories().contains_key("mem_000000000002b"));

    // ── C rejoins ───────────────────────────────────────────────────
    // Pull-before-push: the flush polls first, so C applies the
    // tombstone and both document edits before anything of its own
    // leaves. C also contributes Memory 3.
    c.propose_memory("mem_000000000003c", "memory three, written after rejoining")
        .expect("propose failed")
        .expect("M3 proposal");
    c.flush(&mut relay, TURN_SEED_C).unwrap();
    a.poll(&mut relay).unwrap();
    b.poll(&mut relay).unwrap();

    // C has now seen A's and B's events; a final settle round distributes
    // any remainder (C's Memory 3 already flowed in step 1).
    a.flush(&mut relay, TURN_SEED_A).unwrap();
    b.flush(&mut relay, TURN_SEED_B).unwrap();
    c.poll(&mut relay).unwrap();
    c.flush(&mut relay, TURN_SEED_C).unwrap();
    a.poll(&mut relay).unwrap();
    b.poll(&mut relay).unwrap();

    // ── Convergence assertions ──────────────────────────────────────
    let (da, db, dc) = (a.state_digest(), b.state_digest(), c.state_digest());
    assert_eq!(da, db, "A and B diverged");
    assert_eq!(db, dc, "B and C diverged");

    for (label, node) in [("a", &a), ("b", &b), ("c", &c)] {
        assert!(
            !node.memories().contains_key("mem_000000000001a"),
            "{label}: forgotten memory resurrected"
        );
        assert!(
            node.tombstones().contains_key("mem_000000000001a"),
            "{label}: tombstone missing"
        );
        assert!(
            node.memories().contains_key("mem_000000000002b"),
            "{label}: Memory 2 lost"
        );
        assert!(
            node.memories().contains_key("mem_000000000003c"),
            "{label}: Memory 3 lost"
        );
    }
    // Zero suppressions on this path: nothing proposed M1 again.
    assert_eq!(a.suppressed_resurrections(), 0);
    assert_eq!(b.suppressed_resurrections(), 0);
    assert_eq!(c.suppressed_resurrections(), 0);
}
