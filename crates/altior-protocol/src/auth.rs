//! Per-launch capability token carried by the Desktop handshake.
//!
//! Core generates one token per launch from OS entropy and publishes it in a
//! user-readable token file; Desktop presents it in [`DesktopHello`] and the
//! IPC session layer rejects unauthenticated connections before any
//! negotiation output (`docs/SECURITY.md`, ADR 0006). The token is opaque to
//! this layer: 32 to 128 lowercase hex characters. Generation is an
//! infrastructure concern — this crate only validates and carries tokens.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

/// Minimum token length in hex characters (128 bits).
const MIN_TOKEN_LEN: usize = 32;
/// Maximum token length in hex characters.
const MAX_TOKEN_LEN: usize = 128;

/// An opaque per-launch capability token (ADR 0006).
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        type = "string"
    )
)]
pub struct LaunchToken(String);

impl fmt::Debug for LaunchToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LaunchToken").field(&"[REDACTED]").finish()
    }
}

impl LaunchToken {
    /// Validates and constructs a launch token.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::LaunchTokenLength`] outside 32–128 chars and
    /// [`ProtocolError::InvalidLaunchTokenCharacter`] for anything but
    /// lowercase hex.
    pub fn try_new(value: &str) -> Result<Self, ProtocolError> {
        let length = value.chars().count();
        if !(MIN_TOKEN_LEN..=MAX_TOKEN_LEN).contains(&length) {
            return Err(ProtocolError::LaunchTokenLength { length });
        }
        for (position, character) in value.char_indices() {
            if !character.is_ascii_digit() && !('a'..='f').contains(&character) {
                return Err(ProtocolError::InvalidLaunchTokenCharacter {
                    character,
                    position,
                });
            }
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the canonical hex form of the token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LaunchToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for LaunchToken {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_new(s)
    }
}

impl TryFrom<String> for LaunchToken {
    type Error = ProtocolError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_new(&value)
    }
}

impl From<LaunchToken> for String {
    fn from(value: LaunchToken) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f0";

    #[test]
    fn accepts_hex_tokens_within_bounds() {
        let token = LaunchToken::try_new(TOKEN).unwrap();
        assert_eq!(token.as_str(), TOKEN);
        assert_eq!(token.to_string(), TOKEN);
    }

    #[test]
    fn rejects_wrong_lengths() {
        assert!(matches!(
            LaunchToken::try_new(&TOKEN[1..]),
            Err(ProtocolError::LaunchTokenLength { length: 31 })
        ));
        let long = "0".repeat(MAX_TOKEN_LEN + 1);
        assert!(matches!(
            LaunchToken::try_new(&long),
            Err(ProtocolError::LaunchTokenLength { length: 129 })
        ));
    }

    #[test]
    fn rejects_non_hex_characters() {
        assert!(matches!(
            LaunchToken::try_new("0f1e2d3c4b5a69788796a5b4c3d2e1fG"),
            Err(ProtocolError::InvalidLaunchTokenCharacter {
                character: 'G',
                position: 31
            })
        ));
    }

    #[test]
    fn serde_roundtrips_and_rejects_invalid_values() {
        let token = LaunchToken::try_new(TOKEN).unwrap();
        let encoded = serde_json::to_value(&token).unwrap();
        assert_eq!(encoded, serde_json::json!(TOKEN));
        let decoded: LaunchToken = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, token);
        assert!(serde_json::from_value::<LaunchToken>(serde_json::json!("short")).is_err());
    }

    #[test]
    fn debug_redacts_launch_token_zero_plaintext() {
        let token = LaunchToken::try_new(TOKEN).unwrap();
        let debug_str = format!("{token:?}");
        assert_eq!(debug_str, "LaunchToken(\"[REDACTED]\")");
        assert!(!debug_str.contains(TOKEN));
        let alt_debug_str = format!("{token:#?}");
        assert!(!alt_debug_str.contains(TOKEN));
        assert!(alt_debug_str.contains("[REDACTED]"));
    }
}
