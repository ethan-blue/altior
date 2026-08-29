//! Explicitly negotiated capability contracts.
//!
//! A capability is a canonical identifier string such as `event.streaming`.
//! Both handshake sides declare every capability they know about as
//! `supported` or `unsupported`. A capability is negotiated only when both
//! sides explicitly declare it supported; capabilities claimed by one side
//! only are recorded for diagnostics. Capability ids are data — future ids
//! unknown to one side ride along and classify like any other id. They are
//! never inferred from an agent or application version string (ADR 0004).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

/// Maximum length of a capability id in bytes.
const MAX_ID_LEN: usize = 64;

/// A canonical capability identifier, e.g. `event.streaming`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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
pub struct CapabilityId(String);

impl CapabilityId {
    /// Returns the canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CapabilityId {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let reject = || ProtocolError::MalformedCapabilityId {
            value: s.to_owned(),
        };
        if s.is_empty() || s.len() > MAX_ID_LEN {
            return Err(reject());
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        {
            return Err(reject());
        }
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<&str> for CapabilityId {
    type Error = ProtocolError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_str(value)
    }
}

impl TryFrom<String> for CapabilityId {
    type Error = ProtocolError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

impl From<CapabilityId> for String {
    fn from(value: CapabilityId) -> Self {
        value.0
    }
}

/// One endpoint's declared support state for a capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        rename_all = "lowercase"
    )
)]
pub enum CapabilitySupport {
    /// The endpoint implements the capability.
    Supported,
    /// The endpoint knows the capability and does not implement it.
    Unsupported,
}

/// A deterministic set of declared capabilities keyed by canonical id.
///
/// The map is ordered, so serialization and iteration are deterministic.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(
        export,
        export_to = "../../../apps/desktop/src/ipc/dto/",
        as = "std::collections::BTreeMap<String, CapabilitySupport>"
    )
)]
pub struct CapabilitySet(BTreeMap<CapabilityId, CapabilitySupport>);

impl CapabilitySet {
    /// Creates an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// Declares (or redeclares) one capability.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedCapabilityId`] when `id` violates
    /// the canonical charset or size.
    pub fn declare(&mut self, id: &str, support: CapabilitySupport) -> Result<(), ProtocolError> {
        let id = CapabilityId::from_str(id)?;
        self.0.insert(id, support);
        Ok(())
    }

    /// Declares a capability whose id was already validated by
    /// construction.
    pub(crate) fn declare_validated(&mut self, id: CapabilityId, support: CapabilitySupport) {
        self.0.insert(id, support);
    }

    /// Builds a set from `(id, support)` declaration pairs.
    ///
    /// # Errors
    ///
    /// Returns the first [`ProtocolError::MalformedCapabilityId`] produced
    /// by an invalid id.
    pub fn from_declarations(
        declarations: &[(&str, CapabilitySupport)],
    ) -> Result<Self, ProtocolError> {
        let mut set = Self::new();
        for (id, support) in declarations {
            set.declare(id, *support)?;
        }
        Ok(set)
    }

    /// Returns the declared support state for `id`, if the endpoint knows it.
    #[must_use]
    pub fn get(&self, id: &CapabilityId) -> Option<CapabilitySupport> {
        self.0.get(id).copied()
    }

    /// Returns the number of declared capabilities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether no capability is declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the ids declared `supported` by this set, in canonical order.
    #[must_use]
    pub fn supported_ids(&self) -> BTreeSet<CapabilityId> {
        self.0
            .iter()
            .filter(|(_, support)| **support == CapabilitySupport::Supported)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_capability_ids() {
        assert!(CapabilityId::from_str("event.streaming").is_ok());
        assert!(CapabilityId::from_str("tool.permissions.v2").is_ok());
        assert!(matches!(
            CapabilityId::from_str(""),
            Err(ProtocolError::MalformedCapabilityId { value }) if value.is_empty()
        ));
        assert!(matches!(
            CapabilityId::from_str("Event.Streaming"),
            Err(ProtocolError::MalformedCapabilityId { value }) if value == "Event.Streaming"
        ));
        let too_long = "a".repeat(65);
        assert!(matches!(
            CapabilityId::from_str(&too_long),
            Err(ProtocolError::MalformedCapabilityId { value }) if value.len() == 65
        ));
    }

    #[test]
    fn tracks_declared_support_deterministically() {
        let set = CapabilitySet::from_declarations(&[
            ("event.streaming", CapabilitySupport::Supported),
            ("thread.steering", CapabilitySupport::Unsupported),
            ("terminal.output", CapabilitySupport::Supported),
        ])
        .unwrap();
        let supported: Vec<String> = set
            .supported_ids()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(supported, ["event.streaming", "terminal.output"]);
        assert_eq!(
            set.get(&CapabilityId::from_str("thread.steering").unwrap()),
            Some(CapabilitySupport::Unsupported)
        );
        assert_eq!(
            set.get(&CapabilityId::from_str("unknown.future").unwrap()),
            None
        );
    }
}
