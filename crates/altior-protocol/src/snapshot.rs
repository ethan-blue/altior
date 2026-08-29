//! Versioned snapshot envelopes.
//!
//! The documented Desktop flow requests "an initial bounded snapshot for
//! the visible surface" (`docs/UI_ARCHITECTURE.md`); the snapshot reply is
//! a versioned envelope whose payload is bounded by the same envelope
//! limits as commands. A snapshot is a rendering hint, not durable truth:
//! Core remains authoritative and the renderer may discard and re-request
//! it at any time.

use serde::{Deserialize, Serialize};

use altior_domain::{OperationId, ThreadId, UnixMillis};

use crate::bounded::{BoundedPayload, EnvelopeLimits};
use crate::error::ProtocolError;
use crate::version::{ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// A versioned, bounded snapshot reply. No transport is attached;
/// encoding is a plain JSON contract so any P0.2 transport can carry it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct SnapshotEnvelope {
    /// The protocol version negotiated for this connection.
    pub protocol_version: ProtocolVersion,
    /// The operation that requested this snapshot.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub operation_id: OperationId,
    /// The thread the snapshot covers, when scoped to one.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub thread_id: Option<ThreadId>,
    /// Emitter's view of when this snapshot was assembled. Fixtures use
    /// constants.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub as_of: UnixMillis,
    /// The bounded snapshot payload.
    #[cfg_attr(feature = "dto-export", ts(type = "unknown"))]
    pub data: BoundedPayload,
}

impl SnapshotEnvelope {
    /// Decodes a snapshot envelope from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the input is not
    /// a valid envelope.
    pub fn from_json(input: &str) -> Result<Self, ProtocolError> {
        Ok(serde_json::from_str(input)?)
    }

    /// Encodes the envelope as deterministic JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the envelope
    /// cannot be encoded.
    pub fn to_json(&self) -> Result<String, ProtocolError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Validates the envelope against `limits` and the locally supported
    /// protocol versions.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnsupportedProtocolVersion`] when the
    /// envelope's version is outside [`SUPPORTED_PROTOCOL_VERSIONS`] and
    /// [`ProtocolError::PayloadTooLarge`] when the encoded payload exceeds
    /// the limit.
    pub fn validate(&self, limits: &EnvelopeLimits) -> Result<(), ProtocolError> {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(self.protocol_version) {
            return Err(ProtocolError::UnsupportedProtocolVersion {
                requested: self.protocol_version.as_u32(),
                supported: SUPPORTED_PROTOCOL_VERSIONS,
            });
        }
        self.data.ensure_within(limits.payload_bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> SnapshotEnvelope {
        SnapshotEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: "op_fixture000000010".parse().unwrap(),
            thread_id: Some("thr_fixture000000001".parse().unwrap()),
            as_of: UnixMillis::from_millis(1_700_000_000_003),
            data: BoundedPayload::new(
                serde_json::json!({"title": "fixture thread"}),
                EnvelopeLimits::default().payload_bytes,
            )
            .unwrap(),
        }
    }

    #[test]
    fn roundtrips_and_validates() {
        let envelope = envelope();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        let json = envelope.to_json().unwrap();
        assert_eq!(SnapshotEnvelope::from_json(&json).unwrap(), envelope);
    }

    #[test]
    fn rejects_oversized_snapshot_data() {
        let mut envelope = envelope();
        let oversized = serde_json::json!({"blob": "x".repeat(2048)});
        envelope.data =
            BoundedPayload::new(oversized, EnvelopeLimits::default().payload_bytes).unwrap();
        let strict = EnvelopeLimits {
            payload_bytes: 1024,
            diagnostic_bytes: 4096,
        };
        assert!(matches!(
            envelope.validate(&strict),
            Err(ProtocolError::PayloadTooLarge { .. })
        ));
    }
}
