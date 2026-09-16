//! OS Secret Store integration (ADR 0026).
//!
//! Provides platform-native secret storage and retrieval for ACP agent launch
//! credentials, implementing [`altior_acp::SecretResolver`].
//!
//! # Platform Implementation Status
//!
//! - **Windows**: Implemented and verified via Windows Credential Management API
//!   (`CredReadW`, `CredWriteW`, `CredDeleteW` via `windows-sys`).
//! - **macOS / Linux**: Deferred; explicitly fails closed with
//!   [`SecretStoreError::NotFound`] until native platform drivers are wired.
//!
//! # Security Invariants
//!
//! - Plaintext secrets are never held in domain models, SQLite, Markdown,
//!   diagnostics, logs, or IPC responses.
//! - Secret references (`SecretRef`) are opaque identifiers mapped to
//!   platform keychain targets.
//! - Resolution is strictly one-way: from the OS secret store into child
//!   process environments.
//! - Fail-closed: missing, corrupted, or oversized secrets abort spawn immediately.

use std::fmt;

use altior_acp::{AcpError, MAX_ENV_VALUE_BYTES, SecretRef, SecretResolver};

/// Errors originating from the OS Secret Store.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SecretStoreError {
    /// The requested secret reference was not found in the OS secret store.
    NotFound { secret_ref: String },
    /// The secret reference format or target name is invalid.
    InvalidReference { detail: String },
    /// The secret value exceeds the maximum allowable environment variable length.
    ValueTooLarge { size: usize, max: usize },
    /// A platform-specific OS API call failed.
    PlatformError { code: u32, detail: String },
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { secret_ref } => {
                write!(f, "secret '{secret_ref}' not found in OS secret store")
            }
            Self::InvalidReference { detail } => {
                write!(f, "invalid secret reference: {detail}")
            }
            Self::ValueTooLarge { size, max } => {
                write!(f, "secret value size {size} exceeds maximum {max}")
            }
            Self::PlatformError { code, detail } => {
                write!(f, "OS secret store error (code {code}): {detail}")
            }
        }
    }
}

impl std::error::Error for SecretStoreError {}

/// Canonicalizes an opaque secret reference into an OS target namespace.
///
/// Supported conventions (ADR 0026):
/// - `vault:credentials:<name>` -> `Altior/credentials/<name>`
/// - `secret://<service>/<account>` -> `Altior/<service>/<account>`
/// - `sec_<name>` -> `Altior/credentials/<name>`
/// - `<name>` -> `Altior/credentials/<name>`
///
/// # Errors
///
/// Returns [`SecretStoreError::InvalidReference`] if the reference is empty,
/// contains control characters/null bytes, or exceeds 256 bytes.
pub fn canonical_target_name(secret_ref: &str) -> Result<String, SecretStoreError> {
    let trimmed = secret_ref.trim();
    if trimmed.is_empty() {
        return Err(SecretStoreError::InvalidReference {
            detail: "secret reference cannot be empty".to_owned(),
        });
    }
    if trimmed.len() > 256 {
        return Err(SecretStoreError::InvalidReference {
            detail: format!(
                "secret reference length {} exceeds maximum 256",
                trimmed.len()
            ),
        });
    }
    if trimmed.chars().any(|c| c.is_control() || c == '\0') {
        return Err(SecretStoreError::InvalidReference {
            detail: "secret reference contains control characters or null bytes".to_owned(),
        });
    }

    if let Some(rest) = trimmed.strip_prefix("vault:credentials:") {
        Ok(format!("Altior/credentials/{rest}"))
    } else if let Some(rest) = trimmed.strip_prefix("secret://") {
        Ok(format!("Altior/{rest}"))
    } else if let Some(rest) = trimmed.strip_prefix("sec_") {
        Ok(format!("Altior/credentials/{rest}"))
    } else {
        Ok(format!("Altior/credentials/{trimmed}"))
    }
}

/// OS Secret Store driver.
///
/// Provides access to the platform's native credential store:
/// - Windows: Windows Credential Management API (implemented & verified);
/// - macOS / Linux: deferred (fails closed with [`SecretStoreError::NotFound`]).
#[derive(Clone, Default)]
pub struct OsSecretStore {
    _private: (),
}

impl fmt::Debug for OsSecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OsSecretStore").finish()
    }
}

