//! Versioned event envelopes sent from Core to Desktop.
//!
//! The body carries a closed set of known normalized events plus one
//! `Unknown` variant: an unrecognized wire event deserializes into
//! `Unknown` with the provider kind name and a **bounded diagnostic**
//! (the raw event object, capped), so future provider events never crash
//! the process and never enter stable domain records as structured data
//! (ADR 0004, ADR 0006, and `docs/HARNESSES.md`).

use std::fmt;

use serde::de::{Deserializer, Error as _};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use altior_domain::{EventId, OperationId, ThreadId, TurnId, UnixMillis};

use crate::bounded::{BoundedPayload, DiagnosticText, EnvelopeLimits, MessageText};
use crate::error::ProtocolError;
use crate::version::{ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// Maximum length of an event kind name in bytes.
const MAX_KIND_LEN: usize = 64;

/// The known normalized event kinds defined by protocol version 1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/", tag = "kind")
)]
pub enum KnownEvent {
    /// A turn started (`turn.started`).
    #[serde(rename = "turn.started")]
    #[cfg_attr(feature = "dto-export", ts(rename = "turn.started"))]
    TurnStarted,

    /// A streaming text delta (`message.delta`).
    #[serde(rename = "message.delta")]
    #[cfg_attr(feature = "dto-export", ts(rename = "message.delta"))]
    MessageDelta {
        /// The bounded delta text.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        text: MessageText,
    },

    /// A user permission was requested by the agent (`permission.requested`).
    #[serde(rename = "permission.requested")]
    #[cfg_attr(feature = "dto-export", ts(rename = "permission.requested"))]
    PermissionRequested {
        /// Kind of permission: `"execute"`, `"read"`, `"write"`, or `"network"`.
        permission_kind: String,
        /// Bounded description of the requested action.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        description: DiagnosticText,
    },

    /// A user permission decision was recorded (`permission.decided`).
    #[serde(rename = "permission.decided")]
    #[cfg_attr(feature = "dto-export", ts(rename = "permission.decided"))]
    PermissionDecided {
        /// Decision: `"approved"` or `"denied"`.
        decision: String,
    },

    /// A turn completed successfully (`turn.completed`).
    #[serde(rename = "turn.completed")]
    #[cfg_attr(feature = "dto-export", ts(rename = "turn.completed"))]
    TurnCompleted,

    /// A turn failed with an error and delivery classification (`turn.failed`).
    #[serde(rename = "turn.failed")]
    #[cfg_attr(feature = "dto-export", ts(rename = "turn.failed"))]
    TurnFailed {
        /// Bounded failure diagnostic/reason.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        reason: DiagnosticText,
        /// Delivery classification: `"absent"`, `"confirmed"`, `"rejected"`, or `"indeterminate"`.
        delivery_state: String,
    },

    /// A turn was cancelled cooperatively (`turn.cancelled`).
    #[serde(rename = "turn.cancelled")]
    #[cfg_attr(feature = "dto-export", ts(rename = "turn.cancelled"))]
    TurnCancelled {
        /// Optional bounded reason for cancellation.
        #[cfg_attr(feature = "dto-export", ts(type = "string | null", optional))]
        reason: Option<DiagnosticText>,
    },

    /// Core runtime health/status update (`runtime.status`).
    #[serde(rename = "runtime.status")]
    #[cfg_attr(feature = "dto-export", ts(rename = "runtime.status"))]
    RuntimeStatus {
        /// Status string: `"ready"`, `"busy"`, `"degraded"`, `"shutting_down"`.
        status: String,
        /// Number of active threads in memory.
        #[cfg_attr(feature = "dto-export", ts(type = "number"))]
        active_threads: u32,
        /// Optional redacted diagnostic text.
        #[cfg_attr(feature = "dto-export", ts(type = "string | null", optional))]
        diagnostics: Option<DiagnosticText>,
    },

    /// Typed command success response (`command.result`).
    #[serde(rename = "command.result")]
    #[cfg_attr(feature = "dto-export", ts(rename = "command.result"))]
    CommandResult {
        /// Operation ID this result satisfies.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        operation_id: OperationId,
        /// Whether the operation completed successfully.
        #[cfg_attr(feature = "dto-export", ts(type = "boolean"))]
        success: bool,
        /// Optional bounded result payload.
        #[cfg_attr(feature = "dto-export", ts(type = "unknown", optional))]
        data: Option<BoundedPayload>,
    },

