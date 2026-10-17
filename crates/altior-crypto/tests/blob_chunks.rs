//! ADR 0028 blob plane: convergent chunk encryption contract.

use altior_crypto::{
    BLOB_CHUNK_PLAINTEXT_MAX, BlobManifest, ChunkMap, blob_id_of, decrypt_and_reassemble,
    split_and_encrypt,
};

const VAULT_KEY: [u8; 32] = [42; 32];

/// Deterministic pseudo-plaintext spanning several chunks.
fn fixture_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn roundtrips_multi_chunk_blobs() {
    for len in [
        0,
        1,
        BLOB_CHUNK_PLAINTEXT_MAX - 1,
        BLOB_CHUNK_PLAINTEXT_MAX,
        BLOB_CHUNK_PLAINTEXT_MAX * 2 + 77,
    ] {
        let plaintext = fixture_bytes(len);
        let (chunks, manifest) = split_and_encrypt(&VAULT_KEY, &plaintext);
        assert_eq!(manifest.total_size, len as u64);
        assert!(chunks.len() >= 1);
        assert_eq!(chunks.len(), manifest.entries.len());

        let map: ChunkMap = chunks
            .iter()
            .map(|chunk| (chunk.blob_id, chunk.ciphertext.clone()))
            .collect();
        let restored = decrypt_and_reassemble(&VAULT_KEY, &manifest, &map)
            .unwrap_or_else(|error| panic!("len {len}: {error}"));
        assert_eq!(restored, plaintext, "len {len} did not roundtrip");
    }
}

#[test]
fn encryption_is_convergent_and_content_addressed() {
    let plaintext = fixture_bytes(BLOB_CHUNK_PLAINTEXT_MAX + 5);
    let (chunks_a, manifest_a) = split_and_encrypt(&VAULT_KEY, &plaintext);
    let (chunks_b, manifest_b) = split_and_encrypt(&VAULT_KEY, &plaintext);

    // Same vault + same plaintext: byte-identical ciphertext and ids on
    // both sides - the property that makes cross-device dedup work.
    assert_eq!(manifest_a, manifest_b);
    for (a, b) in chunks_a.iter().zip(&chunks_b) {
        assert_eq!(a.blob_id, b.blob_id);
        assert_eq!(a.ciphertext, b.ciphertext);
    }

    // A different vault key yields different ciphertext for the same
    // plaintext (the vault boundary holds on the blob plane too).
    let other_key = [7; 32];
    let (chunks_other, _) = split_and_encrypt(&other_key, &plaintext);
    assert_ne!(chunks_a[0].ciphertext, chunks_other[0].ciphertext);

    // The whole-blob id covers the ordered ciphertexts.
    assert_eq!(blob_id_of(&manifest_a), blob_id_of(&manifest_b));
}

#[test]
fn tampered_or_missing_chunks_fail_closed() {
    let plaintext = fixture_bytes(BLOB_CHUNK_PLAINTEXT_MAX + 5);
    let (chunks, manifest) = split_and_encrypt(&VAULT_KEY, &plaintext);
    let map: ChunkMap = chunks
        .iter()
        .map(|chunk| (chunk.blob_id, chunk.ciphertext.clone()))
        .collect();

    // A single flipped bit fails the chunk's AEAD or its manifest digest.
    let mut tampered = map.clone();
    let first_id = chunks[0].blob_id;
    let mut bad = tampered[&first_id].clone();
    bad[0] ^= 0x01;
    tampered.insert(first_id, bad);
    assert!(decrypt_and_reassemble(&VAULT_KEY, &manifest, &tampered).is_err());

    // A missing chunk fails closed with the typed error, never partial
    // plaintext.
    let mut incomplete: ChunkMap = map.clone();
    incomplete.remove(&chunks[1].blob_id);
    let error = decrypt_and_reassemble(&VAULT_KEY, &manifest, &incomplete)
        .expect_err("incomplete blob reassembled");
    assert!(matches!(
        error,
        altior_crypto::CryptoError::BlobChunkMissing { index: 1, .. }
    ));
    let _ = BlobManifest {
        total_size: 0,
        entries: Vec::new(),
    };
}

#[test]
fn manifest_mismatch_is_refused() {
    let plaintext_one = fixture_bytes(64);
    let plaintext_two = fixture_bytes(BLOB_CHUNK_PLAINTEXT_MAX + 3);
    let (chunks_two, _) = split_and_encrypt(&VAULT_KEY, &plaintext_two);

    // Present the right chunk id holding the WRONG ciphertext: the
    // transport digest passes (the id names the wrong bytes), but the
    // convergent key and AEAD tag refuse it - defense in depth behind
    // the manifest's signed pointer.
    let (chunks_one, manifest_one) = split_and_encrypt(&VAULT_KEY, &plaintext_one);
    let mut map: ChunkMap = chunks_two
        .iter()
        .map(|chunk| (chunk.blob_id, chunk.ciphertext.clone()))
        .collect();
    map.insert(chunks_one[0].blob_id, chunks_two[0].ciphertext.clone());
    assert!(decrypt_and_reassemble(&VAULT_KEY, &manifest_one, &map).is_err());
}
