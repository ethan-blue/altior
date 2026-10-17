//! ADR 0025 suite 4 — `test_stale_device_tombstone_non_resurrection`, on
//! the ADR 0027 sync engine.
//!
//! Memory 1 exists on three devices. Device C goes dormant; Device A
//! forgets Memory 1 and the tombstone replicates to B. C comes back
//! still carrying its old state and re-proposes Memory 1 — the stale
//! write must not resurrect it anywhere: pull-before-push applies the
//! tombstone on C before C's write leaves, and the peers suppress the
//! stale proposal on arrival. A fresh memory C writes after rejoining
//! must survive, proving the gate blocks forgotten data, not the device.

use altior_core::sync::{SyncNode, vault_relay};
use altior_crypto::DeviceId;
use altior_crypto::{CounterBlock, CounterLease, DeviceKeys, SessionEpoch};

const EPOCH_BYTES: [u8; 16] = [13; 16];
const TURN_SEED_A: [u8; 32] = [51; 32];
const TURN_SEED_C: [u8; 32] = [53; 32];

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

fn node(seed: u8, label: &str) -> SyncNode {
    let mut key_seed = [seed; 64];
    key_seed[31] = seed.wrapping_add(3);
    let keys = DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap();
    SyncNode::new(keys, label)
}

#[test]
fn test_stale_device_tombstone_non_resurrection() {
    let mut a = node(1, "device-a");
    let mut b = node(2, "device-b");
    let mut c = node(3, "device-c");
    let epoch = SessionEpoch::from_bytes(EPOCH_BYTES).unwrap();
    let mut relay = vault_relay();

    // Pair the triangle.
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

    // Warm-up: Memory 1 ("dormant-state note") is everywhere before C
    // goes dormant.
    a.propose_memory("mem_000000000001d", "dormant-state note")
        .expect("propose failed")
        .expect("warm-up");
    a.flush(&mut relay, TURN_SEED_A).unwrap();
    b.poll(&mut relay).unwrap();
    c.poll(&mut relay).unwrap();
    for node in [&a, &b, &c] {
        assert!(node.memories().contains_key("mem_000000000001d"));
    }

    // ── C goes dormant; A forgets Memory 1; B learns the tombstone ──
    a.forget_memory("mem_000000000001d", 1_700_100_000_000, [9; 32]);
    a.flush(&mut relay, TURN_SEED_A).unwrap();
    b.poll(&mut relay).unwrap();
    assert!(b.tombstones().contains_key("mem_000000000001d"));

    // ── C, still holding its old state, re-proposes Memory 1 ─────────
    // C has not seen the tombstone, so the proposal is accepted locally
    // (this is the stale-database state ADR 0025 describes).
    let stale = c
        .propose_memory(
            "mem_000000000001d",
            "STALE: resurrecting from my 30-day-old copy",
        )
        .expect("propose failed")
        .expect("C should not know yet; the stale write is queued");

    // C also queues a fresh, legitimate memory before reconnecting.
    c.propose_memory("mem_000000000004c", "fresh memory from the returned device")
        .expect("propose failed")
        .expect("M4");

    // ── C reconnects: pull-before-push ──────────────────────────────
    // flush polls first: C applies the forget, removing its own stale
    // copy, before any of its writes are admitted to the relay.
    c.flush(&mut relay, TURN_SEED_C).unwrap();
    assert!(
        !c.memories().contains_key("mem_000000000001d"),
        "pull-before-push failed: C still holds the forgotten memory after reconnecting"
    );
    assert!(c.tombstones().contains_key("mem_000000000001d"));

    // Distribute C's flush and settle.
    a.poll(&mut relay).unwrap();
    b.poll(&mut relay).unwrap();
    a.flush(&mut relay, TURN_SEED_A).unwrap();
    c.poll(&mut relay).unwrap();

    // ── Assertions: forgotten stays forgotten, fresh memory survives ─
    for (label, node) in [("a", &a), ("b", &b), ("c", &c)] {
        assert!(
            !node.memories().contains_key("mem_000000000001d"),
            "{label}: stale device resurrected the forgotten memory"
        );
        assert!(
            node.tombstones().contains_key("mem_000000000001d"),
            "{label}: tombstone missing"
        );
        assert!(
            node.memories().contains_key("mem_000000000004c"),
            "{label}: the gate blocked a legitimate fresh memory"
        );
    }
    // The stale proposal was suppressed at both surviving peers.
    assert_eq!(
        a.suppressed_resurrections(),
        1,
        "A must suppress C's stale proposal"
    );
    assert_eq!(
        b.suppressed_resurrections(),
        1,
        "B must suppress C's stale proposal"
    );
    assert_eq!(
        c.suppressed_resurrections(),
        0,
        "C suppressed nothing itself; the gate was pull-before-push"
    );

    // Full convergence.
    let (da, db, dc) = (a.state_digest(), b.state_digest(), c.state_digest());
    assert_eq!(da, db, "A and B diverged");
    assert_eq!(db, dc, "B and C diverged");
}
