//! Crypto spike: the encrypted two-device envelope (ADR 0011).
//!
//! `docs/ARCHITECTURE.md` requires end-to-end encryption between the
//! user's own devices with the transport unable to read payloads.
//! This crate proves the minimum honest core: static-static X25519
//! key agreement, HKDF-SHA256 key derivation, ChaCha20-Poly1305
//! envelopes with fully bound associated data, counter nonces,
//! sliding-window replay rejection, and Ed25519 pairing transcripts.
//! No RNG in any library path — keys come from caller-supplied seeds,
//! nonces from counters — so every test is deterministic without
//! sleeps or mocks.
//!
//! Deliberately absent, by scope: forward secrecy (a ratchet), group
//! sessions, PQ hybrids, and any persistence — `SecretStore` owns
//! key storage in a later slice.

pub mod device;
pub mod error;
pub mod pairing;
pub mod session;

pub use device::{DeviceId, DeviceIdentity, DeviceKeys};
pub use error::CryptoError;
pub use pairing::PairingTranscript;
pub use session::{ReplayWindow, Session};
