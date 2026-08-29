//! Versioned event envelopes sent from Core to Desktop.
//!
//! The body carries a small closed set of known normalized events plus one
//! `Unknown` variant: an unrecognized wire event deserializes into
//! `Unknown` with the provider kind name and a **bounded diagnostic**
//! (the raw event object, capped), so future provider events never crash
//! the process and never enter stable domain records as structured data
//! (ADR 0004 and `docs/HARNESSES.md`).

use std::fmt;

use serde::de::{Deserializer, Error as _};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use altior_domain::{EventId, OperationId, ThreadId, TurnId, UnixMillis};

use crate::bounded::{DiagnosticText, EnvelopeLimits, MessageText};
use crate::error::ProtocolError;
use crate::version::{ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// Maximum length of an event kind name in bytes.
const MAX_KIND_LEN: usize = 64;

/// The known normalized event kinds defined by protocol version 1.
///
/// This is the smallest sample of the documented normalized stream needed
/// by envelope fixtures; the full taxonomy lands with the P1 domain
/// runtime.
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
    /// A turn completed (`turn.completed`).
    #[serde(rename = "turn.completed")]
    #[cfg_attr(feature = "dto-export", ts(rename = "turn.completed"))]
    TurnCompleted,
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
        type = r#"{ kind: "turn.started" } | { kind: "message.delta"; text: string } | { kind: "turn.completed" } | { kind: string; diagnostic: string }"#
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
            Self::Known(KnownEvent::TurnCompleted) => "turn.completed",
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
            "message.delta" => {
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
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
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
/// plain JSON contract so any P0.2 transport can carry it.
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
        if let EventBody::Unknown { diagnostic, .. } = &self.body {
            if diagnostic.len() > limits.diagnostic_bytes {
                return Err(ProtocolError::TextTooLarge {
                    size_bytes: diagnostic.len(),
                    limit_bytes: limits.diagnostic_bytes,
                });
            }
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
    fn roundtrips_a_known_event_envelope() {
        let envelope = known_envelope();
        let json = envelope.to_json().unwrap();
        let decoded = EventEnvelope::from_json(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn preserves_unknown_future_events_as_bounded_diagnostics() {
        let json = r#"{
            "protocol_version": 1,
            "event_id": "evt_fixture000000007",
            "operation_id": null,
            "thread_id": null,
            "turn_id": null,
            "sequence": 2,
            "occurred_at": 1700000000001,
            "body": {
                "kind": "usage.updated",
                "input_tokens": 42,
                "note": "future provider field"
            }
        }"#;
        let envelope = EventEnvelope::from_json(json).unwrap();
        let EventBody::Unknown {
            provider_kind,
            diagnostic,
        } = &envelope.body
        else {
            panic!("expected an unknown event body");
        };
        assert_eq!(provider_kind, "usage.updated");
        assert!(diagnostic.as_str().contains("input_tokens"));

        // The preserved form is re-encodable and stable: encode/decode
        // again and compare.
        let re_encoded = envelope.to_json().unwrap();
        let re_decoded = EventEnvelope::from_json(&re_encoded).unwrap();
        assert_eq!(re_decoded, envelope);
        assert_eq!(envelope.to_json().unwrap(), re_encoded);
    }

    #[test]
    fn rejects_oversized_unknown_event_diagnostics() {
        let padding = "\"".to_owned() + &"x".repeat(DiagnosticText::capacity()) + "\"";
        let json = format!(
            r#"{{"protocol_version":1,"event_id":"evt_fixture000000008","operation_id":null,"thread_id":null,"turn_id":null,"sequence":1,"occurred_at":0,"body":{{"kind":"usage.updated","pad":{padding}}}}}"#
        );
        assert!(EventEnvelope::from_json(&json).is_err());
    }

    #[test]
    fn rejects_completely_invalid_envelopes() {
        assert!(EventEnvelope::from_json("").is_err());
        assert!(EventEnvelope::from_json("not json").is_err());
        assert!(EventEnvelope::from_json("{}").is_err());
        // A missing kind discriminator is invalid.
        assert!(EventEnvelope::from_json(
            r#"{"protocol_version":1,"event_id":"evt_fixture000000009","operation_id":null,"thread_id":null,"turn_id":null,"sequence":1,"occurred_at":0,"body":{"text":"hi"}}"#
        )
        .is_err());
    }

    #[test]
    fn validate_rejects_unsupported_protocol_versions() {
        let mut envelope = known_envelope();
        envelope.protocol_version = ProtocolVersion::try_new(99).unwrap();
        let error = envelope
            .validate(&crate::bounded::EnvelopeLimits::default())
            .unwrap_err();
        assert!(matches!(
            error,
            ProtocolError::UnsupportedProtocolVersion { requested: 99, .. }
        ));
        known_envelope()
            .validate(&crate::bounded::EnvelopeLimits::default())
            .unwrap();
    }
}
