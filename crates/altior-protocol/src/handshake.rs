//! Desktop/Core handshake and protocol version negotiation.
//!
//! Desktop sends a [`DesktopHello`]; Core replies with a [`CoreHello`].
//! Negotiation intersects the advertised version ranges and selects the
//! highest common version. An empty intersection fails explicitly — there
//! is no silent downgrade. Capabilities are declared explicitly on both
//! sides; a capability is negotiated only when both sides declare it
//! supported, and one-sided claims are surfaced for diagnostics rather
//! than inferred from product versions (ADR 0004).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityId, CapabilitySet, CapabilitySupport};
use crate::error::ProtocolError;
use crate::version::{ProductVersion, ProtocolVersion, ProtocolVersionRange};

/// Desktop's opening handshake message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct DesktopHello {
    /// Protocol versions this Desktop build can speak, inclusive.
    pub supported_versions: ProtocolVersionRange,
    /// Desktop's product build version, for diagnostics only.
    pub desktop_version: ProductVersion,
    /// Capabilities this Desktop build explicitly declares.
    pub capabilities: CapabilitySet,
}

/// Core's handshake reply.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct CoreHello {
    /// Protocol versions this Core build can speak, inclusive.
    pub supported_versions: ProtocolVersionRange,
    /// Core's product build version, for diagnostics only.
    pub core_version: ProductVersion,
    /// Capabilities this Core build explicitly declares.
    pub capabilities: CapabilitySet,
}

/// The result of a successful handshake negotiation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "dto-export",
    derive(ts_rs::TS),
    ts(export, export_to = "../../../apps/desktop/src/ipc/dto/")
)]
pub struct NegotiatedHandshake {
    /// The highest protocol version common to both sides.
    pub selected_version: ProtocolVersion,
    /// Desktop's product build version, carried for diagnostics.
    pub desktop_version: ProductVersion,
    /// Core's product build version, carried for diagnostics.
    pub core_version: ProductVersion,
    /// Capabilities both sides explicitly declared supported.
    pub negotiated_capabilities: CapabilitySet,
    /// Capabilities declared supported by Desktop only, for diagnostics.
    #[cfg_attr(feature = "dto-export", ts(as = "std::collections::BTreeSet<String>"))]
    pub desktop_only: BTreeSet<CapabilityId>,
    /// Capabilities declared supported by Core only, for diagnostics.
    #[cfg_attr(feature = "dto-export", ts(as = "std::collections::BTreeSet<String>"))]
    pub core_only: BTreeSet<CapabilityId>,
}

/// Negotiates a protocol version and capability set from both hello
/// messages.
///
/// # Errors
///
/// Returns [`ProtocolError::NoCommonProtocolVersion`] when the advertised
/// version ranges have no intersection. Version negotiation never
/// silently downgrades.
pub fn negotiate(
    desktop: &DesktopHello,
    core: &CoreHello,
) -> Result<NegotiatedHandshake, ProtocolError> {
    let intersection = desktop
        .supported_versions
        .intersect(core.supported_versions)
        .ok_or(ProtocolError::NoCommonProtocolVersion {
            desktop: desktop.supported_versions,
            core: core.supported_versions,
        })?;

    let mut negotiated_capabilities = CapabilitySet::new();
    let mut desktop_only = BTreeSet::new();
    let mut core_only = BTreeSet::new();

    let desktop_supported = desktop.capabilities.supported_ids();
    let core_supported = core.capabilities.supported_ids();
    for id in desktop_supported.intersection(&core_supported) {
        negotiated_capabilities.declare_validated(id.clone(), CapabilitySupport::Supported);
    }
    for id in desktop_supported.difference(&core_supported) {
        desktop_only.insert(id.clone());
    }
    for id in core_supported.difference(&desktop_supported) {
        core_only.insert(id.clone());
    }

    Ok(NegotiatedHandshake {
        selected_version: intersection.max(),
        desktop_version: desktop.desktop_version,
        core_version: core.core_version,
        negotiated_capabilities,
        desktop_only,
        core_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop_hello(min: u32, max: u32) -> DesktopHello {
        DesktopHello {
            supported_versions: range(min, max),
            desktop_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
        }
    }

    fn core_hello(min: u32, max: u32) -> CoreHello {
        CoreHello {
            supported_versions: range(min, max),
            core_version: ProductVersion::new(0, 1, 0),
            capabilities: CapabilitySet::new(),
        }
    }

    fn range(min: u32, max: u32) -> ProtocolVersionRange {
        ProtocolVersionRange::try_new(
            ProtocolVersion::try_new(min).unwrap(),
            ProtocolVersion::try_new(max).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn selects_the_highest_common_version() {
        let handshake = negotiate(&desktop_hello(1, 3), &core_hello(2, 5)).unwrap();
        assert_eq!(handshake.selected_version.as_u32(), 3);
        assert_eq!(handshake.desktop_version, ProductVersion::new(0, 1, 0));
        assert_eq!(handshake.core_version, ProductVersion::new(0, 1, 0));
    }

    #[test]
    fn fails_explicitly_when_ranges_do_not_overlap() {
        let error = negotiate(&desktop_hello(1, 2), &core_hello(3, 5)).unwrap_err();
        assert!(matches!(
            error,
            ProtocolError::NoCommonProtocolVersion { ref desktop, ref core }
                if *desktop == range(1, 2) && *core == range(3, 5)
        ));
    }
}
