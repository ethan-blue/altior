//! Production sync hard latch (ADR 0025 / F31).
//!
//! Personal Vault network synchronization stays disabled until the ADR 0025
//! threat mitigations and acceptance suites pass. Call [`ensure_sync_allowed`]
//! at every future entry point that would start pairing-over-network, relay
//! connect, or vault sync — never open those paths behind a soft config flag.

use std::error::Error;
use std::fmt;

/// Authoritative production gate. Must remain `false` until ADR 0025 is closed.
pub const SYNC_ENABLED: bool = false;

/// Fail-closed error when a caller attempts to enable or run sync while gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncDisabled;

impl fmt::Display for SyncDisabled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "personal vault sync is disabled until ADR 0025 mitigations and acceptance suites pass (SYNC_ENABLED=false)"
        )
    }
}

impl Error for SyncDisabled {}

/// Returns `Ok(())` only when production sync is explicitly allowed.
///
/// Today this always returns [`SyncDisabled`]. Flip [`SYNC_ENABLED`] only after
/// ADR 0025 review evidence lands — never via Desktop UI or env vars.
pub fn ensure_sync_allowed() -> Result<(), SyncDisabled> {
    if SYNC_ENABLED {
        Ok(())
    } else {
        Err(SyncDisabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_sync_remains_hard_disabled() {
        assert!(
            !SYNC_ENABLED,
            "SYNC_ENABLED must stay false until ADR 0025 acceptance suites pass"
        );
        assert_eq!(ensure_sync_allowed(), Err(SyncDisabled));
    }

    #[test]
    fn disabled_error_names_adr_0025() {
        let msg = SyncDisabled.to_string();
        assert!(msg.contains("ADR 0025"), "message was: {msg}");
        assert!(msg.contains("SYNC_ENABLED=false"), "message was: {msg}");
    }
}