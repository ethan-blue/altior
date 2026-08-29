//! Protocol and product version contracts.
//!
//! The IPC protocol version is a positive integer. Each endpoint advertises
//! an inclusive range; negotiation intersects the ranges and selects the
//! highest common version, failing explicitly when the intersection is
//! empty (ADR 0004). Product versions describe builds for diagnostics and
//! never gate behavior — capabilities do that.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

/// A Desktop/Core IPC protocol version. Versions start at 1.
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
pub struct ProtocolVersion(u32);

impl ProtocolVersion {
    /// The first Desktop/Core protocol version.
    pub const V1: Self = Self(1);

    /// Constructs a protocol version, rejecting zero.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidProtocolVersion`] when `version` is
    /// zero.
    pub const fn try_new(version: u32) -> Result<Self, ProtocolError> {
        if version == 0 {
            Err(ProtocolError::InvalidProtocolVersion { version })
        } else {
            Ok(Self(version))
        }
    }

    /// Returns the raw version number.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ProtocolVersion {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed: u32 = s
            .parse()
            .map_err(|_| ProtocolError::MalformedProtocolVersion {
                value: s.to_owned(),
            })?;
        Self::try_new(parsed)
    }
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = ProtocolError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

/// An inclusive range of protocol versions advertised by one endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ProtocolVersionRange {
    min: ProtocolVersion,
    max: ProtocolVersion,
}

impl ProtocolVersionRange {
    /// Constructs a validated inclusive range.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidProtocolVersionRange`] when `min`
    /// exceeds `max`.
    pub const fn try_new(
        min: ProtocolVersion,
        max: ProtocolVersion,
    ) -> Result<Self, ProtocolError> {
        if min.0 > max.0 {
            Err(ProtocolError::InvalidProtocolVersionRange {
                min: min.0,
                max: max.0,
            })
        } else {
            Ok(Self { min, max })
        }
    }

    /// Returns the inclusive lower bound.
    #[must_use]
    pub const fn min(self) -> ProtocolVersion {
        self.min
    }

    /// Returns the inclusive upper bound.
    #[must_use]
    pub const fn max(self) -> ProtocolVersion {
        self.max
    }

    /// Returns whether `version` falls inside this range.
    #[must_use]
    pub const fn contains(self, version: ProtocolVersion) -> bool {
        version.0 >= self.min.0 && version.0 <= self.max.0
    }

    /// Returns the intersection of two ranges, if non-empty.
    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        (min.0 <= max.0).then_some(Self { min, max })
    }
}

impl fmt::Display for ProtocolVersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}]", self.min, self.max)
    }
}

/// The protocol versions this endpoint can speak. Extending the maximum is
/// additive; retiring a version is a documented release action (ADR 0004).
pub const SUPPORTED_PROTOCOL_VERSIONS: ProtocolVersionRange = ProtocolVersionRange {
    min: ProtocolVersion::V1,
    max: ProtocolVersion::V1,
};

/// A `major.minor.patch` product build version.
///
/// Product versions travel through the handshake for diagnostics and
/// upgrade prompts only. They never gate protocol behavior; capability
/// support is negotiated explicitly instead of being inferred from a
/// version string.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct ProductVersion {
    /// Major component.
    pub major: u16,
    /// Minor component.
    pub minor: u16,
    /// Patch component.
    pub patch: u16,
}

impl ProductVersion {
    /// Constructs a product version from components.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for ProductVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for ProductVersion {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let reject = || ProtocolError::MalformedProductVersion {
            value: s.to_owned(),
        };
        let mut parts = s.split('.');
        let major = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(reject)?;
        let minor = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(reject)?;
        let patch = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(reject)?;
        if parts.next().is_some() {
            return Err(reject());
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_non_numeric_versions() {
        assert!(matches!(
            ProtocolVersion::try_new(0),
            Err(ProtocolError::InvalidProtocolVersion { version: 0 })
        ));
        assert!(matches!(
            "1.x".parse::<ProtocolVersion>(),
            Err(ProtocolError::MalformedProtocolVersion { value }) if value == "1.x"
        ));
        assert_eq!(
            "2".parse::<ProtocolVersion>().unwrap().as_u32(),
            ProtocolVersion::try_new(2).unwrap().as_u32()
        );
    }

    #[test]
    fn rejects_empty_version_ranges() {
        let min = ProtocolVersion::V1;
        let max = ProtocolVersion::try_new(5).unwrap();
        assert!(matches!(
            ProtocolVersionRange::try_new(max, min),
            Err(ProtocolError::InvalidProtocolVersionRange { min: 5, max: 1 })
        ));
    }

    #[test]
    fn intersects_ranges_and_reports_emptiness() {
        let a = ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(1).unwrap(),
            ProtocolVersion::try_new(3).unwrap(),
        )
        .unwrap();
        let b = ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(2).unwrap(),
            ProtocolVersion::try_new(5).unwrap(),
        )
        .unwrap();
        let c = ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(4).unwrap(),
            ProtocolVersion::try_new(6).unwrap(),
        )
        .unwrap();
        let intersection = a.intersect(b).unwrap();
        assert_eq!(intersection.min().as_u32(), 2);
        assert_eq!(intersection.max().as_u32(), 3);
        assert!(a.intersect(c).is_none());
        assert!(a.contains(ProtocolVersion::try_new(3).unwrap()));
        assert!(!a.contains(ProtocolVersion::try_new(4).unwrap()));
    }

    #[test]
    fn parses_and_displays_product_versions() {
        let version: ProductVersion = "0.1.0".parse().unwrap();
        assert_eq!(version, ProductVersion::new(0, 1, 0));
        assert_eq!(version.to_string(), "0.1.0");
        assert!("1.2".parse::<ProductVersion>().is_err());
        assert!("1.2.3.4".parse::<ProductVersion>().is_err());
        assert!("1.2.x".parse::<ProductVersion>().is_err());
    }
}
