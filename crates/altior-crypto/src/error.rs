//! Typed failures for the two-device crypto spike (ADR 0011).

use std::error::Error;
use std::fmt;

/// Typed failure for a crypto operation.
///
/// `#[non_exhaustive]` so new failure kinds can be added without a
/// breaking release. AEAD failures are deliberately opaque: the
/// underlying `aead::Error` distinguishes nothing, so callers must not
/// branch on why an envelope failed to open — only that it did.
#[derive(Debug)]
#[non_exhaustive]
pub enum CryptoError {
    /// A device id is empty or exceeds the protocol bound.
    DeviceIdInvalid {
        /// The invalid UTF-8 byte length.
        length: usize,
        /// The maximum accepted UTF-8 byte length.
        max_length: usize,
    },
    /// Both sides of a pairing/session used the same device id.
    DeviceIdCollision {
        /// The duplicated id.
        id: String,
    },
    /// The deterministic seed is the reserved all-zero value.
    SeedAllZero,
    /// The envelope is shorter than header + nonce + tag; not ours or
    /// damaged in transit.
    EnvelopeTruncated {
        /// The received size in bytes.
        size: usize,
    },
    /// The envelope's version byte names a protocol version this code
    /// does not speak.
    EnvelopeVersion {
        /// The version byte that was found.
        found: u8,
    },
    /// Sealing failed at the AEAD layer.
    EnvelopeSeal {
        /// The underlying AEAD failure.
        source: chacha20poly1305::aead::Error,
    },
    /// Opening failed: wrong key, tampered bytes, or mismatched
    /// context (sender/receiver/counter). All cases are one failure on
    /// purpose — no oracle about which check tripped.
    EnvelopeOpen {
        /// The underlying AEAD failure.
        source: chacha20poly1305::aead::Error,
    },
    /// A well-formed, authenticated envelope was already delivered, or
    /// is older than the replay window allows.
    ReplayRejected {
        /// The counter carried by the rejected envelope.
        counter: u64,
    },
    /// The peer signature over the pairing transcript did not verify
    /// against the presented identity.
    SignatureInvalid,
    /// The send counter exhausted the 64-bit space; the session must
    /// be rekeyed, never reused.
    CounterExhausted,
    /// The session epoch is the reserved all-zero value (v2).
    EpochAllZero,
    /// The turn-ephemeral seed is the reserved all-zero value (v2).
    EphemeralSeedAllZero,
    /// The X25519 shared secret came out all-zero — the peer key is
    /// low-order or non-contributory (v2, ADR 0027 §4.5).
    DegenerateSharedSecret,
    /// The leased counter block ran out; the caller must durably reserve
    /// the next block and install it (v2). Sealing never reuses.
    CounterBlockExhausted,
    /// An installed counter block starts below the session's next
    /// counter — the caller would cause counter reuse (v2). Fails
    /// closed.
    CounterBlockRewind {
        /// The rejected block's start.
        block_start: u64,
        /// The next counter the session would hand out.
        next_counter: u64,
    },
    /// The durable counter lease failed; nothing is sealed (v2).
    CounterLeaseFailed {
        /// The underlying storage failure.
        source: std::io::Error,
    },
    /// The envelope names a different epoch or an older epoch sequence
    /// than this session — a foreign pairing or a rolled-back disk (v2).
    EpochMismatch {
        /// The epoch sequence this session expects.
        expected_seq: u32,
        /// The epoch sequence the envelope carried.
        found_seq: u32,
    },
    /// The envelope's turn is not the current or the immediately next
    /// turn — skipped, stale, or forged (v2). Fail closed; re-pair.
    TurnMismatch {
        /// The turn this session is at.
        current: u64,
        /// The turn the envelope carried.
        found: u64,
    },
    /// `open_turn` was called on a receiving session (v2).
    TurnNotOpen,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceIdInvalid { length, max_length } => write!(
                f,
                "device id has {length} bytes; expected 1..={max_length} bytes"
            ),
            Self::DeviceIdCollision { id } => {
                write!(f, "pairing requires distinct device ids; both were {id}")
            }
            Self::SeedAllZero => write!(f, "device key seed is the reserved all-zero value"),
            Self::EnvelopeTruncated { size } => {
                write!(
                    f,
                    "envelope truncated ({size} bytes, below header + nonce + tag)"
                )
            }
            Self::EnvelopeVersion { found } => {
                write!(f, "envelope version {found} is not supported")
            }
            Self::EnvelopeSeal { .. } => write!(f, "envelope sealing failed"),
            Self::EnvelopeOpen { .. } => write!(f, "envelope failed authentication"),
            Self::ReplayRejected { counter } => {
                write!(
                    f,
                    "envelope counter {counter} rejected by the replay window"
                )
            }
            Self::SignatureInvalid => write!(f, "pairing signature did not verify"),
            Self::CounterExhausted => {
                write!(f, "send counter exhausted; rekey the session")
            }
            Self::EpochAllZero => write!(f, "session epoch is the reserved all-zero value"),
            Self::EphemeralSeedAllZero => {
                write!(f, "turn ephemeral seed is the reserved all-zero value")
            }
            Self::DegenerateSharedSecret => write!(
                f,
                "x25519 shared secret is all-zero; peer key is low-order or non-contributory"
            ),
            Self::CounterBlockExhausted => write!(
                f,
                "leased counter block exhausted; reserve and install the next block"
            ),
            Self::CounterBlockRewind {
                block_start,
                next_counter,
            } => write!(
                f,
                "counter block starts at {block_start}, below the next counter {next_counter}"
            ),
            Self::CounterLeaseFailed { source } => {
                write!(f, "counter lease failed: {source}")
            }
            Self::EpochMismatch {
                expected_seq,
                found_seq,
            } => write!(
                f,
                "envelope epoch sequence {found_seq} does not match the expected {expected_seq}"
            ),
            Self::TurnMismatch { current, found } => {
                write!(f, "envelope turn {found} is not the current turn {current}")
            }
            Self::TurnNotOpen => write!(f, "open_turn is an outbound operation"),
        }
    }
}

impl Error for CryptoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        // The AEAD failure is kept in the variants for `Debug`, but
        // `aead::Error` implements no `std::error::Error` (it is
        // deliberately opaque), so there is no `source()` to chain.
        None
    }
}
