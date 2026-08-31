//! Launch configuration and secret-reference boundary for ACP subprocesses.
//!
//! Subprocess launches must be bounded, typed, and strictly reject plaintext
//! secrets in persisted or logged configurations (P1.2, ADR 0007).
//!
//! Shell command concatenation is prohibited; invocations must use explicit
//! `program` + `args` arrays.
//!
//! Secret resolution is kept behind an integration seam ([`SecretResolver`]);
//! this adapter never touches OS keychains or DPAPI directly, and all
//! `Debug`/`Display` implementations redact resolved environment variables.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::AcpError;

/// Maximum program path length in bytes (matches domain `BoundedPath`).
pub const MAX_PROGRAM_BYTES: usize = 4096;

/// Maximum number of command arguments.
pub const MAX_ARGS_COUNT: usize = 256;

/// Maximum length of a single argument in bytes (64 KiB).
pub const MAX_ARG_BYTES: usize = 64 * 1024;

/// Maximum number of environment variable entries.
pub const MAX_ENV_COUNT: usize = 256;

/// Maximum environment variable key length in bytes.
pub const MAX_ENV_KEY_BYTES: usize = 256;

/// Maximum environment variable value length in bytes (64 KiB).
pub const MAX_ENV_VALUE_BYTES: usize = 64 * 1024;

/// Maximum bounded stderr capture buffer in bytes (64 KiB).
pub const MAX_STDERR_CAPTURE_BYTES: usize = 64 * 1024;

/// Maximum secret reference identifier length in bytes.
pub const MAX_SECRET_REF_BYTES: usize = 256;

/// An opaque reference to a secret stored in the platform's secret store.
///
/// Plaintext secrets must never be held in this type or serialized.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SecretRef(String);

impl SecretRef {
    /// Creates a new validated opaque secret reference.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if `reference` is empty or exceeds
    /// [`MAX_SECRET_REF_BYTES`].
    pub fn new(reference: impl Into<String>) -> Result<Self, AcpError> {
        let reference = reference.into();
        let trimmed = reference.trim();
        if trimmed.is_empty() {
            return Err(AcpError::InvalidConfig {
                diagnostic: "secret reference must not be empty".to_owned(),
            });
        }
        if trimmed.len() > MAX_SECRET_REF_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "secret reference length {} exceeds maximum {}",
                    trimmed.len(),
                    MAX_SECRET_REF_BYTES
                ),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the reference identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretRef").field(&self.0).finish()
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The value of an environment variable in a [`LaunchConfig`].
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum EnvVarValue {
    /// A non-secret literal value.
    Literal(String),
    /// An opaque secret reference to be resolved before spawn.
    SecretRef(SecretRef),
}

impl fmt::Debug for EnvVarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(val) => f.debug_tuple("Literal").field(val).finish(),
            Self::SecretRef(sref) => f.debug_tuple("SecretRef").field(sref).finish(),
        }
    }
}

/// Typed, bounded launch configuration for an ACP agent subprocess.
///
/// Shell concatenation is prohibited: `program` and `args` are strictly separate.
/// Secrets are accepted only as opaque [`SecretRef`] entries.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaunchConfig {
    /// Executable program path or name.
    program: String,
    /// Ordered command-line arguments.
    args: Vec<String>,
    /// Optional working directory.
    working_dir: Option<String>,
    /// Environment variable configuration.
    env: BTreeMap<String, EnvVarValue>,
}

