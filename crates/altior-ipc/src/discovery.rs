//! Endpoint discovery and token file publication (ADR 0006).
//!
//! On startup, Core publishes an atomic discovery file containing its
//! instance ID, endpoint, and opaque launch token. Desktop reads this
//! file to attach to the live Core instance.
//!
//! File operations use atomic write (write to temp file in same directory +
//! rename) and platform-appropriate permissions (0600 on Unix, default ACL
//! best effort on Windows). Stale discovery files can be detected and cleaned up.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use altior_domain::CoreInstanceId;
use altior_protocol::{LaunchToken, ProtocolError};

use crate::endpoint::Endpoint;
use crate::error::IpcError;

/// The endpoint discovery document published by Core.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EndpointDiscovery {
    /// The Core launch identifier.
    pub instance_id: CoreInstanceId,
    /// The derived IPC endpoint for this launch.
    pub endpoint: Endpoint,
    /// The opaque capability token required to authenticate.
    pub launch_token: LaunchToken,
}

impl fmt::Debug for EndpointDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EndpointDiscovery")
            .field("instance_id", &self.instance_id)
            .field("endpoint", &self.endpoint)
            .field("launch_token", &"[REDACTED]")
            .finish()
    }
}

/// Encodes endpoint discovery as JSON.
///
/// # Errors
///
/// Returns [`IpcError::Protocol`] when encoding fails.
pub fn encode_discovery(discovery: &EndpointDiscovery) -> Result<String, IpcError> {
    serde_json::to_string(discovery).map_err(|error| IpcError::Protocol {
        source: ProtocolError::MalformedEnvelope { source: error },
    })
}

/// Parses endpoint discovery from JSON.
///
/// # Errors
///
/// Returns [`IpcError::Protocol`] when the input is not a valid discovery document.
pub fn decode_discovery(input: &str) -> Result<EndpointDiscovery, IpcError> {
    serde_json::from_str(input).map_err(|error| IpcError::Protocol {
        source: ProtocolError::MalformedEnvelope { source: error },
    })
}

/// Atomically writes discovery metadata to `path`.
///
/// Creates parent directories if missing. Writes to a temporary file
/// adjacent to `path` with permissions restricted to the current user
/// (0600 on Unix, default per-user ACL on Windows), then renames it
/// to `path` for atomic publication.
///
/// # Errors
///
/// Returns [`IpcError`] on I/O or serialization failure.
pub fn write_discovery_file(path: &Path, discovery: &EndpointDiscovery) -> Result<(), IpcError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded = encode_discovery(discovery)?;
    let tmp_path = temporary_path(path);

    std::fs::write(&tmp_path, encoded.as_bytes())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&tmp_path, perms);
    }

    std::fs::rename(&tmp_path, path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        IpcError::from(err)
    })?;

    Ok(())
}

/// Reads and decodes discovery metadata from `path`.
///
/// # Errors
///
/// Returns [`IpcError::NotFound`] if the file does not exist,
/// [`IpcError::AccessDenied`] on permission error, or [`IpcError::Protocol`] on bad JSON.
pub fn read_discovery_file(path: &Path) -> Result<EndpointDiscovery, IpcError> {
    let content = std::fs::read_to_string(path).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => IpcError::NotFound {
            endpoint: path.display().to_string(),
        },
        std::io::ErrorKind::PermissionDenied => IpcError::AccessDenied {
            endpoint: path.display().to_string(),
        },
        _ => IpcError::from(err),
    })?;

    decode_discovery(&content)
}

/// Cleans up a stale discovery file if it exists.
///
/// # Errors
///
/// Returns [`IpcError::AccessDenied`] if file exists but cannot be removed due to permissions,
/// or [`IpcError::Io`] for other failures. Missing files succeed cleanly.
pub fn cleanup_stale_discovery(path: &Path) -> Result<(), IpcError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => IpcError::NotFound {
                endpoint: path.display().to_string(),
            },
            std::io::ErrorKind::PermissionDenied => IpcError::AccessDenied {
                endpoint: path.display().to_string(),
            },
            _ => IpcError::from(err),
        })?;
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = path.file_name().map_or_else(
        || "discovery".to_owned(),
        |s| s.to_string_lossy().to_string(),
    );
    path.with_file_name(format!(".{file_name}.{pid}.{id}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::mint_launch_token;

    #[test]
    fn discovery_debug_redacts_launch_token() {
        let discovery = EndpointDiscovery {
            instance_id: "cor_fixture000000009".parse().unwrap(),
            endpoint: Endpoint::WindowsPipe(r"\\.\pipe\altior-core-ethan".to_owned()),
            launch_token: mint_launch_token(&[0xaa; 16]).unwrap(),
        };
        let debug_str = format!("{discovery:?}");
        assert!(!debug_str.contains("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn discovery_file_atomic_write_and_read_and_cleanup() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("core-discovery.json");

        // Reading non-existent file gives NotFound
        assert!(matches!(
            read_discovery_file(&path),
            Err(IpcError::NotFound { .. })
        ));

        let discovery = EndpointDiscovery {
            instance_id: "cor_fixture000000009".parse().unwrap(),
            endpoint: Endpoint::WindowsPipe(r"\\.\pipe\altior-core-test".to_owned()),
            launch_token: mint_launch_token(&[0x12; 16]).unwrap(),
        };

        // Write discovery
        write_discovery_file(&path, &discovery).unwrap();

        // Read discovery
        let read = read_discovery_file(&path).unwrap();
        assert_eq!(read, discovery);

        // Cleanup
        cleanup_stale_discovery(&path).unwrap();
        assert!(!path.exists());

        // Cleanup on non-existent file succeeds
        cleanup_stale_discovery(&path).unwrap();
    }
}
