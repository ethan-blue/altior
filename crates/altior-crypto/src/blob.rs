//! The blob plane's convergent chunk encryption (ADR 0028).
//!
//! A blob splits into fixed 64 KiB plaintext chunks. Each chunk is
//! sealed under a **convergent** key — HKDF over the vault data key with
//! the chunk's own content hash as salt — so identical plaintext yields
//! identical ciphertext and an identical storage identity on every
//! device, without coordination. Nonces are derived per (chunk key,
//! chunk index), never random: distinct contents derive distinct keys,
//! and one content's chunks have distinct indices, so no (key, nonce)
//! pair repeats. The AEAD's associated data binds the content hash,
//! chunk index, chunk count, and total size — every claim a reassembler
//! could check is authenticated.
//!
//! Identities: `content_hash = SHA-256(plaintext)` and per-chunk
//! `blob_id = SHA-256(ciphertext)`. The blob id is the hash of exactly
//! the bytes at rest, so any store verifies a fetched chunk against its
//! own identity, and the manifest's ids prove the whole. No RNG sits in
//! the message path; the vault data key comes from the caller (pairing
//! owns it, ADR 0027 §5).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// Largest plaintext chunk before splitting (64 KiB).
pub const BLOB_CHUNK_PLAINTEXT_MAX: usize = 64 * 1024;

/// The blob format version, bound into every key derivation and AAD.
const BLOB_FORMAT_VERSION: u8 = 1;

const KDF_DOMAIN: &[u8] = b"altior/blob/chunk-key";
const NONCE_DOMAIN: &[u8] = b"altior/blob/nonce";

/// A chunk's stored form: its ciphertext plus both identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkCiphertext {
    /// `SHA-256(ciphertext)` — the at-rest storage identity.
    pub blob_id: [u8; 32],
    /// `SHA-256(plaintext)` — the convergent salt the chunk was sealed
    /// under; safe to publish (a hash of the plaintext is inherent to
    /// convergent encryption and is what makes dedup possible).
    pub content_hash: [u8; 32],
    /// The sealed bytes.
    pub ciphertext: Vec<u8>,
}

/// One manifest entry: the pointer a peer needs to fetch and verify a
/// chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkManifestEntry {
    /// `SHA-256(plaintext)`.
    pub content_hash: [u8; 32],
    /// `SHA-256(ciphertext)` — the CAS id to request.
    pub blob_id: [u8; 32],
    /// The ciphertext length in bytes.
    pub ciphertext_len: usize,
}

/// The ordered chunk list plus total size — small enough to travel as a
/// signed journal event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobManifest {
    /// Total plaintext size in bytes.
    pub total_size: u64,
    /// One entry per chunk, in order.
    pub entries: Vec<ChunkManifestEntry>,
}

/// The whole-blob identity: `SHA-256(ciphertext_0 ‖ ciphertext_1 ‖ …)`.
#[must_use]
pub fn blob_id_of(manifest: &BlobManifest) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for entry in &manifest.entries {
        hasher.update(entry.blob_id);
    }
    hasher.finalize().into()
}

/// The convergent chunk key: HKDF over the vault data key with the
/// content hash as salt.
fn chunk_key(vault_key: &[u8; 32], content_hash: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(Some(content_hash), vault_key);
    let mut info = Vec::from(KDF_DOMAIN);
    info.push(BLOB_FORMAT_VERSION);
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(&info, key.as_mut())
        .expect("32-byte okm from sha256");
    key
}

/// The deterministic nonce for one chunk of one content.
fn chunk_nonce(key: &[u8; 32], chunk_index: usize) -> [u8; 12] {
    let hk = Hkdf::<Sha256>::new(None, key);
    let mut info = Vec::from(NONCE_DOMAIN);
    info.push(BLOB_FORMAT_VERSION);
    info.extend_from_slice(&(chunk_index as u64).to_be_bytes());
    let mut nonce = [0u8; 12];
    hk.expand(&info, &mut nonce)
        .expect("12-byte nonce from sha256");
    nonce
}