    /// Typed command failure response (`command.error`).
    #[serde(rename = "command.error")]
    #[cfg_attr(feature = "dto-export", ts(rename = "command.error"))]
    CommandError {
        /// Operation ID this error belongs to.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        operation_id: OperationId,
        /// Stable error code.
        code: String,
        /// Bounded human-readable error message.
        #[cfg_attr(feature = "dto-export", ts(type = "string"))]
        message: DiagnosticText,
    },

    /// The subscriber's catch-up range is no longer retained; Desktop must
    /// request a snapshot (`stream.gap`, ADR 0006).
    #[serde(rename = "stream.gap")]
    #[cfg_attr(feature = "dto-export", ts(rename = "stream.gap"))]
    StreamGap {
        /// The first sequence the subscriber is missing.
        #[cfg_attr(feature = "dto-export", ts(type = "number"))]
        from: Sequence,
    },

    /// A catch-up replay finished; live delivery follows (`stream.replayed`,
    /// ADR 0006).
    #[serde(rename = "stream.replayed")]
    #[cfg_attr(feature = "dto-export", ts(rename = "stream.replayed"))]
    StreamReplayed {
        /// The first replayed sequence.
        #[cfg_attr(feature = "dto-export", ts(type = "number"))]
        from: Sequence,
        /// The last replayed sequence.
        #[cfg_attr(feature = "dto-export", ts(type = "number"))]
        through: Sequence,
    },
}

/// The body of an event envelope: a known normalized event or a safely
/// preserved unknown future event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        type = r#"{ kind: "turn.started" } | { kind: "message.delta"; text: string } | { kind: "permission.requested"; permission_kind: string; description: string } | { kind: "permission.decided"; decision: string } | { kind: "turn.completed" } | { kind: "turn.failed"; reason: string; delivery_state: string } | { kind: "turn.cancelled"; reason?: string | null } | { kind: "runtime.status"; status: string; active_threads: number; diagnostics?: string | null } | { kind: "command.result"; operation_id: string; success: boolean; data?: unknown } | { kind: "command.error"; operation_id: string; code: string; message: string } | { kind: "stream.gap"; from: number } | { kind: "stream.replayed"; from: number; through: number } | { kind: string; diagnostic: string }"#
    )
)]
pub enum EventBody {
    /// A normalized event this protocol version understands.
    Known(KnownEvent),
    /// An unrecognized event preserved as a bounded diagnostic.
    Unknown {
        /// The provider event kind name, e.g. `usage.updated`.
        provider_kind: String,
        /// The raw event object, capped at the diagnostic byte limit.
        diagnostic: DiagnosticText,
    },
}

impl EventBody {
    /// Returns the event kind name for diagnostics and routing.
    #[must_use]
    pub fn kind_name(&self) -> &str {
        match self {
            Self::Known(KnownEvent::TurnStarted) => "turn.started",
            Self::Known(KnownEvent::MessageDelta { .. }) => "message.delta",
            Self::Known(KnownEvent::PermissionRequested { .. }) => "permission.requested",
            Self::Known(KnownEvent::PermissionDecided { .. }) => "permission.decided",
            Self::Known(KnownEvent::TurnCompleted) => "turn.completed",
            Self::Known(KnownEvent::TurnFailed { .. }) => "turn.failed",
            Self::Known(KnownEvent::TurnCancelled { .. }) => "turn.cancelled",
            Self::Known(KnownEvent::RuntimeStatus { .. }) => "runtime.status",
            Self::Known(KnownEvent::CommandResult { .. }) => "command.result",
            Self::Known(KnownEvent::CommandError { .. }) => "command.error",
            Self::Known(KnownEvent::StreamGap { .. }) => "stream.gap",
            Self::Known(KnownEvent::StreamReplayed { .. }) => "stream.replayed",
            Self::Unknown { provider_kind, .. } => provider_kind,
        }
    }
}

impl fmt::Display for EventBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind_name())
    }
}

fn validate_kind_name(kind: &str) -> Result<(), ProtocolError> {
    if kind.is_empty() || kind.len() > MAX_KIND_LEN {
        return Err(ProtocolError::MalformedEventKind {
            kind: kind.to_owned(),
        });
    }
    Ok(())
}

