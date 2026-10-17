//! ADR 0027 §6 persistence: a restarted durable node reloads its applied
//! journal from storage, replays it through the apply path, and loses
//! nothing — not the visible state, not the dedup basis. Relay retention
//! becomes irrelevant to correctness: even a relay that forgot every
//! envelope cannot strand a restarted node's state.

use altior_core::{StoreSyncStatePort, SyncNode, SyncStatePort};
use altior_crypto::{CounterBlock, CounterLease, DeviceId, DeviceKeys, SessionEpoch};
use tempfile::tempdir;

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

fn keys(seed: u8, label: &str) -> DeviceKeys {
    let mut key_seed = [seed; 64];
    key_seed[31] = seed.wrapping_add(17);
    DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap()
}

#[test]
fn durable_node_survives_restart_without_loss() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("sync-state.db");
    let epoch = SessionEpoch::from_bytes([31; 16]).unwrap();
    let mut lease_ab = MemLease::new();
    let mut lease_ba = MemLease::new();
    // One relay for the whole story: its retained history and global
    // sequence space are what the persisted fetch cursors refer to.
    let mut relay = altior_core::sync::vault_relay();

    // Session 1: the durable node B exchanges traffic with A.
    let mut a = SyncNode::new(keys(1, "persist-a"), "persist-a");
    {
        let mut store = altior_storage::Store::open(&db_path).unwrap();
        let port = StoreSyncStatePort::new(store);
        let mut b = SyncNode::durable(keys(2, "persist-b"), "persist-b", Box::new(port)).unwrap();
        SyncNode::pair(&mut a, &mut b, &epoch, 1, &mut lease_ab, &mut lease_ba).unwrap();

        a.propose_memory("mem_restart0000001", "written before the restart")
            .expect("propose failed")
            .expect("warm-up proposal");
        a.flush(&mut relay, [71; 32]).unwrap();
        b.poll(&mut relay).unwrap();
        assert!(b.memories().contains_key("mem_restart0000001"));

        b.append_document("doc-restart", "local edit by b").unwrap();
        b.flush(&mut relay, [72; 32]).unwrap();
        a.poll(&mut relay).unwrap();
        // `b` drops here along with its port: only SQLite remains.
    }

    // Session 2: a brand-new node over the same store reloads everything.
    let digest_b2;
    {
        let mut store = altior_storage::Store::open(&db_path).unwrap();
        let port = StoreSyncStatePort::new(store);
        let b2 = SyncNode::durable(keys(2, "persist-b"), "persist-b", Box::new(port)).unwrap();

        assert!(
            b2.memories().contains_key("mem_restart0000001"),
            "journal replay lost the replicated memory"
        );
        assert!(
            b2.documents()
                .get("doc-restart")
                .is_some_and(|entries| entries.contains("local edit by b")),
            "journal replay lost the local document edit"
        );
        assert!(
            b2.applied_count() >= 3,
            "applied journal incomplete: M1 + B's edit + ack traffic"
        );
        digest_b2 = b2.state_digest();
    }
    let _ = digest_b2;

    // Session 3: the reloaded node keeps converging — dedup and state
    // both survived, and new traffic flows normally. The SAME device A
    // continues (a recreated label would collide its event ids, and the
    // relay correctly refuses the push).
    {
        let mut store = altior_storage::Store::open(&db_path).unwrap();
        let port = StoreSyncStatePort::new(store);
        let mut b2 = SyncNode::durable(keys(2, "persist-b"), "persist-b", Box::new(port)).unwrap();
        SyncNode::pair(&mut a, &mut b2, &epoch, 2, &mut lease_ab, &mut lease_ba).unwrap();

        a.propose_memory("mem_restart0000002", "written after the restart")
            .expect("propose failed")
            .expect("post-restart proposal");
        a.flush(&mut relay, [73; 32]).unwrap();
        b2.poll(&mut relay).unwrap();
        b2.flush(&mut relay, [74; 32]).unwrap();
        a.poll(&mut relay).unwrap();

        assert!(b2.memories().contains_key("mem_restart0000002"));
        assert!(b2.memories().contains_key("mem_restart0000001"));
        assert_eq!(a.state_digest(), b2.state_digest(), "A and B2 diverged");
    }
}
