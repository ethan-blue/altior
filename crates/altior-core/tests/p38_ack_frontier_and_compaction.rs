//! ADR 0025 threat 10, closing loop: ack frontiers flow back as signed
//! journal events, the per-origin minimum across peers becomes the
//! tombstone compaction gate, and the gate refuses to release a
//! tombstone until every peer has applied the forget.

use altior_core::sync::{SyncNode, vault_relay};
use altior_crypto::{CounterBlock, CounterLease, DeviceId, DeviceKeys, SessionEpoch};

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
    key_seed[31] = seed.wrapping_add(11);
    let keys = DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap();
    SyncNode::new(keys, label)
}

/// One full settle round: every node flushes (pull-before-push) then
/// polls, twice, so ack events and their consequences propagate.
fn settle(nodes: &mut [&mut SyncNode], relay: &mut altior_relay::Relay, seeds: &[[u8; 32]]) {
    for round in 0..2 {
        for (index, node) in nodes.iter_mut().enumerate() {
            node.flush(relay, seeds[index]).unwrap();
        }
        for node in nodes.iter_mut() {
            node.poll(relay).unwrap();
        }
        let _ = round;
    }
}

#[test]
fn ack_frontiers_gate_tombstone_compaction() {
    let mut a = node(1, "gate-a");
    let mut b = node(2, "gate-b");
    let mut c = node(3, "gate-c");
    let epoch = SessionEpoch::from_bytes([21; 16]).unwrap();
    let mut relay = vault_relay();
    let seeds = [[61; 32], [62; 32], [63; 32]];

    {
        let (mut ab, mut ba) = (MemLease::new(), MemLease::new());
        SyncNode::pair(&mut a, &mut b, &epoch, &mut ab, &mut ba).unwrap();
    }
    {
        let (mut ac, mut ca) = (MemLease::new(), MemLease::new());
        SyncNode::pair(&mut a, &mut c, &epoch, &mut ac, &mut ca).unwrap();
    }
    {
        let (mut bc, mut cb) = (MemLease::new(), MemLease::new());
        SyncNode::pair(&mut b, &mut c, &epoch, &mut bc, &mut cb).unwrap();
    }

    // Memory 1 everywhere, then A forgets it.
    a.propose_memory("mem_gate0000000001", "to be forgotten")
        .unwrap_or_else(|| panic!("warm-up"));
    settle(&mut [&mut a, &mut b, &mut c], &mut relay, &seeds);
    let forget_id = a.forget_memory("mem_gate0000000001", 1_700_200_000_000, [15; 32]);
    let forget_seq: u64 = forget_id
        .rsplit(':')
        .next()
        .and_then(|seq| seq.parse().ok())
        .unwrap_or_else(|| panic!("event id carries the origin sequence"));

    // Before distribution the gate is closed: no peer has acked the
    // forget, so A's gc frontier for its own origin is below the forget.
    let pre = a.gc_frontiers().get("gate-a").copied().unwrap_or(0);
    assert!(
        pre < forget_seq,
        "gate opened before any peer acked the forget"
    );

    // Distribute the forget and let the acks flow back through two
    // settle rounds.
    settle(&mut [&mut a, &mut b, &mut c], &mut relay, &seeds);
    settle(&mut [&mut a, &mut b, &mut c], &mut relay, &seeds);

    // Every node now sees a per-origin gate that covers the forget:
    // every peer has durably applied it.
    for (label, node) in [("gate-a", &a), ("gate-b", &b), ("gate-c", &c)] {
        let gate = node.gc_frontiers().get("gate-a").copied().unwrap_or(0);
        assert!(
            gate >= forget_seq,
            "{label}: compaction gate at {gate} still below the forget at {forget_seq}"
        );
        assert!(
            !node.memories().contains_key("mem_gate0000000001"),
            "{label}: forgotten memory present"
        );
    }

    // The gate is honest about membership: a node with a peer that has
    // acked nothing (never polled) pins the gate at 0.
    let mut d = node(4, "gate-d");
    {
        let (mut ad, mut da) = (MemLease::new(), MemLease::new());
        SyncNode::pair(&mut a, &mut d, &epoch, &mut ad, &mut da).unwrap();
    }
    assert_eq!(
        a.gc_frontiers().get("gate-a").copied(),
        Some(0),
        "a fresh unacked peer must pin the compaction gate at zero"
    );
}