impl LaunchConfig {
    /// Creates a new launch configuration with the specified program.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if `program` is empty or exceeds
    /// [`MAX_PROGRAM_BYTES`].
    pub fn new(program: impl Into<String>) -> Result<Self, AcpError> {
        let program = program.into();
        let trimmed = program.trim();
        if trimmed.is_empty() {
            return Err(AcpError::InvalidConfig {
                diagnostic: "program path must not be empty".to_owned(),
            });
        }
        if trimmed.len() > MAX_PROGRAM_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "program path length {} exceeds limit {}",
                    trimmed.len(),
                    MAX_PROGRAM_BYTES
                ),
            });
        }
        Ok(Self {
            program: trimmed.to_owned(),
            args: Vec::new(),
            working_dir: None,
            env: BTreeMap::new(),
        })
    }

    /// Appends a single command argument.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if arg count exceeds [`MAX_ARGS_COUNT`]
    /// or arg length exceeds [`MAX_ARG_BYTES`].
    pub fn with_arg(mut self, arg: impl Into<String>) -> Result<Self, AcpError> {
        if self.args.len() >= MAX_ARGS_COUNT {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!("argument count exceeds limit of {MAX_ARGS_COUNT}"),
            });
        }
        let arg = arg.into();
        if arg.len() > MAX_ARG_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "argument length {} exceeds limit {}",
                    arg.len(),
                    MAX_ARG_BYTES
                ),
            });
        }
        self.args.push(arg);
        Ok(self)
    }

    /// Appends multiple command arguments.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if arg limits are violated.
    pub fn with_args<I, S>(mut self, args: I) -> Result<Self, AcpError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self = self.with_arg(arg)?;
        }
        Ok(self)
    }

    /// Sets the working directory.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if `working_dir` exceeds [`MAX_PROGRAM_BYTES`].
    pub fn with_working_dir(mut self, cwd: impl Into<String>) -> Result<Self, AcpError> {
        let cwd = cwd.into();
        let trimmed = cwd.trim();
        if trimmed.len() > MAX_PROGRAM_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "working directory length {} exceeds limit {}",
                    trimmed.len(),
                    MAX_PROGRAM_BYTES
                ),
            });
        }
        self.working_dir = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        };
        Ok(self)
    }

    /// Adds a literal environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if limits on env count, key length,
    /// or value length are exceeded.
    pub fn with_literal_env(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, AcpError> {
        let key = key.into();
        let value = value.into();
        Self::validate_env_key_and_value(&key, &value, self.env.len())?;
        self.env.insert(key, EnvVarValue::Literal(value));
        Ok(self)
    }

    /// Adds an opaque secret environment variable reference.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if limits on env count or key length
    /// are exceeded.
    pub fn with_secret_env(
        mut self,
        key: impl Into<String>,
        secret_ref: SecretRef,
    ) -> Result<Self, AcpError> {
        let key = key.into();
        Self::validate_env_key(&key, self.env.len())?;
        self.env.insert(key, EnvVarValue::SecretRef(secret_ref));
        Ok(self)
    }

    fn validate_env_key(key: &str, current_count: usize) -> Result<(), AcpError> {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty() {
            return Err(AcpError::InvalidConfig {
                diagnostic: "environment variable key must not be empty".to_owned(),
            });
        }
        if trimmed_key.len() > MAX_ENV_KEY_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "environment variable key length {} exceeds limit {}",
                    trimmed_key.len(),
                    MAX_ENV_KEY_BYTES
                ),
            });
        }
        if current_count >= MAX_ENV_COUNT {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!("environment variable count exceeds limit of {MAX_ENV_COUNT}"),
            });
        }
        Ok(())
    }

    fn validate_env_key_and_value(
        key: &str,
        value: &str,
        current_count: usize,
    ) -> Result<(), AcpError> {
        Self::validate_env_key(key, current_count)?;
        if value.len() > MAX_ENV_VALUE_BYTES {
            return Err(AcpError::InvalidConfig {
                diagnostic: format!(
                    "environment variable value length {} exceeds limit {}",
                    value.len(),
                    MAX_ENV_VALUE_BYTES
                ),
            });
        }
        Ok(())
    }

    /// The program executable path.
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The ordered argument list.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The optional working directory.
    #[must_use]
    pub fn working_dir(&self) -> Option<&str> {
        self.working_dir.as_deref()
    }

    /// The environment variable mapping.
    #[must_use]
    pub fn env(&self) -> &BTreeMap<String, EnvVarValue> {
        &self.env
    }

    /// Resolves all secret references via the given resolver seam to construct
    /// a [`ResolvedLaunchConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::SecretResolutionFailed`] if resolving any secret fails.
    pub fn resolve<R: SecretResolver + ?Sized>(
        &self,
        resolver: &R,
    ) -> Result<ResolvedLaunchConfig, AcpError> {
        let mut resolved_env = BTreeMap::new();
        for (k, v) in &self.env {
            let resolved_val = match v {
                EnvVarValue::Literal(lit) => lit.clone(),
                EnvVarValue::SecretRef(sref) => resolver.resolve_secret(sref)?,
            };
            if resolved_val.len() > MAX_ENV_VALUE_BYTES {
                return Err(AcpError::InvalidConfig {
                    diagnostic: format!(
                        "resolved secret value for '{k}' exceeds maximum length {MAX_ENV_VALUE_BYTES}"
                    ),
                });
            }
            resolved_env.insert(k.clone(), resolved_val);
        }

        Ok(ResolvedLaunchConfig {
            program: self.program.clone(),
            args: self.args.clone(),
            working_dir: self.working_dir.clone(),
            env: resolved_env,
        })
    }
}

impl fmt::Debug for LaunchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchConfig")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("working_dir", &self.working_dir)
            .field("env", &self.env)
            .finish()
    }
}

/// Secret resolution seam interface.
///
/// Core or the device OS secret store provides an implementation of this trait
/// to resolve opaque references into runtime strings for spawning processes.
pub trait SecretResolver {
    /// Resolves an opaque secret reference into its runtime secret string.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::SecretResolutionFailed`] if the secret cannot be found
    /// or decrypted.
    fn resolve_secret(&self, secret_ref: &SecretRef) -> Result<String, AcpError>;
}

/// A closure or function pointer can act as a `SecretResolver`.
impl<F> SecretResolver for F
where
    F: Fn(&SecretRef) -> Result<String, AcpError>,
{
    fn resolve_secret(&self, secret_ref: &SecretRef) -> Result<String, AcpError> {
        (self)(secret_ref)
    }
}

/// Default no-secrets resolver for environments where secrets are not yet wired.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoSecretsResolver;

