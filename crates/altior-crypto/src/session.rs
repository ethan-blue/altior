//! The encrypted two-device envelope (ADR 0011).
//!
//! Construction follows the Signal lineage, simplified to a spike: an
//! X25519 static-static ECDH (one key pair per device, no ratchet yet)
//! feeds HKDF-SHA256; the 32-byte session key seals every message
//! with ChaCha20-Poly1305 under a counter-derived nonce; the envelope
//! carries `version || counter || nonce || ciphertext+tag`; and the
//! associated data binds version, sender, receiver, and counter into
//! the authentication tag. Delivery is at-least-once, so receiving
//! runs a 64-delivery sliding replay window (the DTLS/DTLS-SRTP
//! pattern; Signal's per-message counters are the same idea).
//!
//! Counters start at 1 and never repeat within a session, which is
//! exactly what makes a counter-nonce safe: nonce reuse under one key
//! is structurally impossible, and no RNG sits in the message path.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::device::{DeviceIdentity, DeviceKeys};
use crate::error::CryptoError;

/// The envelope wire version this build speaks.
const ENVELOPE_VERSION: u8 = 1;

/// HKDF domain-separation info string.
const KDF_DOMAIN: &[u8] = b"altior/crypto/session-key";

/// Counter bytes in the envelope.
const COUNTER_LEN: usize = 8;

/// Header: version byte + counter.
const HEADER_LEN: usize = 1 + COUNTER_LEN;

/// ChaCha20-Poly1305 nonce: 4 zero bytes + counter.
const NONCE_LEN: usize = 12;

/// Poly1305 tag.
const TAG_LEN: usize = 16;

/// Smallest legal envelope: header + nonce + tag over empty plaintext.
const MIN_ENVELOPE_LEN: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// One direction of the encrypted channel between two known devices.
///
/// The key is symmetric (static-static ECDH), but sender and receiver
/// differ per direction, so a pair of devices holds two `Session`s
/// per side: `outbound` for sending, `inbound` for receiving.
pub struct Session {
    key: Zeroizing<[u8; 32]>,
    sender: crate::device::DeviceId,
    receiver: crate::device::DeviceId,
    send_counter: u64,
    replay: ReplayWindow,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("sender", &self.sender)
            .field("receiver", &self.receiver)
            .field("send_counter", &self.send_counter)
            .field("replay", &self.replay)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl Session {
    /// The session for sending from `local` to `peer`.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DeviceIdCollision`] when the endpoints
    /// do not define a direction.
    pub fn outbound(local: &DeviceKeys, peer: &DeviceIdentity) -> Result<Self, CryptoError> {
        let key = derive_key(local, peer, local.id(), peer.id())?;
        Ok(Self {
            key,
            sender: local.id().clone(),
            receiver: peer.id().clone(),
            send_counter: 0,
            replay: ReplayWindow::default(),
        })
    }

    /// The session for receiving what `peer` sends to `local`.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DeviceIdCollision`] when the endpoints
    /// do not define a direction.
    pub fn inbound(local: &DeviceKeys, peer: &DeviceIdentity) -> Result<Self, CryptoError> {
        let key = derive_key(local, peer, peer.id(), local.id())?;
        Ok(Self {
            key,
            sender: peer.id().clone(),
            receiver: local.id().clone(),
            send_counter: 0,
            replay: ReplayWindow::default(),
        })
    }

    /// Seals `plaintext` into a fresh envelope. Counters advance by
    /// one per envelope from 1, so identical plaintexts seal to
    /// different (and reproducible) wire bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::CounterExhausted`] if the 64-bit
    /// counter space is used up (rekey, never reuse); AEAD sealing
    /// itself cannot fail for a valid key.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.send_counter = self
            .send_counter
            .checked_add(1)
            .ok_or(CryptoError::CounterExhausted)?;
        let counter = self.send_counter;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[NONCE_LEN - COUNTER_LEN..].copy_from_slice(&counter.to_be_bytes());
        let aad = associated_data(&self.sender, &self.receiver, counter);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(self.key.as_ref()));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|source| CryptoError::EnvelopeSeal { source })?;

        let mut envelope = Vec::with_capacity(HEADER_LEN + NONCE_LEN + ciphertext.len());
        envelope.push(ENVELOPE_VERSION);
        envelope.extend_from_slice(&counter.to_be_bytes());
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    /// Authenticates and decrypts an envelope, then runs the replay
    /// window. Order matters: authentication first (never act on
    /// unauthenticated counters), window second.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EnvelopeTruncated`] or
    /// [`CryptoError::EnvelopeVersion`] for malformed input;
    /// [`CryptoError::EnvelopeOpen`] on any authentication failure;
    /// [`CryptoError::ReplayRejected`] for a duplicate or stale
    /// counter.
    ///
    /// # Panics
    ///
    /// Panics when the envelope is long enough for a header but the
    /// fixed-size slices still misalign — structurally impossible
    /// after the length check, so the panic marks a programming error.
    pub fn open(&mut self, envelope: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if envelope.len() < MIN_ENVELOPE_LEN {
            return Err(CryptoError::EnvelopeTruncated {
                size: envelope.len(),
            });
        }
        if envelope[0] != ENVELOPE_VERSION {
            return Err(CryptoError::EnvelopeVersion { found: envelope[0] });
        }
        let counter =
            u64::from_be_bytes(envelope[1..HEADER_LEN].try_into().expect("8 counter bytes"));
        let nonce_bytes: [u8; NONCE_LEN] = envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN]
            .try_into()
            .expect("12 nonce bytes");

        let aad = associated_data(&self.sender, &self.receiver, counter);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(self.key.as_ref()));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &envelope[HEADER_LEN + NONCE_LEN..],
                    aad: &aad,
                },
            )
            .map_err(|source| CryptoError::EnvelopeOpen { source })?;

        if !self.replay.accept(counter) {
            return Err(CryptoError::ReplayRejected { counter });
        }
        Ok(plaintext)
    }

    /// The next counter `seal` will use (1 on a fresh session).
    #[must_use]
    pub fn next_counter(&self) -> u64 {
        self.send_counter + 1
    }
}

