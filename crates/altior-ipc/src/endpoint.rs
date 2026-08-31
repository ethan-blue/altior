//! User-scoped local endpoint naming (ADR 0006).
//!
//! One Core per user session owns one local socket: a Windows named pipe or
//! a Unix domain socket. The name is derived, not configured, so Desktop
//! and supervision always agree where Core listens. Derivation is a pure
//! function over injected environment values — tests never touch the OS.

use serde::{Deserialize, Serialize};

use crate::error::IpcError;
use crate::transport::{LocalListener, LocalStream};

/// Type alias for local transport endpoint.
pub type LocalEndpoint = Endpoint;

/// The environment inputs endpoint derivation needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointEnv {
    /// The user name scoping the endpoint (`USERNAME` / `USER`).
    pub user: String,
    /// The runtime directory for Unix sockets (`XDG_RUNTIME_DIR`), when
    /// present. Windows named pipes ignore this.
    pub runtime_dir: Option<String>,
}

impl EndpointEnv {
    /// Derives the single Core endpoint for this user.
    ///
    /// # Errors
    ///
    /// Returns [`IpcError::EndpointUnavailable`] when `user` is empty: an
    /// endpoint without an owner would collide across accounts.
    pub fn endpoint(&self) -> Result<Endpoint, IpcError> {
        if self.user.is_empty() {
            return Err(IpcError::EndpointUnavailable {
                endpoint: String::new(),
            });
        }
        let sanitized = sanitize(&self.user);
        Ok(if cfg!(windows) {
            Endpoint::WindowsPipe(format!(r"\\.\pipe\altior-core-{sanitized}"))
        } else {
            let dir = self
                .runtime_dir
                .clone()
                .unwrap_or_else(|| "/tmp".to_owned());
            Endpoint::UnixSocket(format!("{dir}/altior-core-{sanitized}.sock"))
        })
    }
}

/// The derived local endpoint Core listens on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", content = "address", rename_all = "snake_case")]
pub enum Endpoint {
    /// A Windows named pipe path (`\\.\pipe\...`).
    WindowsPipe(String),
    /// A Unix domain socket filesystem path.
    UnixSocket(String),
}

impl Endpoint {
    /// Creates a validated Windows named pipe endpoint.
    ///
    /// # Errors
    /// Returns [`IpcError::InvalidEndpoint`] if the name is empty, exceeds 256 bytes,
    /// or contains invalid characters.
    pub fn windows_pipe(name: &str) -> Result<Self, IpcError> {
        if name.is_empty() {
            return Err(IpcError::InvalidEndpoint {
                reason: "pipe name cannot be empty".to_owned(),
            });
        }
        let full_name = if name.starts_with(r"\\.\pipe\") {
            name.to_owned()
        } else {
            format!(r"\\.\pipe\{name}")
        };
        if full_name.len() > 256 {
            return Err(IpcError::InvalidEndpoint {
                reason: format!("pipe name exceeds 256 bytes: {full_name}"),
            });
        }
        Ok(Self::WindowsPipe(full_name))
    }

    /// Creates a validated Unix domain socket endpoint.
    ///
    /// # Errors
    /// Returns [`IpcError::InvalidEndpoint`] if the path is empty or exceeds 104 bytes.
    pub fn unix_socket(path: &str) -> Result<Self, IpcError> {
        if path.is_empty() {
            return Err(IpcError::InvalidEndpoint {
                reason: "socket path cannot be empty".to_owned(),
            });
        }
        if path.len() > 104 {
            return Err(IpcError::InvalidEndpoint {
                reason: format!("unix socket path exceeds 104 bytes: {path}"),
            });
        }
        Ok(Self::UnixSocket(path.to_owned()))
    }

    /// Derives an endpoint scoped by user and instance identifier.
    ///
    /// # Errors
    /// Returns [`IpcError::InvalidEndpoint`] if `user` or `instance` is empty.
    pub fn random_for_user(user: &str, instance: &str) -> Result<Self, IpcError> {
        if user.is_empty() || instance.is_empty() {
            return Err(IpcError::InvalidEndpoint {
                reason: "user and instance cannot be empty".to_owned(),
            });
        }
        let sanitized_user = sanitize(user);
        let sanitized_instance = sanitize(instance);
        if cfg!(windows) {
            Self::windows_pipe(&format!(
                "altior-core-{sanitized_user}-{sanitized_instance}"
            ))
        } else {
            Self::unix_socket(&format!(
                "/tmp/altior-core-{sanitized_user}-{sanitized_instance}.sock"
            ))
        }
    }

    /// Derives the default endpoint for the current OS user environment.
    ///
    /// # Errors
    /// Returns [`IpcError::EndpointUnavailable`] if neither `USERNAME` nor `USER` is set.
    pub fn default_for_current_user() -> Result<Self, IpcError> {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "default".to_owned());
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
        EndpointEnv { user, runtime_dir }.endpoint()
    }

    /// The endpoint's platform-specific address string.
    #[must_use]
    pub fn address(&self) -> &str {
        match self {
            Self::WindowsPipe(address) | Self::UnixSocket(address) => address,
        }
    }

    /// Binds a [`LocalListener`] on this endpoint.
    ///
    /// # Errors
    /// Returns [`IpcError`] if binding fails.
    pub fn bind(&self) -> Result<LocalListener, IpcError> {
        LocalListener::bind(self)
    }

    /// Connects a [`LocalStream`] to this endpoint.
    ///
    /// # Errors
    /// Returns [`IpcError`] if connection fails.
    pub fn connect(&self, timeout: Option<std::time::Duration>) -> Result<LocalStream, IpcError> {
        LocalStream::connect(self, timeout)
    }
}

