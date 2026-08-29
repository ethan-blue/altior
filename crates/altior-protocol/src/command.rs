//! Versioned command envelopes sent from Desktop to Core.
//!
//! P0.1 defines only the envelope mechanics and the command kinds the
//! documented IPC flow needs: transport health checks, the initial bounded
//! snapshot request, and cooperative cancellation. Real thread/turn
//! commands arrive with the P1 domain runtime and must not be invented
//! here. Unknown command kinds fail explicitly; commands are requests, not
//! forward-compatible observations (ADR 0004).

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;

use altior_domain::{OperationId, UnixMillis};

use crate::bounded::{BoundedPayload, EnvelopeLimits};
use crate::error::ProtocolError;
use crate::version::{ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// The command kinds defined by protocol version 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        rename_all = "snake_case"
    )
)]
pub enum CommandKind {
    /// Transport liveness check used for Core health monitoring.
    Ping,
    /// Request an initial bounded snapshot for the visible surface.
    RequestSnapshot,
    /// Cooperatively cancel the operation named in the payload.
    Cancel,
}

impl CommandKind {
    /// Returns the canonical wire name of this kind.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::RequestSnapshot => "request_snapshot",
            Self::Cancel => "cancel",
        }
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_name())
    }
}

impl FromStr for CommandKind {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ping" => Ok(Self::Ping),
            "request_snapshot" => Ok(Self::RequestSnapshot),
            "cancel" => Ok(Self::Cancel),
            other => Err(ProtocolError::UnsupportedCommandKind {
                kind: other.to_owned(),
            }),
        }
    }
}

/// A versioned command envelope. No transport is attached; encoding is a
/// plain JSON contract so any P0.2 transport can carry it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CommandEnvelope {
    /// The protocol version negotiated for this connection.
    pub protocol_version: ProtocolVersion,
    /// The Altior operation this command belongs to.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub operation_id: OperationId,
    /// The command discriminator.
    pub kind: CommandKind,
    /// Optional bounded JSON payload.
    #[cfg_attr(feature = "dto-export", ts(type = "unknown"))]
    pub payload: Option<BoundedPayload>,
    /// When the sender issued the command. Supplied by the sender's clock;
    /// fixtures use constants.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub issued_at: UnixMillis,
}

impl CommandEnvelope {
    /// Decodes a command envelope from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the input is not
    /// a valid envelope, including unknown command kinds.
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

    /// Builds a cooperative cancellation command for `target`.
    ///
    /// Cancellation is a convention, not a mechanism: the command names an
    /// operation, is idempotent, and never carries prompt-retry semantics
    /// (ADR 0002). The target rides in the payload as
    /// `{"operation_id": "..."}`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the payload exceeds
    /// `limits.payload_bytes`.
    pub fn cancel(
        target: &OperationId,
        operation_id: OperationId,
        issued_at: UnixMillis,
        limits: &EnvelopeLimits,
    ) -> Result<Self, ProtocolError> {
        let payload = BoundedPayload::new(
            json!({ "operation_id": target.as_str() }),
            limits.payload_bytes,
        )?;
        Ok(Self {
            protocol_version: ProtocolVersion::V1,
            operation_id,
            kind: CommandKind::Cancel,
            payload: Some(payload),
            issued_at,
        })
    }

    /// Returns the target operation of a `cancel` command.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the command is a
    /// `cancel` whose payload is missing or does not carry a well-formed
    /// `operation_id`. Non-cancel commands return `Ok(None)`.
    pub fn cancel_target(&self) -> Result<Option<OperationId>, ProtocolError> {
        if self.kind != CommandKind::Cancel {
            return Ok(None);
        }
        let malformed = |message: &'static str| ProtocolError::MalformedEnvelope {
            source: serde_json::Error::custom(message),
        };
        let payload = self
            .payload
            .as_ref()
            .ok_or_else(|| malformed("cancel command without payload"))?;
        let target = payload
            .value()
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| malformed("cancel payload without operation_id"))?;
        let parsed =
            OperationId::try_from(target).map_err(|_| malformed("invalid operation id"))?;
        Ok(Some(parsed))
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
        if let Some(payload) = &self.payload {
            payload.ensure_within(limits.payload_bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_commands_carry_their_target_in_the_payload() {
        let target: OperationId = "op_fixture000000005".parse().unwrap();
        let envelope = CommandEnvelope::cancel(
            &target,
            "op_fixture000000011".parse().unwrap(),
            UnixMillis::from_millis(1_700_000_000_000),
            &EnvelopeLimits::default(),
        )
        .unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        assert_eq!(envelope.kind, CommandKind::Cancel);
        assert_eq!(
            envelope
                .cancel_target()
                .unwrap()
                .map(|id| id.as_str().to_owned()),
            Some("op_fixture000000005".to_owned())
        );

        let json = envelope.to_json().unwrap();
        let decoded = CommandEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn malformed_cancel_payloads_fail_explicitly() {
        let no_payload = CommandEnvelope {
            protocol_version: ProtocolVersion::V1,
            operation_id: "op_fixture000000011".parse().unwrap(),
            kind: CommandKind::Cancel,
            payload: None,
            issued_at: UnixMillis::from_millis(0),
        };
        assert!(matches!(
            no_payload.cancel_target(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));

        let bad_target = CommandEnvelope {
            payload: Some(
                BoundedPayload::new(
                    serde_json::json!({"operation_id": "not-an-id"}),
                    EnvelopeLimits::default().payload_bytes,
                )
                .unwrap(),
            ),
            ..no_payload
        };
        assert!(matches!(
            bad_target.cancel_target(),
            Err(ProtocolError::MalformedEnvelope { .. })
        ));
    }
}