impl Serialize for EventBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(event) => event.serialize(serializer),
            Self::Unknown {
                provider_kind,
                diagnostic,
            } => {
                let mut object = Map::new();
                object.insert("kind".to_owned(), Value::String(provider_kind.clone()));
                object.insert(
                    "diagnostic".to_owned(),
                    Value::String(diagnostic.as_str().to_owned()),
                );
                Value::Object(object).serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for EventBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let Some(kind) = raw.get("kind").and_then(Value::as_str) else {
            return Err(D::Error::missing_field("kind"));
        };
        match kind {
            "turn.started" => Ok(Self::Known(KnownEvent::TurnStarted)),
            "turn.completed" => Ok(Self::Known(KnownEvent::TurnCompleted)),
            "message.delta"
            | "permission.requested"
            | "permission.decided"
            | "turn.failed"
            | "turn.cancelled"
            | "runtime.status"
            | "command.result"
            | "command.error"
            | "stream.gap"
            | "stream.replayed" => {
                let event: KnownEvent = serde_json::from_value(raw).map_err(D::Error::custom)?;
                Ok(Self::Known(event))
            }
            other => {
                validate_kind_name(other).map_err(|error| D::Error::custom(error.to_string()))?;
                // An already-preserved unknown event serializes to exactly a
                // `kind` plus one string `diagnostic` field. Recognize that
                // shape so encode/decode roundtrips stay stable instead of
                // re-wrapping the diagnostic on every pass.
                let preserved = matches!(
                    raw.as_object(),
                    Some(object)
                        if object.len() == 2
                            && matches!(object.get("diagnostic"), Some(Value::String(_)))
                );
                if preserved {
                    let Value::String(diagnostic) = &raw["diagnostic"] else {
                        return Err(D::Error::custom("unexpected diagnostic shape"));
                    };
                    let diagnostic = DiagnosticText::try_from(diagnostic.clone())
                        .map_err(|error: ProtocolError| D::Error::custom(error.to_string()))?;
                    return Ok(Self::Unknown {
                        provider_kind: other.to_owned(),
                        diagnostic,
                    });
                }
                // A fresh unrecognized wire event is preserved as a bounded
                // diagnostic of the whole raw object.
                let serialized = serde_json::to_string(&raw)
                    .map_err(|error| D::Error::custom(error.to_string()))?;
                let diagnostic = DiagnosticText::try_from(serialized)
                    .map_err(|error: ProtocolError| D::Error::custom(error.to_string()))?;
                Ok(Self::Unknown {
                    provider_kind: other.to_owned(),
                    diagnostic,
                })
            }
        }
    }
}

/// A 1-based position of an event within its ordered stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        type = "number"
    )
)]
pub struct Sequence(u64);

impl Serialize for Sequence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Sequence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u64::deserialize(deserializer)?;
        Self::try_new(raw).map_err(D::Error::custom)
    }
}

impl Sequence {
    /// The first sequence number of a stream.
    pub const FIRST: Self = Self(1);