fn associated_data(
    content_hash: &[u8; 32],
    chunk_index: usize,
    total_chunks: usize,
    total_size: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(1 + 32 + 8 + 8 + 8);
    aad.push(BLOB_FORMAT_VERSION);
    aad.extend_from_slice(content_hash);
    aad.extend_from_slice(&(chunk_index as u64).to_be_bytes());
    aad.extend_from_slice(&(total_chunks as u64).to_be_bytes());
    aad.extend_from_slice(&total_size.to_be_bytes());
    aad
}

/// Splits and encrypts `plaintext` into chunks plus the manifest.
///
/// An empty plaintext yields one zero-length chunk (a valid, empty blob)
/// rather than a special case.
///
/// # Panics
///
/// Panics if HKDF rejects its fixed output lengths — impossible for
/// SHA-256, so the panic marks a programming error.
#[must_use]
pub fn split_and_encrypt(
    vault_key: &[u8; 32],
    plaintext: &[u8],
) -> (Vec<ChunkCiphertext>, BlobManifest) {
    let total_size = plaintext.len() as u64;
    // An empty plaintext is one zero-length chunk: a valid empty blob,
    // no special case downstream.
    let slices: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[]]
    } else {
        plaintext.chunks(BLOB_CHUNK_PLAINTEXT_MAX).collect()
    };
    let total_chunks = slices.len();
    let mut chunks = Vec::with_capacity(total_chunks);
    let mut entries = Vec::with_capacity(total_chunks);
    for (index, chunk) in slices.into_iter().enumerate() {
        let content_hash: [u8; 32] = Sha256::digest(chunk).into();
        let key = chunk_key(vault_key, &content_hash);
        let nonce = chunk_nonce(&key, index);
        let aad = associated_data(&content_hash, index, total_chunks, total_size);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: chunk,
                    aad: &aad,
                },
            )
            .expect("sealing with a valid key cannot fail");
        let blob_id: [u8; 32] = Sha256::digest(&ciphertext).into();
        entries.push(ChunkManifestEntry {
            content_hash,
            blob_id,
            ciphertext_len: ciphertext.len(),
        });
        chunks.push(ChunkCiphertext {
            blob_id,
            content_hash,
            ciphertext,
        });
    }
    (
        chunks,
        BlobManifest {
            total_size,
            entries,
        },
    )
}

/// Decrypts and reassembles a blob from its fetched chunks. Every chunk
/// is authenticated against its manifest entry before any plaintext is
/// produced; a missing or failing chunk fails closed with the offending
/// index.
///
/// # Errors
///
/// [`CryptoError::BlobChunkMissing`] when a manifest chunk was not
/// supplied; [`CryptoError::EnvelopeOpen`] when a chunk fails
/// authentication or its digest does not match the manifest.
pub fn decrypt_and_reassemble(
    vault_key: &[u8; 32],
    manifest: &BlobManifest,
    chunks: &ChunkMap,
) -> Result<Vec<u8>, CryptoError> {
    let total_chunks = manifest.entries.len();
    let mut plaintext =
        Vec::with_capacity(usize::try_from(manifest.total_size).unwrap_or(usize::MAX));
    for (index, entry) in manifest.entries.iter().enumerate() {
        let ciphertext = chunks
            .get(&entry.blob_id)
            .ok_or(CryptoError::BlobChunkMissing {
                index,
                blob_id: entry.blob_id,
            })?;
        if ciphertext.len() != entry.ciphertext_len {
            return Err(CryptoError::BlobChunkMismatch { index });
        }
        let key = chunk_key(vault_key, &entry.content_hash);
        let nonce = chunk_nonce(&key, index);
        let aad = associated_data(
            &entry.content_hash,
            index,
            total_chunks,
            manifest.total_size,
        );
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
        let chunk_plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|source| CryptoError::EnvelopeOpen { source })?;
        let content_hash: [u8; 32] = Sha256::digest(&chunk_plaintext).into();
        if content_hash != entry.content_hash {
            return Err(CryptoError::BlobChunkMismatch { index });
        }
        plaintext.extend_from_slice(&chunk_plaintext);
    }
    if plaintext.len() as u64 != manifest.total_size {
        return Err(CryptoError::BlobChunkMismatch {
            index: total_chunks,
        });
    }
    Ok(plaintext)
}

/// Chunk lookup keyed by chunk blob id.
pub type ChunkMap = std::collections::BTreeMap<[u8; 32], Vec<u8>>;
