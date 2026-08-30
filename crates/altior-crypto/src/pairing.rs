//! The pairing transcript (ADR 0011).
//!
//! Pairing is where a machine-in-the-middle would substitute keys, so
//! the transcript makes the *whole* exchange tamper-evident: both
//! devices canonicalize the same bytes — both ids and both public key
//! pairs, in canonical id order — and each signs those bytes with its
//! Ed25519 identity key. A signature verifies only against the exact
//! identities the signer saw, so substituted keys or renamed devices
//! fail verification. This is Signal's safety-number discipline
//! reduced to signatures: instead of humans comparing digits, the
//! software compares signatures and humans only decide to trust the
//! channel the identities traveled over.

use ed25519_dalek::Signer;

use crate::device::{DeviceIdentity, DeviceKeys};
use crate::error::CryptoError;

/// The canonical pairing transcript. Byte-identical on both devices
/// regardless of construction order.
#[derive(Debug)]
pub struct PairingTranscript {
    bytes: Vec<u8>,
}

impl PairingTranscript {
    /// The transcript over the two exchanged identities. Field order
    /// is canonical (id order), so `new(a, b)` and `new(b, a)`
    /// produce identical bytes and both sides can verify the same
    /// signatures.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DeviceIdCollision`] when the endpoints
    /// have the same device id.
    pub fn new(a: &DeviceIdentity, b: &DeviceIdentity) -> Result<Self, CryptoError> {
        if a.id() == b.id() {
            return Err(CryptoError::DeviceIdCollision {
                id: a.id().to_string(),
            });
        }
        let mut bytes = Vec::from(&b"altior/crypto/v1/pairing"[..]);
        match a.id().cmp(b.id()) {
            std::cmp::Ordering::Less => {
                push_identity(&mut bytes, a);
                push_identity(&mut bytes, b);
            }
            std::cmp::Ordering::Greater => {
                push_identity(&mut bytes, b);
                push_identity(&mut bytes, a);
            }
            std::cmp::Ordering::Equal => unreachable!("equal ids rejected above"),
        }
        Ok(Self { bytes })
    }

    /// The canonical transcript bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Signs the transcript with the local device's Ed25519 identity
    /// key; send the result to the peer alongside the public identity.
    #[must_use]
    pub fn sign(&self, keys: &DeviceKeys) -> [u8; 64] {
        keys.signing().sign(&self.bytes).to_bytes()
    }

    /// Verifies a peer's signature over this transcript against the
    /// peer identity it presented. Any mismatch — wrong key, wrong
    /// ids, altered transcript — is one failure, `SignatureInvalid`.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::SignatureInvalid`] when the presented
    /// identity, the signature, and the transcript do not all agree.
    pub fn verify(&self, signer: &DeviceIdentity, signature: &[u8]) -> Result<(), CryptoError> {
        let verifying = ed25519_dalek::VerifyingKey::from_bytes(&signer.ed25519_public())
            .map_err(|_| CryptoError::SignatureInvalid)?;
        let signature = ed25519_dalek::Signature::from_slice(signature)
            .map_err(|_| CryptoError::SignatureInvalid)?;
        verifying
            .verify_strict(&self.bytes, &signature)
            .map_err(|_| CryptoError::SignatureInvalid)
    }
}

/// One identity into the transcript: length-prefixed id, then both
/// fixed-size public keys.
fn push_identity(buf: &mut Vec<u8>, identity: &DeviceIdentity) {
    let id = identity.id();
    buf.extend_from_slice(&(id.as_str().len() as u64).to_be_bytes());
    buf.extend_from_slice(id.as_str().as_bytes());
    buf.extend_from_slice(&identity.x25519_public());
    buf.extend_from_slice(&identity.ed25519_public());
}
