//! Core discovery: user-scoped endpoint resolution and launch token lookup (ADR 0006).
//!
//! Plaintext secret rule (ADR 0006 / ADR 0014): Launch tokens are capability secrets
//! and must NEVER be printed to logs, traces, or debug outputs.

use std::fmt;
use std::path::PathBuf;

use altior_ipc::auth::LaunchCredentials;
use altior_ipc::endpoint::{Endpoint, EndpointEnv};

use crate::error::DiscoveryError;

/// Trait for discovering a running Core instance and resolving its IPC endpoint.
pub trait CoreDiscovery: Send + Sync {
    /// Discovers existing Core launch credentials, if available.
    fn discover_credentials(&self) -> Result<Option<LaunchCredentials>, DiscoveryError>;

    /// Resolves the local IPC endpoint address for this user session.
    fn resolve_endpoint(&self) -> Result<Endpoint, DiscoveryError>;

    /// Invalidates / cleans up a stale discovery file when Core has exited.
    fn invalidate_stale_token(&self) -> Result<(), DiscoveryError>;

    /// Returns the resolved token file path for diagnostics (without reading token value).
    fn token_file_path(&self) -> Result<PathBuf, DiscoveryError>;
}

/// Filesystem-based Core discovery using standard user runtime directories.
pub struct FsCoreDiscovery {
    custom_token_path: Option<PathBuf>,
    env: EndpointEnv,
}

impl FsCoreDiscovery {
    /// Creates discovery using OS environment variables (`USERNAME`/`USER`, `XDG_RUNTIME_DIR`).
    #[must_use]
    pub fn from_env() -> Self {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "default-user".to_string());
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
        Self {
            custom_token_path: std::env::var("ALTIOR_TOKEN_FILE").ok().map(PathBuf::from),
            env: EndpointEnv { user, runtime_dir },
        }
    }

    /// Creates discovery with explicit user environment and optional custom token file path.
    #[must_use]
    pub fn new(env: EndpointEnv, custom_token_path: Option<PathBuf>) -> Self {
        Self {
            custom_token_path,
            env,
        }
    }

    /// Derives the canonical token file path for the current user session.
    fn derive_token_path(&self) -> Result<PathBuf, DiscoveryError> {
        if let Some(ref path) = self.custom_token_path {
            return Ok(path.clone());
        }

        let sanitized_user = sanitize_user(&self.env.user);
        let token_filename = format!("altior-core-{sanitized_user}.token");

        if cfg!(windows) {
            // Windows: %LOCALAPPDATA%\altior or %TEMP%\altior
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                let dir = PathBuf::from(local_app_data).join("altior");
                let _ = std::fs::create_dir_all(&dir);
                return Ok(dir.join(token_filename));
            }
            let temp_dir = std::env::temp_dir().join("altior");
            let _ = std::fs::create_dir_all(&temp_dir);
            Ok(temp_dir.join(token_filename))
        } else {
            // Unix: $XDG_RUNTIME_DIR/altior or /tmp
            if let Some(ref runtime_dir) = self.env.runtime_dir {
                let dir = PathBuf::from(runtime_dir).join("altior");
                let _ = std::fs::create_dir_all(&dir);
                return Ok(dir.join(token_filename));
            }
            Ok(PathBuf::from("/tmp").join(token_filename))
        }
    }
}

impl CoreDiscovery for FsCoreDiscovery {
    fn discover_credentials(&self) -> Result<Option<LaunchCredentials>, DiscoveryError> {
        let path = self.derive_token_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => return Err(DiscoveryError::Io(err.to_string())),
        };

        // Parse credentials without printing token
        let creds = altior_ipc::decode_token_file(&content).map_err(|err| {
            DiscoveryError::DecodeFailed(format!("failed to parse token file: {err}"))
        })?;

        Ok(Some(creds))
    }

    fn resolve_endpoint(&self) -> Result<Endpoint, DiscoveryError> {
        self.env
            .endpoint()
            .map_err(|err| DiscoveryError::EndpointDerivation(err.to_string()))
    }

    fn invalidate_stale_token(&self) -> Result<(), DiscoveryError> {
        let path = self.derive_token_path()?;
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }

    fn token_file_path(&self) -> Result<PathBuf, DiscoveryError> {
        self.derive_token_path()
    }
}

// Redacted debug output to prevent token leakage in debug logs
impl fmt::Debug for FsCoreDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FsCoreDiscovery")
            .field("user", &self.env.user)
            .field("token_path", &self.derive_token_path().ok())
            .finish()
    }
}

fn sanitize_user(user: &str) -> String {
    user.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
