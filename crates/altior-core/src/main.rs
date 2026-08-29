//! Altior Core composition root.
//!
//! P0.1 wires only the handshake contract; process supervision, IPC
//! transport, and the agent runtime arrive in later work packages.

use std::str::FromStr;

use altior_protocol::{CapabilitySet, CoreHello, ProductVersion, SUPPORTED_PROTOCOL_VERSIONS};

fn main() {
    // `CARGO_PKG_VERSION` is the single source of truth for the build
    // version; a malformed value here is a build bug, not a runtime input.
    let core_version =
        ProductVersion::from_str(env!("CARGO_PKG_VERSION")).expect("workspace version is semver");
    let hello = CoreHello {
        supported_versions: SUPPORTED_PROTOCOL_VERSIONS,
        core_version,
        capabilities: CapabilitySet::new(),
    };
    println!(
        "Altior Core {} (IPC {}), capabilities declared: {}",
        hello.core_version,
        hello.supported_versions,
        hello.capabilities.len(),
    );
}