/// Static-static ECDH into one directional 32-byte session key. The
/// protocol version, purpose, sender, and receiver are length-bound
/// in HKDF info, so opposite directions never reuse a key.
///
/// # Panics
///
/// Panics when HKDF rejects a 32-byte output — impossible for
/// SHA-256.
fn derive_key(
    local: &DeviceKeys,
    peer: &DeviceIdentity,
    sender: &crate::device::DeviceId,
    receiver: &crate::device::DeviceId,
) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    if sender == receiver {
        return Err(CryptoError::DeviceIdCollision {
            id: sender.to_string(),
        });
    }
    let shared = local
        .static_secret()
        .diffie_hellman(&x25519_dalek::PublicKey::from(peer.x25519_public()));

    let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
    let mut info = Vec::from(KDF_DOMAIN);
    info.push(ENVELOPE_VERSION);
    info.extend_from_slice(b"send-direction");
    push_id(&mut info, sender);
    push_id(&mut info, receiver);
    let mut key = Zeroizing::new([0u8; 32]);
    hk.expand(&info, key.as_mut()).expect("32-byte okm");
    Ok(key)
}

/// Length-prefixed id bytes, so `("ab", "c")` and `("a", "bc")` never
/// produce the same associated data.
fn push_id(buf: &mut Vec<u8>, id: &crate::device::DeviceId) {
    buf.extend_from_slice(&(id.as_str().len() as u64).to_be_bytes());
    buf.extend_from_slice(id.as_str().as_bytes());
}

/// The associated data: version, sender, receiver, counter — every
/// routing claim an attacker could rewrite is authenticated.
fn associated_data(
    sender: &crate::device::DeviceId,
    receiver: &crate::device::DeviceId,
    counter: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(1 + 2 * 8 + sender.as_str().len() + receiver.as_str().len());
    aad.push(ENVELOPE_VERSION);
    push_id(&mut aad, sender);
    push_id(&mut aad, receiver);
    aad.extend_from_slice(&counter.to_be_bytes());
    aad
}

/// A 64-delivery sliding replay window: accept anything above the
/// highest seen counter, accept reordering within the window, reject
/// duplicates and anything older than the window.
///
/// Counters start at 1; 0 is never accepted.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    highest: u64,
    seen: u64,
}

impl ReplayWindow {
    /// Records `counter` if it is new enough, returning whether it was
    /// accepted. A return of `false` means duplicate or stale — the
    /// caller drops the message.
    pub fn accept(&mut self, counter: u64) -> bool {
        let acceptable = counter != 0
            && (counter > self.highest || {
                let delta = self.highest - counter;
                delta < 64 && self.seen & (1 << delta) == 0
            });
        if acceptable {
            self.record(counter);
        }
        acceptable
    }

    fn record(&mut self, counter: u64) {
        if counter > self.highest {
            let delta = counter - self.highest;
            if delta >= 64 {
                self.seen = 0;
            } else {
                self.seen <<= delta;
            }
            self.highest = counter;
            self.seen |= 1;
        } else if counter > 0 {
            self.seen |= 1 << (self.highest - counter);
        }
    }
}