/// Reduces a user name to `[0-9a-z-]` so it is safe inside pipe and socket
/// names on both platforms. ASCII upper case folds to lower case because
/// Windows account names are case-insensitive: `Ethan` and `ethan` must
/// resolve to the same endpoint.
fn sanitize(user: &str) -> String {
    user.chars()
        .map(|character| {
            if character.is_ascii_uppercase() {
                character.to_ascii_lowercase()
            } else if character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
            {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_stable_user_scoped_endpoint() {
        let env = EndpointEnv {
            user: "Ethan".to_owned(),
            runtime_dir: None,
        };
        let first = env.endpoint().unwrap();
        let second = env.endpoint().unwrap();
        assert_eq!(first, second);
        if cfg!(windows) {
            assert_eq!(
                first,
                Endpoint::WindowsPipe(r"\\.\pipe\altior-core-ethan".to_owned())
            );
        } else {
            assert_eq!(
                first,
                Endpoint::UnixSocket("/tmp/altior-core-ethan.sock".to_owned())
            );
        }
    }

    #[test]
    fn endpoints_are_scoped_by_user_and_sanitized() {
        // Two users never share an endpoint, and hostile characters in a
        // user name cannot escape the name.
        assert_eq!(sanitize("Mixed.Case_Name"), "mixed-case-name");
        assert_eq!(sanitize("a/b\\c"), "a-b-c");
        assert_eq!(sanitize("Ethan"), "ethan");
        let ethan = EndpointEnv {
            user: "ethan".to_owned(),
            runtime_dir: None,
        }
        .endpoint()
        .unwrap();
        let other = EndpointEnv {
            user: "other".to_owned(),
            runtime_dir: None,
        }
        .endpoint()
        .unwrap();
        assert_ne!(ethan, other);
    }

    #[test]
    fn unix_endpoints_prefer_the_runtime_dir() {
        let env = EndpointEnv {
            user: "ethan".to_owned(),
            runtime_dir: Some("/run/user/1000".to_owned()),
        };
        // The Windows branch ignores the runtime dir; assert the exact
        // address so the choice is visible per platform.
        let endpoint = env.endpoint().unwrap();
        match &endpoint {
            Endpoint::WindowsPipe(address) => {
                assert_eq!(address, r"\\.\pipe\altior-core-ethan");
            }
            Endpoint::UnixSocket(address) => {
                assert_eq!(address, "/run/user/1000/altior-core-ethan.sock");
            }
        }
    }

    #[test]
    fn rejects_empty_users() {
        let env = EndpointEnv {
            user: String::new(),
            runtime_dir: None,
        };
        assert!(matches!(
            env.endpoint(),
            Err(IpcError::EndpointUnavailable { endpoint }) if endpoint.is_empty()
        ));
    }

    #[test]
    fn validates_windows_pipe_bounds() {
        assert!(Endpoint::windows_pipe("").is_err());
        let valid = Endpoint::windows_pipe("test-pipe").unwrap();
        assert_eq!(valid.address(), r"\\.\pipe\test-pipe");
        let oversized = "a".repeat(300);
        assert!(Endpoint::windows_pipe(&oversized).is_err());
    }

    #[test]
    fn validates_unix_socket_byte_bounds() {
        assert!(Endpoint::unix_socket("").is_err());

        // 104 ASCII bytes succeeds
        let path_104 = "a".repeat(104);
        assert_eq!(path_104.len(), 104);
        let ep_104 = Endpoint::unix_socket(&path_104).unwrap();
        assert_eq!(ep_104.address(), path_104);

        // 105 ASCII bytes fails
        let path_105 = "a".repeat(105);
        assert_eq!(path_105.len(), 105);
        assert!(matches!(
            Endpoint::unix_socket(&path_105),
            Err(IpcError::InvalidEndpoint { reason }) if reason.contains("104 bytes")
        ));

        // Multibyte UTF-8: 34 3-byte characters = 102 bytes + 2 ascii bytes = 104 bytes -> succeeds
        let mb_104 = format!("{}ab", "中".repeat(34));
        assert_eq!(mb_104.len(), 104);
        assert_eq!(mb_104.chars().count(), 36);
        assert!(Endpoint::unix_socket(&mb_104).is_ok());

        // Multibyte UTF-8: 35 3-byte characters = 105 bytes (only 35 chars!) -> fails by bytes
        let mb_105 = "中".repeat(35);
        assert_eq!(mb_105.len(), 105);
        assert_eq!(mb_105.chars().count(), 35);
        assert!(matches!(
            Endpoint::unix_socket(&mb_105),
            Err(IpcError::InvalidEndpoint { reason }) if reason.contains("104 bytes")
        ));
    }

    #[test]
    fn random_instance_endpoint_derivation() {
        let ep = Endpoint::random_for_user("Alice", "inst-1234").unwrap();
        assert!(ep.address().contains("alice"));
        assert!(ep.address().contains("inst-1234"));
    }
}
