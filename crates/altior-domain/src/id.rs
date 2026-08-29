//! Stable, kind-prefixed identifiers for first-class Altior entities.
//!
//! Every identifier is a distinct newtype over a canonical
//! `<prefix>_<body>` string, where `body` is 16 to 64 characters from
//! `[0-9a-z]`. The domain parses, validates, displays, and serializes
//! identifiers; it never generates them. Generation needs a randomness or
//! UUID source, which is an infrastructure concern owned by `altior-core`.
//! Tests and fixtures build identifiers by parsing fixed synthetic literals,
//! which keeps them deterministic. See ADR 0004.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Minimum number of characters in an identifier body after the prefix.
const MIN_BODY_LEN: usize = 16;
/// Maximum number of characters in an identifier body after the prefix.
const MAX_BODY_LEN: usize = 64;

/// Typed validation failure for a malformed identifier string.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum IdParseError {
    /// The candidate string is empty.
    Empty,
    /// The candidate string has no `_` separator after its prefix.
    MissingSeparator,
    /// The candidate string carries a different type's prefix.
    UnexpectedPrefix {
        /// The prefix this identifier type requires, including `_`.
        expected: &'static str,
        /// The prefix segment that was found, including `_`.
        found: String,
    },
    /// The body contains a character outside `[0-9a-z]`.
    InvalidCharacter {
        /// The offending character.
        character: char,
        /// The byte offset of `character` within the full string.
        position: usize,
    },
    /// The body is shorter than the minimum length.
    BodyTooShort {
        /// The body length that was found.
        length: usize,
    },
    /// The body is longer than the maximum length.
    BodyTooLong {
        /// The body length that was found.
        length: usize,
    },
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "identifier is empty"),
            Self::MissingSeparator => write!(f, "identifier has no prefix separator"),
            Self::UnexpectedPrefix { expected, found } => {
                write!(f, "identifier prefix {found} does not match {expected}")
            }
            Self::InvalidCharacter {
                character,
                position,
            } => {
                write!(
                    f,
                    "identifier has invalid character {character:?} at byte {position}"
                )
            }
            Self::BodyTooShort { length } => {
                write!(
                    f,
                    "identifier body has {length} chars, fewer than {MIN_BODY_LEN}"
                )
            }
            Self::BodyTooLong { length } => {
                write!(
                    f,
                    "identifier body has {length} chars, more than {MAX_BODY_LEN}"
                )
            }
        }
    }
}

impl std::error::Error for IdParseError {}

/// Validates a candidate identifier against one type's prefix.
fn validate(candidate: &str, prefix: &'static str) -> Result<(), IdParseError> {
    if candidate.is_empty() {
        return Err(IdParseError::Empty);
    }
    let (head, body) = candidate
        .split_once('_')
        .ok_or(IdParseError::MissingSeparator)?;
    let found = format!("{head}_");
    if found != prefix {
        return Err(IdParseError::UnexpectedPrefix {
            expected: prefix,
            found,
        });
    }
    for (offset, character) in body.char_indices() {
        if !character.is_ascii_lowercase() && !character.is_ascii_digit() {
            return Err(IdParseError::InvalidCharacter {
                character,
                position: found.len() + offset,
            });
        }
    }
    let length = body.chars().count();
    if length < MIN_BODY_LEN {
        return Err(IdParseError::BodyTooShort { length });
    }
    if length > MAX_BODY_LEN {
        return Err(IdParseError::BodyTooLong { length });
    }
    Ok(())
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The canonical prefix (including `_`) used by this identifier kind.
            #[must_use]
            pub const fn prefix() -> &'static str {
                $prefix
            }

            /// Returns the canonical string form of this identifier.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                validate(s, $prefix)?;
                Ok(Self(s.to_owned()))
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdParseError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::from_str(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::from_str(&value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

define_id!(
    /// Identity of an Altior-owned agent profile (ADR 0002).
    AgentProfileId,
    "agp_"
);
define_id!(
    /// Identity of a device-local harness launch binding (ADR 0002).
    HarnessBindingId,
    "hsb_"
);
define_id!(
    /// Identity of an Altior-owned conversation thread.
    ThreadId,
    "thr_"
);
define_id!(
    /// Identity of one delivery-safe unit of work inside a thread.
    TurnId,
    "trn_"
);
define_id!(
    /// Identity of a parent/child coordination operation.
    OperationId,
    "op_"
);
define_id!(
    /// Identity of one normalized domain event.
    EventId,
    "evt_"
);

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "thr_fixture000000001";
    const TURN: &str = "trn_fixture000000002";

    #[test]
    fn parses_and_displays_each_identifier_kind() {
        assert_eq!(
            AgentProfileId::from_str("agp_fixture000000003")
                .unwrap()
                .to_string(),
            "agp_fixture000000003"
        );
        assert_eq!(
            HarnessBindingId::from_str("hsb_fixture000000004")
                .unwrap()
                .to_string(),
            "hsb_fixture000000004"
        );
        assert_eq!(ThreadId::from_str(THREAD).unwrap().to_string(), THREAD);
        assert_eq!(TurnId::from_str(TURN).unwrap().to_string(), TURN);
        assert_eq!(
            OperationId::from_str("op_fixture000000005")
                .unwrap()
                .to_string(),
            "op_fixture000000005"
        );
        assert_eq!(
            EventId::from_str("evt_fixture000000006")
                .unwrap()
                .to_string(),
            "evt_fixture000000006"
        );
    }

    #[test]
    fn rejects_empty_candidate() {
        assert_eq!(ThreadId::from_str(""), Err(IdParseError::Empty));
    }

    #[test]
    fn rejects_missing_separator() {
        assert_eq!(
            ThreadId::from_str("thrfixture000000001"),
            Err(IdParseError::MissingSeparator)
        );
    }

    #[test]
    fn rejects_foreign_prefix() {
        // A turn id parsed as a thread id must fail: kinds cannot be
        // interchanged even at the string boundary.
        assert_eq!(
            ThreadId::from_str(TURN),
            Err(IdParseError::UnexpectedPrefix {
                expected: "thr_",
                found: "trn_".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_invalid_characters() {
        assert!(matches!(
            ThreadId::from_str("thr_Fixture000000001"),
            Err(IdParseError::InvalidCharacter { character: 'F', .. })
        ));
        assert!(matches!(
            ThreadId::from_str("thr_fixture-00000001"),
            Err(IdParseError::InvalidCharacter { character: '-', .. })
        ));
    }

    #[test]
    fn rejects_out_of_bounds_body_length() {
        let short = format!("thr_{}", "0".repeat(15));
        assert_eq!(
            ThreadId::from_str(&short),
            Err(IdParseError::BodyTooShort { length: 15 })
        );
        let long = format!("thr_{}", "0".repeat(65));
        assert_eq!(
            ThreadId::from_str(&long),
            Err(IdParseError::BodyTooLong { length: 65 })
        );
    }

    #[test]
    fn serde_roundtrips_and_rejects_invalid_values() {
        let id = ThreadId::from_str(THREAD).unwrap();
        let encoded = serde_json::to_value(&id).unwrap();
        assert_eq!(encoded, serde_json::json!(THREAD));
        let decoded: ThreadId = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, id);

        let bad = serde_json::from_value::<ThreadId>(serde_json::json!(TURN));
        assert!(bad.is_err());
    }
}