impl SecretResolver for NoSecretsResolver {
    fn resolve_secret(&self, secret_ref: &SecretRef) -> Result<String, AcpError> {
        Err(AcpError::SecretResolutionFailed {
            secret_ref: secret_ref.to_string(),
            diagnostic: "no secret resolver configured in composition".to_string(),
        })
    }
}

/// A resolved launch configuration ready for process execution.
///
/// Environment variable values are redacted in `Debug` to prevent
/// credential leakage into diagnostic logs.
#[derive(Clone, Eq, PartialEq)]
pub struct ResolvedLaunchConfig {
    /// Executable program path.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Resolved environment map.
    pub env: BTreeMap<String, String>,
}

impl ResolvedLaunchConfig {
    /// Direct constructor for pre-resolved launch configurations (e.g. in tests).
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::InvalidConfig`] if program or bounds are invalid.
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        working_dir: Option<String>,
        env: BTreeMap<String, String>,
    ) -> Result<Self, AcpError> {
        let lc = LaunchConfig::new(program)?
            .with_args(args)?
            .with_working_dir(working_dir.unwrap_or_default())?;
        for (k, v) in &env {
            LaunchConfig::validate_env_key_and_value(k, v, 0)?;
        }
        Ok(Self {
            program: lc.program,
            args: lc.args,
            working_dir: lc.working_dir,
            env,
        })
    }
}

impl fmt::Debug for ResolvedLaunchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Redact all env values to prevent secret leakage in trace/debug logs.
        let redacted_env: BTreeMap<&str, &str> = self
            .env
            .keys()
            .map(|k| (k.as_str(), "[REDACTED]"))
            .collect();
        f.debug_struct("ResolvedLaunchConfig")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("working_dir", &self.working_dir)
            .field("env", &redacted_env)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_config_validates_bounds_and_structure() {
        let config = LaunchConfig::new("acp-agent")
            .unwrap()
            .with_arg("--verbose")
            .unwrap()
            .with_arg("--model=test")
            .unwrap()
            .with_working_dir("/tmp/scratch")
            .unwrap()
            .with_literal_env("RUST_LOG", "info")
            .unwrap()
            .with_secret_env("API_KEY", SecretRef::new("anthropic-main-key").unwrap())
            .unwrap();

        assert_eq!(config.program(), "acp-agent");
        assert_eq!(config.args(), &["--verbose", "--model=test"]);
        assert_eq!(config.working_dir(), Some("/tmp/scratch"));
        assert_eq!(config.env().len(), 2);
    }

    #[test]
    fn empty_or_oversized_program_is_rejected() {
        assert!(matches!(
            LaunchConfig::new(""),
            Err(AcpError::InvalidConfig { .. })
        ));
        assert!(matches!(
            LaunchConfig::new("   "),
            Err(AcpError::InvalidConfig { .. })
        ));
        let huge_prog = "a".repeat(MAX_PROGRAM_BYTES + 1);
        assert!(matches!(
            LaunchConfig::new(huge_prog),
            Err(AcpError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn secret_resolver_integration_seam_works_and_debug_redacts() {
        let config = LaunchConfig::new("agent")
            .unwrap()
            .with_literal_env("MODE", "production")
            .unwrap()
            .with_secret_env("AGENT_SECRET", SecretRef::new("sec-123").unwrap())
            .unwrap();

        let resolver = |sref: &SecretRef| -> Result<String, AcpError> {
            if sref.as_str() == "sec-123" {
                Ok("super-secret-token-value".to_owned())
            } else {
                Err(AcpError::SecretResolutionFailed {
                    secret_ref: sref.to_string(),
                    diagnostic: "not found".to_owned(),
                })
            }
        };

        let resolved_cfg = config.resolve(&resolver).unwrap();
        assert_eq!(
            resolved_cfg.env.get("AGENT_SECRET").map(String::as_str),
            Some("super-secret-token-value")
        );
        assert_eq!(
            resolved_cfg.env.get("MODE").map(String::as_str),
            Some("production")
        );

        // Verify Debug output masks secrets.
        let debug_str = format!("{resolved_cfg:?}");
        assert!(!debug_str.contains("super-secret-token-value"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn secret_resolver_trait_object_and_default_no_secrets() {
        let config = LaunchConfig::new("agent")
            .unwrap()
            .with_secret_env("AGENT_SECRET", SecretRef::new("sec-trait").unwrap())
            .unwrap();

        // Default NoSecretsResolver fails with typed error
        assert!(matches!(
            config.resolve(&NoSecretsResolver),
            Err(AcpError::SecretResolutionFailed { .. })
        ));

        // Trait object resolution works (&dyn SecretResolver)
        let resolver: Box<dyn SecretResolver> =
            Box::new(|sref: &SecretRef| -> Result<String, AcpError> {
                if sref.as_str() == "sec-trait" {
                    Ok("dyn-secret-value".to_owned())
                } else {
                    Err(AcpError::SecretResolutionFailed {
                        secret_ref: sref.to_string(),
                        diagnostic: "not found".to_owned(),
                    })
                }
            });

        let resolved_config = config.resolve(&*resolver).unwrap();
        assert_eq!(
            resolved_config.env.get("AGENT_SECRET").map(String::as_str),
            Some("dyn-secret-value")
        );
    }
}