    /// Constructs a sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ZeroSequence`] when `value` is zero;
    /// sequences are 1-based.
    pub const fn try_new(value: u64) -> Result<Self, ProtocolError> {
        if value == 0 {
            Err(ProtocolError::ZeroSequence)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the raw sequence value.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns the next sequence number without wraparound.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::SequenceOverflow`] at the representable
    /// maximum.
    pub const fn next(self) -> Result<Self, ProtocolError> {
        if self.0 == u64::MAX {
            Err(ProtocolError::SequenceOverflow { at: self.0 })
        } else {
            Ok(Self(self.0 + 1))
        }
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A versioned event envelope. No transport is attached; encoding is a
/// plain JSON contract so any local IPC transport can carry it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct EventEnvelope {
    /// The protocol version negotiated for this connection.
    pub protocol_version: ProtocolVersion,
    /// Identity of this event.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub event_id: EventId,
    /// The operation this event belongs to, when scoped to one.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub operation_id: Option<OperationId>,
    /// The thread this event belongs to, when scoped to one.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub thread_id: Option<ThreadId>,
    /// The turn this event belongs to, when scoped to one.
    #[cfg_attr(feature = "dto-export", ts(as = "Option<String>"))]
    pub turn_id: Option<TurnId>,
    /// The 1-based position of this event within its ordered stream.
    pub sequence: Sequence,
    /// When the event occurred. Supplied by the emitting side; fixtures
    /// use constants.
    #[cfg_attr(feature = "dto-export", ts(type = "number"))]
    pub occurred_at: UnixMillis,
    /// The event discriminator and payload.
    pub body: EventBody,
}

impl EventEnvelope {
    /// Decodes an event envelope from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the input is not
    /// a valid envelope, when a known event payload is malformed, or when
    /// an unknown event's diagnostic exceeds its byte cap.
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

    /// Builds a `turn.started` event envelope.
    #[must_use]
    pub fn turn_started(
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::TurnStarted),
        }
    }

    /// Builds a `message.delta` event envelope.
    #[must_use]
    pub fn message_delta(
        text: MessageText,
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::MessageDelta { text }),
        }
    }

    /// Builds a `permission.requested` event envelope.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn permission_requested(
        permission_kind: String,
        description: DiagnosticText,
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::PermissionRequested {
                permission_kind,
                description,
            }),
        }
    }

    /// Builds a `permission.decided` event envelope.
    #[must_use]
    pub fn permission_decided(
        decision: String,
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::PermissionDecided { decision }),
        }
    }

    /// Builds a `turn.completed` event envelope.
    #[must_use]
    pub fn turn_completed(
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::TurnCompleted),
        }
    }

    /// Builds a `turn.failed` event envelope.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn turn_failed(
        reason: DiagnosticText,
        delivery_state: String,
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::TurnFailed {
                reason,
                delivery_state,
            }),
        }
    }

    /// Builds a `turn.cancelled` event envelope.
    #[must_use]
    pub fn turn_cancelled(
        reason: Option<DiagnosticText>,
        event_id: EventId,
        operation_id: Option<OperationId>,
        thread_id: Option<ThreadId>,
        turn_id: Option<TurnId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id,
            turn_id,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::TurnCancelled { reason }),
        }
    }

    /// Builds a `runtime.status` event envelope.
    #[must_use]
    pub fn runtime_status(
        status: String,
        active_threads: u32,
        diagnostics: Option<DiagnosticText>,
        event_id: EventId,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id: None,
            thread_id: None,
            turn_id: None,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::RuntimeStatus {
                status,
                active_threads,
                diagnostics,
            }),
        }
    }

    /// Builds a `command.result` event envelope.
    #[must_use]
    pub fn command_result(
        operation_id: OperationId,
        success: bool,
        data: Option<BoundedPayload>,
        event_id: EventId,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id: Some(operation_id.clone()),
            thread_id: None,
            turn_id: None,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::CommandResult {
                operation_id,
                success,
                data,
            }),
        }
    }

    /// Builds a `command.error` event envelope.
    #[must_use]
    pub fn command_error(
        operation_id: OperationId,
        code: String,
        message: DiagnosticText,
        event_id: EventId,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id: Some(operation_id.clone()),
            thread_id: None,
            turn_id: None,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::CommandError {
                operation_id,
                code,
                message,
            }),
        }
    }

    /// Builds a `stream.gap` event envelope.
    #[must_use]
    pub fn stream_gap(
        from: Sequence,
        event_id: EventId,
        operation_id: Option<OperationId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id: None,
            turn_id: None,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::StreamGap { from }),
        }
    }

    /// Builds a `stream.replayed` event envelope.
    #[must_use]
    pub fn stream_replayed(
        from: Sequence,
        through: Sequence,
        event_id: EventId,
        operation_id: Option<OperationId>,
        sequence: Sequence,
        occurred_at: UnixMillis,
    ) -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            event_id,
            operation_id,
            thread_id: None,
            turn_id: None,
            sequence,
            occurred_at,
            body: EventBody::Known(KnownEvent::StreamReplayed { from, through }),
        }
    }

    /// Validates the envelope against `limits` and the locally supported
    /// protocol versions.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnsupportedProtocolVersion`] when the
    /// envelope's version is outside [`SUPPORTED_PROTOCOL_VERSIONS`] and
    /// [`ProtocolError::TextTooLarge`] when an unknown-event diagnostic
    /// exceeds the limit.
    pub fn validate(&self, limits: &EnvelopeLimits) -> Result<(), ProtocolError> {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(self.protocol_version) {
            return Err(ProtocolError::UnsupportedProtocolVersion {
                requested: self.protocol_version.as_u32(),
                supported: SUPPORTED_PROTOCOL_VERSIONS,
            });
        }
        if let EventBody::Unknown { diagnostic, .. } = &self.body
            && diagnostic.len() > limits.diagnostic_bytes
        {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: diagnostic.len(),
                limit_bytes: limits.diagnostic_bytes,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known_envelope() -> EventEnvelope {
        EventEnvelope {
            protocol_version: ProtocolVersion::V1,
            event_id: "evt_fixture000000006".parse().unwrap(),
            operation_id: Some("op_fixture000000005".parse().unwrap()),
            thread_id: Some("thr_fixture000000001".parse().unwrap()),
            turn_id: Some("trn_fixture000000002".parse().unwrap()),
            sequence: Sequence::FIRST,
            occurred_at: UnixMillis::from_millis(1_700_000_000_000),
            body: EventBody::Known(KnownEvent::MessageDelta {
                text: MessageText::try_from("hello").unwrap(),
            }),
        }
    }

    #[test]
    fn sequence_is_one_based_and_refuses_overflow() {
        assert!(matches!(
            Sequence::try_new(0),
            Err(ProtocolError::ZeroSequence)
        ));
        assert_eq!(Sequence::FIRST.as_u64(), 1);
        assert_eq!(Sequence::try_new(41).unwrap().next().unwrap().as_u64(), 42);
        assert!(matches!(
            Sequence::try_new(u64::MAX).unwrap().next(),
            Err(ProtocolError::SequenceOverflow { at: u64::MAX })
        ));
    }

    #[test]
    fn sequence_decoding_rejects_zero_and_roundtrips_numbers() {
        let valid: Sequence = serde_json::from_value(serde_json::json!(7)).unwrap();
        assert_eq!(valid.as_u64(), 7);
        assert_eq!(serde_json::to_value(valid).unwrap(), serde_json::json!(7));
        assert!(serde_json::from_value::<Sequence>(serde_json::json!(0)).is_err());
    }

    #[test]
    fn known_events_roundtrip_through_json() {
        let envelope = known_envelope();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        let json = envelope.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(decoded.body.kind_name(), "message.delta");
    }

    #[test]
    fn permission_events_roundtrip_through_json() {
        let req = EventEnvelope::permission_requested(
            "execute".to_string(),
            DiagnosticText::try_from("Execute rm -rf /tmp/test").unwrap(),
            "evt_fixture000000020".parse().unwrap(),
            Some("op_fixture000000005".parse().unwrap()),
            Some("thr_fixture000000001".parse().unwrap()),
            Some("trn_fixture000000002".parse().unwrap()),
            Sequence::FIRST,
            UnixMillis::from_millis(1_700_000_000_000),
        );
        let json = req.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, req);

        let dec = EventEnvelope::permission_decided(
            "approved".to_string(),
            "evt_fixture000000021".parse().unwrap(),
            Some("op_fixture000000005".parse().unwrap()),
            Some("thr_fixture000000001".parse().unwrap()),
            Some("trn_fixture000000002".parse().unwrap()),
            Sequence::try_new(2).unwrap(),
            UnixMillis::from_millis(1_700_000_000_001),
        );
        let json = dec.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, dec);
    }

    #[test]
    fn turn_lifecycle_events_roundtrip() {
        let completed = EventEnvelope::turn_completed(
            "evt_fixture000000022".parse().unwrap(),
            Some("op_fixture000000005".parse().unwrap()),
            Some("thr_fixture000000001".parse().unwrap()),
            Some("trn_fixture000000002".parse().unwrap()),
            Sequence::try_new(3).unwrap(),
            UnixMillis::from_millis(1_700_000_000_002),
        );
        let json = completed.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, completed);

        let failed = EventEnvelope::turn_failed(
            DiagnosticText::try_from("Process exited with 1").unwrap(),
            "confirmed".to_string(),
            "evt_fixture000000023".parse().unwrap(),
            Some("op_fixture000000005".parse().unwrap()),
            Some("thr_fixture000000001".parse().unwrap()),
            Some("trn_fixture000000002".parse().unwrap()),
            Sequence::try_new(4).unwrap(),
            UnixMillis::from_millis(1_700_000_000_003),
        );
        let json = failed.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, failed);