impl OsSecretStore {
    /// Creates a new instance of the OS secret store driver.
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Retrieves a secret value by its opaque reference.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError`] if the secret does not exist, cannot be accessed,
    /// or exceeds [`MAX_ENV_VALUE_BYTES`].
    pub fn get_secret(&self, secret_ref: &str) -> Result<String, SecretStoreError> {
        let target = canonical_target_name(secret_ref)?;

        #[cfg(windows)]
        {
            platform_windows::get_credential(&target, secret_ref)
        }

        #[cfg(not(windows))]
        {
            Err(SecretStoreError::NotFound {
                secret_ref: secret_ref.to_owned(),
            })
        }
    }

    /// Stores or updates a secret in the OS secret store.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError`] if the platform write call fails.
    pub fn set_secret(&self, secret_ref: &str, secret_value: &str) -> Result<(), SecretStoreError> {
        let target = canonical_target_name(secret_ref)?;
        if secret_value.len() > MAX_ENV_VALUE_BYTES {
            return Err(SecretStoreError::ValueTooLarge {
                size: secret_value.len(),
                max: MAX_ENV_VALUE_BYTES,
            });
        }

        #[cfg(windows)]
        {
            platform_windows::set_credential(&target, secret_value)
        }

        #[cfg(not(windows))]
        {
            let _ = target;
            let _ = secret_value;
            Err(SecretStoreError::PlatformError {
                code: 1,
                detail: "OS secret store not implemented for this platform".to_owned(),
            })
        }
    }

    /// Deletes a secret from the OS secret store.
    ///
    /// If the secret does not exist, this is a no-op and succeeds idempotently.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError`] if the platform delete call fails.
    pub fn delete_secret(&self, secret_ref: &str) -> Result<(), SecretStoreError> {
        let target = canonical_target_name(secret_ref)?;

        #[cfg(windows)]
        {
            platform_windows::delete_credential(&target)
        }

        #[cfg(not(windows))]
        {
            let _ = target;
            Ok(())
        }
    }

    /// Checks if a secret exists in the OS secret store.
    #[must_use]
    pub fn has_secret(&self, secret_ref: &str) -> bool {
        self.get_secret(secret_ref).is_ok()
    }
}

impl SecretResolver for OsSecretStore {
    fn resolve_secret(&self, secret_ref: &SecretRef) -> Result<String, AcpError> {
        self.get_secret(secret_ref.as_str()).map_err(|e| match e {
            SecretStoreError::NotFound { .. } => AcpError::SecretResolutionFailed {
                secret_ref: secret_ref.to_string(),
                diagnostic: "credential not found in OS secret store".to_owned(),
            },
            SecretStoreError::ValueTooLarge { size, max } => AcpError::InvalidConfig {
                diagnostic: format!("secret value size {size} exceeds maximum {max}"),
            },
            SecretStoreError::InvalidReference { detail } => AcpError::InvalidConfig {
                diagnostic: format!("invalid secret reference: {detail}"),
            },
            SecretStoreError::PlatformError { code, detail } => AcpError::SecretResolutionFailed {
                secret_ref: secret_ref.to_string(),
                diagnostic: format!("OS secret store failure (code {code}): {detail}"),
            },
        })
    }
}

#[cfg(windows)]
mod platform_windows {
    use super::SecretStoreError;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn get_credential(
        target: &str,
        secret_ref: &str,
    ) -> Result<String, SecretStoreError> {
        let target_wide = to_wide(target);
        let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();

        let success = unsafe {
            CredReadW(
                target_wide.as_ptr(),
                CRED_TYPE_GENERIC,
                0,
                &raw mut cred_ptr,
            )
        };

        if success == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NOT_FOUND {
                return Err(SecretStoreError::NotFound {
                    secret_ref: secret_ref.to_owned(),
                });
            }
            return Err(SecretStoreError::PlatformError {
                code: error,
                detail: format!("CredReadW failed for target '{target}'"),
            });
        }

        if cred_ptr.is_null() {
            return Err(SecretStoreError::NotFound {
                secret_ref: secret_ref.to_owned(),
            });
        }

        let slice = unsafe {
            let blob = (*cred_ptr).CredentialBlob;
            let size = (*cred_ptr).CredentialBlobSize as usize;
            if blob.is_null() || size == 0 {
                CredFree(cred_ptr as *const std::ffi::c_void);
                return Ok(String::new());
            }
            std::slice::from_raw_parts(blob, size)
        };

        let result = match std::str::from_utf8(slice) {
            Ok(s) => Ok(s.to_owned()),
            Err(e) => Err(SecretStoreError::PlatformError {
                code: 0,
                detail: format!("credential blob for '{target}' is not valid UTF-8: {e}"),
            }),
        };

        unsafe {
            CredFree(cred_ptr as *const std::ffi::c_void);
        }

