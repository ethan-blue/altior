//! ADR 0028 blob plane sync: a multi-chunk blob crosses between two
//! devices as a signed manifest plus chunk envelopes, both sides can
//! reassemble identical plaintext, dedup collapses identical chunks, and
//! the whole plane fails closed on tampering.

use altior_core::sync::{SyncNode, vault_relay};
use altior_crypto::{
    BLOB_CHUNK_PLAINTEXT_MAX, CounterBlock, CounterLease, DeviceId, DeviceKeys, SessionEpoch,
};

const VAULT_KEY: [u8; 32] = [88; 32];

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
    key_seed[31] = seed.wrapping_add(23);
    DeviceKeys::from_seed(key_seed, DeviceId::new(label).unwrap()).unwrap()
}

/// Deterministic multi-chunk plaintext (about 2.5 chunks).
fn fixture_bytes() -> Vec<u8> {
    (0..BLOB_CHUNK_PLAINTEXT_MAX * 2 + 33_333)
        .map(|i| u8::try_from((i * 7) % 256).unwrap_or(0))
        // cast is safe: the value is < 256 by construction
        .collect()
}

#[test]
fn blob_plane_syncs_and_verifies_between_devices() {
    let mut a = SyncNode::new(keys(1, "blob-a"), "blob-a");
    let mut b = SyncNode::new(keys(2, "blob-b"), "blob-b");
    let epoch = SessionEpoch::from_bytes([44; 16]).unwrap();
    let mut relay = vault_relay();
    {
        let (mut ab, mut ba) = (MemLease::new(), MemLease::new());
        SyncNode::pair(&mut a, &mut b, &epoch, 1, &mut ab, &mut ba).unwrap();
    }

    // A stores a multi-chunk blob; its manifest event and chunk envelopes
    // fan out on flush.
    let plaintext = fixture_bytes();
    let blob_id = a.put_blob(&VAULT_KEY, &plaintext).unwrap();
    a.flush(&mut relay, [81; 32]).unwrap();
    b.poll(&mut relay).unwrap();
    b.flush(&mut relay, [82; 32]).unwrap();

    // B reassembles byte-identical plaintext.
    let restored = b.get_blob(&VAULT_KEY, &blob_id).unwrap();
    assert_eq!(restored, plaintext);

    // Convergence round the other way: B's flush carried the manifest
    // and chunks back to A, whose digest is unaffected but whose blob
    // state is complete.
    let restored_a = a.get_blob(&VAULT_KEY, &blob_id).unwrap();
    assert_eq!(restored_a, plaintext);

    // Dedup: re-storing identical content adds no new chunk identities
    // and still returns the same blob id (convergent encryption).
    let blob_id_again = a.put_blob(&VAULT_KEY, &plaintext).unwrap();
    assert_eq!(blob_id_again, blob_id);

    // Tampering fails closed at the crypto layer (see
    // altior-crypto/tests/blob_chunks.rs); the engine adds the transport
    // digest check in ingest_chunk, unit-covered there.
}
