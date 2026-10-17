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
    /// The push id (addressing metadata) exceeds the relay's metadata
    /// quota. Refused whole; the bucket is untouched. (ADR 0025 threat 7:
    /// metadata is bounded at 4 KiB.)
    PushIdTooLarge {
        /// The rejected id size in bytes.
        size_bytes: usize,
        /// The configured limit in bytes.
        limit_bytes: usize,
    },
    /// The bucket's retained payload bytes would exceed its byte quota.
    /// Backpressure like : the sender waits for fetches and
    /// compaction, then retries. (ADR 0025 threat 7: per-vault quota.)
    BucketQuotaExceeded {
        /// The bucket whose quota was hit.
        bucket: String,
        /// The byte size the new push would have added.
        incoming_bytes: usize,
        /// The configured byte limit.
        limit_bytes: usize,
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
            Self::PushIdTooLarge {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "push id of {size_bytes} bytes exceeds the {limit_bytes}-byte metadata quota"
            ),
            Self::BucketQuotaExceeded {
                bucket,
                incoming_bytes,
                limit_bytes,
            } => write!(
                f,
                "bucket {bucket} would exceed its {limit_bytes}-byte quota (adding {incoming_bytes})"
            ),
        }
    }
}

impl Error for RelayError {}
