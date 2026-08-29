//! Core's post-handshake greeting: instance identity and retained window.
//!
//! After authentication and version negotiation succeed, Core sends exactly
//! one [`CoreGreeting`] before any events. It names the launch
//! (`instance_id`) and the window of past events still retained in memory.
//! Desktop compares the instance id against its last session: a different id
//! means Core restarted and every stored sequence is stale, so Desktop must
//! re-derive state from a snapshot instead of assuming continuity
//! (ADR 0006). `Sequence` values are meaningful only within one instance.

use serde::{Deserialize, Serialize};

use altior_domain::CoreInstanceId;

use crate::error::ProtocolError;
use crate::event::Sequence;
use crate::version::{ProductVersion, ProtocolVersion, SUPPORTED_PROTOCOL_VERSIONS};

/// The inclusive range of past events Core still holds in memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct RetainedWindow {
    /// Oldest retained sequence, inclusive.
    pub from: Sequence,
    /// Newest retained sequence, inclusive.
    pub through: Sequence,
}

impl RetainedWindow {
    /// Validates that the window is well ordered.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidRetainedWindow`] when `from` is
    /// greater than `through`.
    pub const fn validate(&self) -> Result<(), ProtocolError> {
        if self.from.as_u64() > self.through.as_u64() {
            Err(ProtocolError::InvalidRetainedWindow {
                from: self.from.as_u64(),
                through: self.through.as_u64(),
            })
        } else {
            Ok(())
        }
    }
}

/// Core's first message after a successful handshake (ADR 0006).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CoreGreeting {
    /// The protocol version negotiated for this connection.
    pub protocol_version: ProtocolVersion,
    /// Identity of this Core launch. Differs across restarts.
    #[cfg_attr(feature = "dto-export", ts(type = "string"))]
    pub instance_id: CoreInstanceId,
    /// Core's product build version, for diagnostics only.
    pub core_version: ProductVersion,
    /// The retained event window, or `None` on a fresh Core with no events.
    pub retained: Option<RetainedWindow>,
}

impl CoreGreeting {
    /// Decodes a greeting from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the input is not a
    /// valid greeting.
    pub fn from_json(input: &str) -> Result<Self, ProtocolError> {
        Ok(serde_json::from_str(input)?)
    }

    /// Encodes the greeting as deterministic JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedEnvelope`] when the greeting cannot
    /// be encoded.
    pub fn to_json(&self) -> Result<String, ProtocolError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Validates the greeting against the locally supported protocol
    /// versions and the retained-window ordering.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnsupportedProtocolVersion`] outside the
    /// supported range and [`ProtocolError::InvalidRetainedWindow`] for a
    /// mis-ordered window.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(self.protocol_version) {
            return Err(ProtocolError::UnsupportedProtocolVersion {
                requested: self.protocol_version.as_u32(),
                supported: SUPPORTED_PROTOCOL_VERSIONS,
            });
        }
        if let Some(retained) = &self.retained {
            retained.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greeting() -> CoreGreeting {
        CoreGreeting {
            protocol_version: ProtocolVersion::V1,
            instance_id: "cor_fixture000000009".parse().unwrap(),
            core_version: ProductVersion::new(0, 1, 0),
            retained: Some(RetainedWindow {
                from: Sequence::try_new(4).unwrap(),
                through: Sequence::try_new(9).unwrap(),
            }),
        }
    }

    #[test]
    fn roundtrips_a_greeting() {
        let original = greeting();
        let json = original.to_json().unwrap();
        let decoded = CoreGreeting::from_json(&json).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.to_json().unwrap(), json);
    }

    #[test]
    fn fresh_cores_carry_no_retained_window() {
        let fresh = CoreGreeting {
            retained: None,
            ..greeting()
        };
        fresh.validate().unwrap();
        assert!(fresh.to_json().unwrap().contains("\"retained\":null"));
    }

    #[test]
    fn rejects_misordered_windows_and_bad_versions() {
        let mut misordered = greeting();
        misordered.retained = Some(RetainedWindow {
            from: Sequence::try_new(9).unwrap(),
            through: Sequence::try_new(4).unwrap(),
        });
        assert!(matches!(
            misordered.validate(),
            Err(ProtocolError::InvalidRetainedWindow {
                from: 9,
                through: 4
            })
        ));

        let mut unsupported = greeting();
        unsupported.protocol_version = ProtocolVersion::try_new(99).unwrap();
        assert!(matches!(
            unsupported.validate(),
            Err(ProtocolError::UnsupportedProtocolVersion { requested: 99, .. })
        ));
    }
}
