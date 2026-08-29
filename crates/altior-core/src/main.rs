//! Altior Core composition root.
//!
//! P0.2 links the supervision, ownership, and IPC contract layers; the OS
//! transport (tokio named pipes / Unix domain sockets, ADR 0006) and the
//! agent runtime arrive in later work packages. The binary remains a thin
//! banner until the transport slice lands.

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
