//! Typed protocol errors shared by every Desktop/Core contract.

use std::fmt;

use crate::version::ProtocolVersionRange;

/// Typed failure for a protocol contract operation.
///
/// Public fallible functions return this type; no public API communicates
/// failure through an ad-hoc string. The enum is `#[non_exhaustive]` so
/// new failure kinds can be added without a breaking release. It is not
/// `Clone`/`PartialEq` because [`Self::MalformedEnvelope`] carries the
/// underlying decoder error, which is neither.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProtocolError {
    /// A version number outside the representable protocol range (zero).
    InvalidProtocolVersion {
        /// The rejected version number.
        version: u32,
    },
    /// A protocol version string that is not a positive integer.
    MalformedProtocolVersion {
        /// The rejected input.
        value: String,
    },
    /// A version range whose minimum exceeds its maximum.
    InvalidProtocolVersionRange {
        /// The rejected lower bound.
        min: u32,
        /// The rejected upper bound.
        max: u32,
    },
    /// A version outside the locally supported range.
    UnsupportedProtocolVersion {
        /// The rejected version.
        requested: u32,
        /// The range this endpoint supports.
        supported: ProtocolVersionRange,
    },
    /// The Desktop and Core version ranges have no common version.
    NoCommonProtocolVersion {
        /// The range advertised by Desktop.
        desktop: ProtocolVersionRange,
        /// The range advertised by Core.
        core: ProtocolVersionRange,
    },
    /// A product version string that is not `major.minor.patch`.
    MalformedProductVersion {
        /// The rejected input.
        value: String,
    },
    /// A capability id outside the canonical `[a-z0-9.-]` charset or size.
    MalformedCapabilityId {
        /// The rejected input.
        value: String,
    },
    /// A command kind this protocol version does not define.
    UnsupportedCommandKind {
        /// The rejected kind name.
        kind: String,
    },
    /// An event kind name that is empty, malformed, or too long.
    MalformedEventKind {
        /// The rejected kind name.
        kind: String,
    },
    /// A sequence number of zero. Sequences are 1-based.
    ZeroSequence,
    /// A sequence increment past the representable maximum.
    SequenceOverflow {
        /// The maximum sequence value that overflowed.
        at: u64,
    },
    /// A launch token containing a character outside lowercase hex.
    InvalidLaunchTokenCharacter {
        /// The offending character.
        character: char,
        /// The byte offset of `character` within the token.
        position: usize,
    },
    /// A launch token outside its length bounds (32 to 128 hex chars).
    LaunchTokenLength {
        /// The token length that was found.
        length: usize,
    },
    /// A retained-event window whose start follows its end.
    InvalidRetainedWindow {
        /// The rejected inclusive start.
        from: u64,
        /// The rejected inclusive end.
        through: u64,
    },
    /// An encoded payload larger than the negotiated limit.
    PayloadTooLarge {
        /// The encoded payload size in bytes.
        size_bytes: usize,
        /// The maximum allowed size in bytes.
        limit_bytes: usize,
    },
    /// Text longer than its type-level byte cap.
    TextTooLarge {
        /// The text length in bytes.
        size_bytes: usize,
        /// The maximum allowed size in bytes.
        limit_bytes: usize,
    },
    /// An envelope that could not be decoded or violates its contract.
    MalformedEnvelope {
        /// The underlying decoding failure.
        source: serde_json::Error,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProtocolVersion { version } => {
                write!(f, "protocol version {version} is not a positive integer")
            }
            Self::MalformedProtocolVersion { value } => {
                write!(f, "malformed protocol version {value:?}")
            }
            Self::InvalidProtocolVersionRange { min, max } => {
                write!(f, "protocol version range min {min} exceeds max {max}")
            }
            Self::UnsupportedProtocolVersion {
                requested,
                supported,
            } => write!(
                f,
                "protocol version {requested} is outside the supported range {supported}"
            ),
            Self::NoCommonProtocolVersion { desktop, core } => write!(
                f,
                "no common protocol version: desktop supports {desktop}, core supports {core}"
            ),
            Self::MalformedProductVersion { value } => {
                write!(f, "malformed product version {value:?}")
            }
            Self::MalformedCapabilityId { value } => {
                write!(f, "malformed capability id {value:?}")
            }
            Self::UnsupportedCommandKind { kind } => {
                write!(f, "unsupported command kind {kind:?}")
            }
            Self::MalformedEventKind { kind } => write!(f, "malformed event kind {kind:?}"),
            Self::ZeroSequence => write!(f, "sequence numbers start at 1"),
            Self::SequenceOverflow { at } => write!(f, "sequence overflow at {at}"),
            Self::InvalidLaunchTokenCharacter {
                character,
                position,
            } => write!(
                f,
                "launch token has invalid character {character:?} at byte {position}"
            ),
            Self::LaunchTokenLength { length } => write!(
                f,
                "launch token has {length} chars, expected 32 to 128 hex chars"
            ),
            Self::InvalidRetainedWindow { from, through } => {
                write!(f, "retained window starts at {from} but ends at {through}")
            }
            Self::PayloadTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "payload is {size_bytes} bytes, limit is {limit_bytes} bytes"
            ),
            Self::TextTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "text is {size_bytes} bytes, limit is {limit_bytes} bytes"
            ),
            Self::MalformedEnvelope { source } => write!(f, "malformed envelope: {source}"),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MalformedEnvelope { source } => Some(source),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(source: serde_json::Error) -> Self {
        Self::MalformedEnvelope { source }
    }
}
