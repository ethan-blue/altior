//! Device key material and public identities (ADR 0011).

use std::fmt;

use ed25519_dalek::SigningKey;
use x25519_dalek::{PublicKey, StaticSecret};

/// A device's short, human-meaningful identifier. Binds into the
/// session HKDF info/context, the envelope associated data, and the
/// pairing transcript — a key pair alone is not an identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(String);

impl DeviceId {
    /// Maximum UTF-8 byte length carried by crypto contexts.
    pub const MAX_LEN: usize = 128;

    /// A device id from any string label (hostnames, pet names — the
    /// pairing transcript is what makes them trustworthy).
    ///
    /// # Errors
    ///
    /// Returns [`crate::CryptoError::DeviceIdInvalid`] for an empty or
    /// over-128-byte UTF-8 label.
    pub fn new(name: impl Into<String>) -> Result<Self, crate::CryptoError> {
        let name = name.into();
        if name.is_empty() || name.len() > Self::MAX_LEN {
            return Err(crate::CryptoError::DeviceIdInvalid {
                length: name.len(),
                max_length: Self::MAX_LEN,
            });
        }
        Ok(Self(name))
    }

    /// The label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The private half of a device: an X25519 static secret for session
/// derivation and an Ed25519 signing key for pairing. Both zeroize on
/// drop (the dalek `zeroize` feature); nothing here ever hits disk in
/// the spike — `SecretStore` (docs/ARCHITECTURE.md) owns persistence
/// later.
pub struct DeviceKeys {
    id: DeviceId,
    static_secret: StaticSecret,
    signing: SigningKey,
}

impl fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceKeys")
            .field("id", &self.id)
            .field("static_secret", &"<redacted>")
            .field("signing", &"<redacted>")
            .finish()
    }
}

impl DeviceKeys {
    /// Deterministic key material from 64 seed bytes: the first 32
    /// become the X25519 static secret (clamped per RFC 7748 by the
    /// library), the last 32 the Ed25519 signing seed. The spike uses
    /// fixed seeds so every test is reproducible; production derives
    /// them from the OS RNG inside `SecretStore`.
    ///
    /// A seed of all zeros is rejected: it is the canonical
    /// "uninitialized" value and never a real key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::CryptoError::SeedAllZero`] for the reserved
    /// uninitialized seed. Caller-controlled input never panics.
    pub fn from_seed(seed: [u8; 64], id: DeviceId) -> Result<Self, crate::CryptoError> {
        if seed == [0; 64] {
            return Err(crate::CryptoError::SeedAllZero);
        }
        let mut x25519 = [0u8; 32];
        x25519.copy_from_slice(&seed[..32]);
        let mut ed25519 = [0u8; 32];
        ed25519.copy_from_slice(&seed[32..]);
        Ok(Self {
            id,
            static_secret: StaticSecret::from(x25519),
            signing: SigningKey::from_bytes(&ed25519),
        })
    }

    /// The matching public identity, safe to share over any channel
    /// (the pairing transcript is what authenticates it).
    #[must_use]
    pub fn public_identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            id: self.id.clone(),
            x25519_public: PublicKey::from(&self.static_secret).to_bytes(),
            ed25519_public: self.signing.verifying_key().to_bytes(),
        }
    }

    pub(crate) fn id(&self) -> &DeviceId {
        &self.id
    }

    pub(crate) fn static_secret(&self) -> &StaticSecret {
        &self.static_secret
    }

    pub(crate) fn signing(&self) -> &SigningKey {
        &self.signing
    }
}

/// The public half of a device: routing id plus both public keys. What
/// two devices exchange when pairing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceIdentity {
    id: DeviceId,
    x25519_public: [u8; 32],
    ed25519_public: [u8; 32],
}

impl DeviceIdentity {
    /// An identity from raw parts — for reconstructing a known peer
    /// from stored bytes and for tests. Authenticity comes from the
    /// pairing signature, never from construction.
    #[must_use]
    pub fn new(id: DeviceId, x25519_public: [u8; 32], ed25519_public: [u8; 32]) -> Self {
        Self {
            id,
            x25519_public,
            ed25519_public,
        }
    }

    /// The device's id.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// The X25519 static public key, used for session derivation.
    #[must_use]
    pub fn x25519_public(&self) -> [u8; 32] {
        self.x25519_public
    }

    /// The Ed25519 verifying key, used for pairing signatures.
    #[must_use]
    pub fn ed25519_public(&self) -> [u8; 32] {
        self.ed25519_public
    }
}