        result
    }

    pub(super) fn set_credential(target: &str, secret: &str) -> Result<(), SecretStoreError> {
        let target_wide = to_wide(target);
        let user_wide = to_wide("Altior");

        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_wide.as_ptr().cast_mut(),
            Comment: std::ptr::null_mut(),
            LastWritten: unsafe { std::mem::zeroed() },
            CredentialBlobSize: u32::try_from(secret.len()).unwrap_or(0),
            CredentialBlob: secret.as_bytes().as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: user_wide.as_ptr().cast_mut(),
        };

        let success = unsafe { CredWriteW(&raw const cred, 0) };
        if success == 0 {
            let error = unsafe { GetLastError() };
            return Err(SecretStoreError::PlatformError {
                code: error,
                detail: format!("CredWriteW failed for target '{target}'"),
            });
        }

        Ok(())
    }

    pub(super) fn delete_credential(target: &str) -> Result<(), SecretStoreError> {
        let target_wide = to_wide(target);
        let success = unsafe { CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if success == 0 {
            let error = unsafe { GetLastError() };
            // Deleting a non-existent credential is treated as a success (idempotent delete)
            if error == ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(SecretStoreError::PlatformError {
                code: error,
                detail: format!("CredDeleteW failed for target '{target}'"),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_target_name() {
        assert_eq!(
            canonical_target_name("vault:credentials:anthropic_key").unwrap(),
            "Altior/credentials/anthropic_key"
        );
        assert_eq!(
            canonical_target_name("secret://providers/openai").unwrap(),
            "Altior/providers/openai"
        );
        assert_eq!(
            canonical_target_name("sec_my_secret_token").unwrap(),
            "Altior/credentials/my_secret_token"
        );
        assert_eq!(
            canonical_target_name("simple_key").unwrap(),
            "Altior/credentials/simple_key"
        );

        // Rejections
        assert!(canonical_target_name("").is_err());
        assert!(canonical_target_name("   ").is_err());
        assert!(canonical_target_name("bad\x00key").is_err());
        assert!(canonical_target_name("bad\nkey").is_err());
        let oversized = "a".repeat(257);
        assert!(canonical_target_name(&oversized).is_err());
    }

    #[test]
    fn test_secret_store_debug_and_error_redaction() {
        let store = OsSecretStore::new();
        let debug_str = format!("{store:?}");
        assert_eq!(debug_str, "OsSecretStore");

        let err = SecretStoreError::NotFound {
            secret_ref: "sec_test_canary_not_found".to_owned(),
        };
        assert!(err.to_string().contains("sec_test_canary_not_found"));
        assert!(!err.to_string().contains("secret_value"));
    }

    #[test]
    fn test_missing_secret_fails_closed() {
        let store = OsSecretStore::new();
        let non_existent_ref = format!("sec_non_existent_ref_{}", 999_999_999);
        let res = store.get_secret(&non_existent_ref);
        assert!(matches!(res, Err(SecretStoreError::NotFound { .. })));

        let sref = SecretRef::new(non_existent_ref).unwrap();
        let resolve_res = store.resolve_secret(&sref);
        assert!(matches!(
            resolve_res,
            Err(AcpError::SecretResolutionFailed { .. })
        ));
    }

    #[cfg(windows)]
    struct CleanupGuard<'a> {
        store: &'a OsSecretStore,
        secret_ref: &'a str,
    }
    #[cfg(windows)]
    impl Drop for CleanupGuard<'_> {
        fn drop(&mut self) {
            let _ = self.store.delete_secret(self.secret_ref);
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_credential_manager_roundtrip_with_cleanup() {
        let store = OsSecretStore::new();
        let random_suffix = format!(
            "{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let secret_ref = format!("sec_test_altior_{random_suffix}");
        let secret_value = "super_canary_token_42_xyz";

        let _guard = CleanupGuard {
            store: &store,
            secret_ref: &secret_ref,
        };

        // Write
        store
            .set_secret(&secret_ref, secret_value)
            .expect("set secret in Windows Credential Manager");

        // Has secret
        assert!(store.has_secret(&secret_ref));

        // Read directly
        let retrieved = store
            .get_secret(&secret_ref)
            .expect("get secret from Windows Credential Manager");
        assert_eq!(retrieved, secret_value);

        // Resolve via SecretResolver
        let sref = SecretRef::new(&secret_ref).unwrap();
        let resolved = store
            .resolve_secret(&sref)
            .expect("resolve secret via SecretResolver trait");
        assert_eq!(resolved, secret_value);

        // Explicit delete
        store.delete_secret(&secret_ref).expect("delete secret");

        // Verify deleted
        assert!(!store.has_secret(&secret_ref));
        assert!(matches!(
            store.get_secret(&secret_ref),
            Err(SecretStoreError::NotFound { .. })
        ));
    }
}
