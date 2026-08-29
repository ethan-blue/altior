//! User-scoped local endpoint naming (ADR 0006).
//!
//! One Core per user session owns one local socket: a Windows named pipe or
//! a Unix domain socket. The name is derived, not configured, so Desktop
//! and supervision always agree where Core listens. Derivation is a pure
//! function over injected environment values — tests never touch the OS.

use serde::{Deserialize, Serialize};

use crate::error::IpcError;

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
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum Endpoint {
    /// A Windows named pipe path (`\\.\pipe\...`).
    WindowsPipe(String),
    /// A Unix domain socket filesystem path.
    UnixSocket(String),
}

impl Endpoint {
    /// The endpoint's platform-specific address string.
    #[must_use]
    pub fn address(&self) -> &str {
        match self {
            Self::WindowsPipe(address) | Self::UnixSocket(address) => address,
        }
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
}
