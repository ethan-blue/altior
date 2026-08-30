//! Typed failures for the relay state machine (ADR 0012).

use std::error::Error;
use std::fmt;

/// Typed failure for a relay operation.
///
/// `#[non_exhaustive]` so new failure kinds can be added without a
/// breaking release.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelayError {
    /// A retained push id was reused for different opaque bytes.
    PushIdCollision {
        /// The conflicting id.
        id: String,
        /// The original retained sequence.
        existing_seq: u64,
    },
    /// The payload exceeds the relay's per-push quota. The push is
    /// refused whole; the bucket is untouched.
    PushTooLarge {
        /// The rejected payload size in bytes.
        size_bytes: usize,
        /// The configured limit in bytes.
        limit_bytes: usize,
    },
    /// The bucket already holds its maximum queue depth of retained
    /// payloads. Backpressure: the sender waits for the receiver to
    /// fetch and the relay to compact, then retries.
    BucketFull {
        /// The configured depth limit.
        depth: usize,
        /// The bucket whose quota was hit.
        bucket: String,
    },
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PushIdCollision { id, existing_seq } => write!(
                f,
                "push id {id} already names different payload at sequence {existing_seq}"
            ),
            Self::PushTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "payload of {size_bytes} bytes exceeds the {limit_bytes}-byte push quota"
            ),
            Self::BucketFull { depth, bucket } => {
                write!(f, "bucket {bucket} is full ({depth} retained payloads)")
            }
        }
    }
}

impl Error for RelayError {}