        let cancelled = EventEnvelope::turn_cancelled(
            Some(DiagnosticText::try_from("User cancelled").unwrap()),
            "evt_fixture000000024".parse().unwrap(),
            Some("op_fixture000000005".parse().unwrap()),
            Some("thr_fixture000000001".parse().unwrap()),
            Some("trn_fixture000000002".parse().unwrap()),
            Sequence::try_new(5).unwrap(),
            UnixMillis::from_millis(1_700_000_000_004),
        );
        let json = cancelled.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, cancelled);
    }

    #[test]
    fn command_result_and_error_events_roundtrip() {
        let result = EventEnvelope::command_result(
            "op_fixture000000005".parse().unwrap(),
            true,
            Some(
                BoundedPayload::new(
                    serde_json::json!({"created": true}),
                    EnvelopeLimits::default().payload_bytes,
                )
                .unwrap(),
            ),
            "evt_fixture000000025".parse().unwrap(),
            Sequence::try_new(6).unwrap(),
            UnixMillis::from_millis(1_700_000_000_005),
        );
        let json = result.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, result);

        let error = EventEnvelope::command_error(
            "op_fixture000000005".parse().unwrap(),
            "NOT_FOUND".to_string(),
            DiagnosticText::try_from("Thread not found").unwrap(),
            "evt_fixture000000026".parse().unwrap(),
            Sequence::try_new(7).unwrap(),
            UnixMillis::from_millis(1_700_000_000_006),
        );
        let json = error.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, error);
    }

    #[test]
    fn unknown_future_events_are_preserved_not_rejected() {
        let json = r#"{"protocol_version":1,"event_id":"evt_fixture000000007","operation_id":null,"thread_id":null,"turn_id":null,"sequence":2,"occurred_at":1700000000002,"body":{"kind":"usage.updated","input_tokens":42}}"#;
        let envelope = EventEnvelope::from_json(json).unwrap();
        envelope.validate(&EnvelopeLimits::default()).unwrap();
        match &envelope.body {
            EventBody::Unknown {
                provider_kind,
                diagnostic,
            } => {
                assert_eq!(provider_kind, "usage.updated");
                assert!(diagnostic.as_str().contains(r#""input_tokens":42"#));
            }
            EventBody::Known(_) => panic!("expected unknown event body"),
        }
        let re_encoded = envelope.to_json().unwrap();
        let decoded_again = EventEnvelope::from_json(&re_encoded).unwrap();
        assert_eq!(decoded_again, envelope);
    }

    #[test]
    fn malformed_event_kind_names_fail_explicitly() {
        let empty_kind = r#"{"protocol_version":1,"event_id":"evt_fixture000000007","operation_id":null,"thread_id":null,"turn_id":null,"sequence":2,"occurred_at":1700000000002,"body":{"kind":""}}"#;
        assert!(EventEnvelope::from_json(empty_kind).is_err());

        let long_kind = format!(
            r#"{{"protocol_version":1,"event_id":"evt_fixture000000007","operation_id":null,"thread_id":null,"turn_id":null,"sequence":2,"occurred_at":1700000000002,"body":{{"kind":"{}"}}}}"#,
            "a".repeat(MAX_KIND_LEN + 1)
        );
        assert!(EventEnvelope::from_json(&long_kind).is_err());
    }

    #[cfg(feature = "dto-export")]
    #[test]
    fn event_body_manual_union_generates_boolean_without_bool_or_trailing_whitespace() {
        use ts_rs::TS;
        let cfg = ts_rs::Config::default();
        let event_body_decl = EventBody::decl(&cfg);
        let known_event_decl = KnownEvent::decl(&cfg);

        let has_bool_token = |s: &str| {
            s.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|token| token == "bool")
        };

        // Must generate boolean for success, not bool.
        assert!(
            event_body_decl.contains("success: boolean"),
            "EventBody decl must contain `success: boolean`, got: {event_body_decl}"
        );
        assert!(
            !has_bool_token(&event_body_decl),
            "EventBody decl must not contain `bool` token: {event_body_decl}"
        );

        assert!(
            known_event_decl.contains("success: boolean"),
            "KnownEvent decl must contain `success: boolean`, got: {known_event_decl}"
        );
        assert!(
            !has_bool_token(&known_event_decl),
            "KnownEvent decl must not contain `bool` token: {known_event_decl}"
        );

        // Verify all known event kinds are present in the manual union.
        let required_kinds = [
            r#"kind: "turn.started""#,
            r#"kind: "message.delta""#,
            r#"kind: "permission.requested""#,
            r#"kind: "permission.decided""#,
            r#"kind: "turn.completed""#,
            r#"kind: "turn.failed""#,
            r#"kind: "turn.cancelled""#,
            r#"kind: "runtime.status""#,
            r#"kind: "command.result""#,
            r#"kind: "command.error""#,
            r#"kind: "stream.gap""#,
            r#"kind: "stream.replayed""#,
            r"kind: string; diagnostic: string",
        ];
        for kind in &required_kinds {
            assert!(
                event_body_decl.contains(kind),
                "EventBody union missing variant: {kind}"
            );
        }
    }
}
