//! Bounded payload and text values.
//!
//! Every envelope field that can carry untrusted or provider-derived data
//! is bounded at construction: text by a type-level byte cap, JSON payloads
//! by an explicit [`EnvelopeLimits`] check. Oversized input is rejected
//! with a typed error instead of being accepted and ballooning memory
//! (ADR 0004).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::error::ProtocolError;

/// Text bounded to a compile-time byte cap.
///
/// The cap is enforced on construction and on deserialization, so an
/// over-limit value cannot enter the system through decoded data.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoundedText<const MAX_BYTES: usize>(String);

impl<const MAX_BYTES: usize> BoundedText<MAX_BYTES> {
    /// The compile-time byte cap of this text kind.
    #[must_use]
    pub const fn capacity() -> usize {
        MAX_BYTES
    }

    /// Returns the text as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the text length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the text is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<const MAX_BYTES: usize> TryFrom<&str> for BoundedText<MAX_BYTES> {
    type Error = ProtocolError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() > MAX_BYTES {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: value.len(),
                limit_bytes: MAX_BYTES,
            });
        }
        Ok(Self(value.to_owned()))
    }
}

impl<const MAX_BYTES: usize> TryFrom<String> for BoundedText<MAX_BYTES> {
    type Error = ProtocolError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > MAX_BYTES {
            return Err(ProtocolError::TextTooLarge {
                size_bytes: value.len(),
                limit_bytes: MAX_BYTES,
            });
        }
        Ok(Self(value))
    }
}

impl<const MAX_BYTES: usize> From<BoundedText<MAX_BYTES>> for String {
    fn from(value: BoundedText<MAX_BYTES>) -> Self {
        value.0
    }
}

impl<const MAX_BYTES: usize> std::fmt::Display for BoundedText<MAX_BYTES> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const MAX_BYTES: usize> Serialize for BoundedText<MAX_BYTES> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const MAX_BYTES: usize> Deserialize<'de> for BoundedText<MAX_BYTES> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::try_from(raw).map_err(D::Error::custom)
    }
}

/// Diagnostic text capped at 4 KiB. Unknown provider event payloads are
/// preserved only as this bounded representation.
pub type DiagnosticText = BoundedText<4096>;

/// Message body text capped at 64 KiB.
pub type MessageText = BoundedText<{ 64 * 1024 }>;

/// A JSON payload bounded by an explicit, runtime-provided limit.
///
/// The payload is stored as a [`serde_json::Value`]; map keys stay sorted
/// because the `preserve_order` feature is not enabled, which keeps
/// serialization deterministic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        type = "unknown"
    )
)]
pub struct BoundedPayload(Value);

impl BoundedPayload {
    /// Accepts a payload after checking its encoded size against
    /// `limit_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the encoded payload
    /// exceeds `limit_bytes`.
    pub fn new(value: Value, limit_bytes: usize) -> Result<Self, ProtocolError> {
        let payload = Self(value);
        payload.ensure_within(limit_bytes)?;
        Ok(payload)
    }

    /// Returns the stored JSON value.
    #[must_use]
    pub fn value(&self) -> &Value {
        &self.0
    }

    /// Returns the encoded size of this payload in bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the payload cannot
    /// be encoded.
    pub fn encoded_len(&self) -> Result<usize, ProtocolError> {
        Ok(serde_json::to_vec(&self.0)?.len())
    }

    /// Validates the encoded size against `limit_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the encoded payload
    /// exceeds `limit_bytes`, and [`ProtocolError::MalformedEnvelope`] when
    /// the payload cannot be encoded.
    pub fn ensure_within(&self, limit_bytes: usize) -> Result<(), ProtocolError> {
        let size_bytes = self.encoded_len()?;
        if size_bytes > limit_bytes {
            return Err(ProtocolError::PayloadTooLarge {
                size_bytes,
                limit_bytes,
            });
        }
        Ok(())
    }
}

/// Resource limits applied when validating an envelope.
///
/// The defaults are P0.1 baseline values; the transport selected in P0.2
/// may negotiate different values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeLimits {
    /// Maximum encoded payload size in bytes.
    pub payload_bytes: usize,
    /// Maximum unknown-event diagnostic size in bytes.
    pub diagnostic_bytes: usize,
}

impl Default for EnvelopeLimits {
    fn default() -> Self {
        Self {
            payload_bytes: 64 * 1024,
            diagnostic_bytes: DiagnosticText::capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_rejects_oversized_values() {
        let oversized = "a".repeat(DiagnosticText::capacity() + 1);
        assert!(matches!(
            DiagnosticText::try_from(oversized.as_str()),
            Err(ProtocolError::TextTooLarge { size_bytes, limit_bytes })
                if size_bytes == DiagnosticText::capacity() + 1
                    && limit_bytes == DiagnosticText::capacity()
        ));
        assert_eq!(DiagnosticText::try_from("ok").unwrap().as_str(), "ok");
    }

    #[test]
    fn bounded_text_rejects_oversized_values_on_deserialize() {
        let oversized = "b".repeat(MessageText::capacity() + 1);
        let bad = serde_json::from_value::<MessageText>(Value::String(oversized));
        assert!(bad.is_err());
    }

    #[test]
    fn bounded_payload_enforces_its_limit() {
        let small = BoundedPayload::new(serde_json::json!({"ping": 1}), 32).unwrap();
        assert_eq!(small.encoded_len().unwrap(), 10);
        small.ensure_within(32).unwrap();

        let error =
            BoundedPayload::new(serde_json::json!({"padding": "0123456789"}), 16).unwrap_err();
        assert!(matches!(
            error,
            ProtocolError::PayloadTooLarge {
                size_bytes: 24,
                limit_bytes: 16
            }
        ));
    }
}
