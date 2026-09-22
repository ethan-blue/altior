//! Spawn-or-attach manager coordinating discovery, detached spawning, and handshake negotiation (ADR 0006).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use altior_ipc::encode_frame;
use altior_protocol::{negotiate, CoreGreeting, CoreHello, EnvelopeLimits, NegotiatedHandshake};

use crate::adapter::{CoreChannel, CoreConnector};
use crate::discovery::CoreDiscovery;
use crate::error::BridgeError;
use crate::session::BridgeSession;
use crate::spawner::CoreSpawner;

/// Maximum time to wait for newly spawned Core process to initialize and publish its token file.
const SPAWN_ATTACH_TIMEOUT: Duration = Duration::from_millis(4000);
/// Interval between discovery polls when waiting for Core startup.
const POLL_INTERVAL: Duration = Duration::from_millis(60);

/// Manages attaching to an existing Core instance or detached-spawning a new one.
pub struct SpawnOrAttachManager {
    discovery: Arc<dyn CoreDiscovery>,
    spawner: Arc<dyn CoreSpawner>,
    connector: Arc<dyn CoreConnector>,
    session: Arc<BridgeSession>,
    data_dir: Result<PathBuf, DataDirError>,
    limits: EnvelopeLimits,
}

impl SpawnOrAttachManager {
    /// Creates a new manager with explicit dependency injection.
    pub fn new(
        discovery: Arc<dyn CoreDiscovery>,
        spawner: Arc<dyn CoreSpawner>,
        connector: Arc<dyn CoreConnector>,
        session: Arc<BridgeSession>,
    ) -> Self {
        Self {
            discovery,
            spawner,
            connector,
            session,
            data_dir: default_data_dir(),
            limits: EnvelopeLimits::default(),
        }
    }

    /// Overrides the data directory passed to Core when detached-spawned.
    #[must_use]
    pub fn with_data_dir(mut self, data_dir: PathBuf) -> Self {
        self.data_dir = validate_data_dir(&data_dir).map(|()| data_dir);
        self
    }

    /// Returns the data directory configured on this manager, or an error if unresolved.
    pub fn data_dir(&self) -> Result<&Path, &DataDirError> {
        self.data_dir.as_deref()
    }

    /// Primary entry point: attempts to attach to an active Core instance, or spawns Core detached.
    pub fn attach_or_spawn(
        &self,
    ) -> Result<(Box<dyn CoreChannel>, NegotiatedHandshake, CoreGreeting), BridgeError> {
        // Step 1: Probe existing discovery
        if let Ok(Some(creds)) = self.discovery.discover_credentials() {
            if let Ok(endpoint) = self.discovery.resolve_endpoint() {
                match self.connector.connect(&endpoint) {
                    Ok(channel) => {
                        match self.perform_handshake(&*channel, &creds.launch_token) {
                            Ok((negotiated, greeting)) => {
                                return Ok((channel, negotiated, greeting));
                            }
                            Err(_) => {
                                // Handshake failed or token was stale; invalidate discovery
                                let _ = self.discovery.invalidate_stale_token();
                            }
                        }
                    }
                    Err(_) => {
                        // Connection failed; endpoint is dead/stale
                        let _ = self.discovery.invalidate_stale_token();
                    }
                }
            }
        }

        // Step 2: Spawn new Core instance in detached mode
        let data_dir = self
            .data_dir
            .as_ref()
            .map_err(|e| BridgeError::SpawnFailed(format!("Invalid data directory: {e}")))?;
        create_secure_data_dir(data_dir).map_err(|e| {
            BridgeError::SpawnFailed(format!("Failed to create data directory: {e}"))
        })?;
        let daemon_args = vec![
            "--daemon".to_string(),
            "--data-dir".to_string(),
            data_dir.to_string_lossy().to_string(),
        ];
        self.spawner
            .spawn_detached(&daemon_args)
            .map_err(|e| BridgeError::SpawnFailed(e.to_string()))?;

        // Step 3: Poll discovery and attach to the newly spawned Core
        let start = Instant::now();
        while start.elapsed() < SPAWN_ATTACH_TIMEOUT {
            std::thread::sleep(POLL_INTERVAL);

            let creds = match self.discovery.discover_credentials() {
                Ok(Some(creds)) => creds,
                _ => continue,
            };

            let endpoint = match self.discovery.resolve_endpoint() {
                Ok(ep) => ep,
                _ => continue,
            };

            let channel = match self.connector.connect(&endpoint) {
                Ok(ch) => ch,
                _ => continue,
            };

            match self.perform_handshake(&*channel, &creds.launch_token) {
                Ok((negotiated, greeting)) => {
                    return Ok((channel, negotiated, greeting));
                }
                Err(_) => {
                    continue;
                }
            }
        }

        Err(BridgeError::TransportUnavailable(
            "Timed out waiting for Core process to start, publish token, and accept handshake"
                .to_string(),
        ))
    }

    /// Performs the initial hello / greet exchange over an established channel.
    pub fn perform_handshake(
        &self,
        channel: &dyn CoreChannel,
        token: &altior_protocol::LaunchToken,
    ) -> Result<(NegotiatedHandshake, CoreGreeting), BridgeError> {
        let hello = self.session.create_hello(token);
        let hello_json =
            serde_json::to_string(&hello).map_err(|e| BridgeError::Serialization(e.to_string()))?;
        let hello_frame = encode_frame(&hello_json).map_err(BridgeError::from)?;

        channel
            .send_frame(&hello_frame)
            .map_err(BridgeError::from)?;

        // Read CoreHello response frame
        let core_hello_raw = channel
            .read_frame(Some(Duration::from_millis(3000)))
            .map_err(BridgeError::from)?;

        let core_hello: CoreHello = serde_json::from_slice(&core_hello_raw)
            .map_err(|e| BridgeError::HandshakeFailed(format!("Failed to parse CoreHello: {e}")))?;

        let negotiated = negotiate(&hello, &core_hello)
            .map_err(|e| BridgeError::HandshakeFailed(e.to_string()))?;

        // Read CoreGreeting response frame
        let greeting_raw = channel
            .read_frame(Some(Duration::from_millis(3000)))
            .map_err(BridgeError::from)?;

        let greeting: CoreGreeting = serde_json::from_slice(&greeting_raw).map_err(|e| {
            BridgeError::HandshakeFailed(format!("Failed to parse CoreGreeting: {e}"))
        })?;

        // Validate greeting epoch with session
        self.session.accept_greeting(&greeting, &negotiated)?;

        Ok((negotiated, greeting))
    }

    /// Returns a reference to the bridge session.
    #[must_use]
    pub fn session(&self) -> &Arc<BridgeSession> {
        &self.session
    }

    /// Returns envelope bounds limits.
    #[must_use]
    pub fn limits(&self) -> &EnvelopeLimits {
        &self.limits
    }
}

/// Errors that can occur when resolving or validating the application data directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataDirError {
    /// Provided data directory path is relative, but an absolute path is required.
    RelativePath(PathBuf),
    /// Path contains parent directory traversal (`..`).
    ParentDirTraversal(PathBuf),
    /// Could not securely resolve a platform per-user directory from environment.
    ResolutionFailed,
}

impl std::fmt::Display for DataDirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RelativePath(p) => {
                write!(f, "data directory path must be absolute: '{}'", p.display())
            }
            Self::ParentDirTraversal(p) => {
                write!(
                    f,
                    "data directory path must not contain parent traversal ('..'): '{}'",
                    p.display()
                )
            }
            Self::ResolutionFailed => {
                write!(
                    f,
                    "cannot securely resolve per-user data directory: environment missing or insecure"
                )
            }
        }
    }
}

impl std::error::Error for DataDirError {}

/// Checks if a path is absolute across Unix and Windows conventions deterministically.
#[must_use]
pub fn is_absolute_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.is_empty() {
        return false;
    }
    if path.is_absolute() {
        return true;
    }
    if s.starts_with('/') || s.starts_with('\\') {
        return true;
    }
    let bytes = s.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    false
}

/// Checks if a path contains parent directory traversal (`..`).
#[must_use]
pub fn has_parent_traversal(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.split(['/', '\\']).any(|part| part == "..")
}

/// Validates that a data directory path is absolute and free of parent traversal (`..`).
///
/// # Errors
///
/// Returns [`DataDirError::RelativePath`] if relative, or [`DataDirError::ParentDirTraversal`] if it contains `..`.
pub fn validate_data_dir(path: &Path) -> Result<(), DataDirError> {
    if !is_absolute_path(path) {
        return Err(DataDirError::RelativePath(path.to_path_buf()));
    }
    if has_parent_traversal(path) {
        return Err(DataDirError::ParentDirTraversal(path.to_path_buf()));
    }
    Ok(())
}

/// Creates directory `path` and any parent directories, ensuring Unix permissions are mode 0700 (rwx------).
///
/// # Errors
///
/// Returns an [`std::io::Error`] if directory creation or permission adjustment fails.
pub fn create_secure_data_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        use std::os::unix::fs::PermissionsExt;

        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        builder.mode(0o700);
        builder.create(path)?;

        if let Ok(metadata) = std::fs::metadata(path) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o777 != 0o700 {
                perms.set_mode(0o700);
                let _ = std::fs::set_permissions(path, perms);
            }
        }

        if let Some(parent) = path.parent() {
            if let Ok(metadata) = std::fs::metadata(parent) {
                let mut perms = metadata.permissions();
                if perms.mode() & 0o777 != 0o700
                    && parent.file_name().map_or(false, |n| n == "altior")
                {
                    perms.set_mode(0o700);
                    let _ = std::fs::set_permissions(parent, perms);
                }
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// Environment inputs for deriving the per-user application data directory in Desktop shell.
///
/// Kept as an explicit data structure so that resolution logic is 100% deterministic
/// and independent of real machine state in tests.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct DataDirEnv {
    /// Explicit override (e.g. `ALTIOR_DATA_DIR`).
    pub altior_data_dir: Option<String>,
    /// Windows local application data (`%LOCALAPPDATA%`).
    pub local_app_data: Option<String>,
    /// Unix/Linux XDG data directory (`$XDG_DATA_HOME`).
    pub xdg_data_home: Option<String>,
    /// User home directory (`$HOME` on Unix/macOS, `%USERPROFILE%` on Windows).
    pub home: Option<String>,
    /// Unix/Linux user runtime directory (`$XDG_RUNTIME_DIR`), provably user-isolated (0700).
    pub xdg_runtime_dir: Option<String>,
    /// Temporary fallback directory.
    pub temp_dir: Option<String>,
}

impl DataDirEnv {
    /// Captures the environment variables from the running process.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            altior_data_dir: std::env::var("ALTIOR_DATA_DIR").ok(),
            local_app_data: std::env::var("LOCALAPPDATA").ok(),
            xdg_data_home: std::env::var("XDG_DATA_HOME").ok(),
            home: std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .ok(),
            xdg_runtime_dir: std::env::var("XDG_RUNTIME_DIR").ok(),
            temp_dir: Some(std::env::temp_dir().to_string_lossy().to_string()),
        }
    }

    /// Resolves data directory specifically using Windows conventions.
    ///
    /// # Errors
    ///
    /// Returns [`DataDirError`] if the explicit override is invalid or if no safe
    /// platform per-user directory can be resolved.
    pub fn resolve_for_windows(&self) -> Result<PathBuf, DataDirError> {
        if let Some(ref custom) = self.altior_data_dir {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                validate_data_dir(&path)?;
                return Ok(path);
            }
        }
        if let Some(ref local_app_data) = self.local_app_data {
            let trimmed = local_app_data.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base.join("altior").join("data"));
            }
        }
        if let Some(ref home) = self.home {
            let trimmed = home.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base
                    .join("AppData")
                    .join("Local")
                    .join("altior")
                    .join("data"));
            }
        }
        Err(DataDirError::ResolutionFailed)
    }

    /// Resolves data directory specifically using macOS conventions.
    ///
    /// # Errors
    ///
    /// Returns [`DataDirError`] if the explicit override is invalid or if no safe
    /// platform per-user directory can be resolved.
    pub fn resolve_for_macos(&self) -> Result<PathBuf, DataDirError> {
        if let Some(ref custom) = self.altior_data_dir {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                validate_data_dir(&path)?;
                return Ok(path);
            }
        }
        if let Some(ref home) = self.home {
            let trimmed = home.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base
                    .join("Library")
                    .join("Application Support")
                    .join("altior")
                    .join("data"));
            }
        }
        Err(DataDirError::ResolutionFailed)
    }

    /// Resolves data directory specifically using Linux/Unix conventions (XDG).
    ///
    /// # Errors
    ///
    /// Returns [`DataDirError`] if the explicit override is invalid or if no safe
    /// platform per-user directory can be resolved.
    pub fn resolve_for_unix(&self) -> Result<PathBuf, DataDirError> {
        if let Some(ref custom) = self.altior_data_dir {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                validate_data_dir(&path)?;
                return Ok(path);
            }
        }
        if let Some(ref xdg) = self.xdg_data_home {
            let trimmed = xdg.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base.join("altior").join("data"));
            }
        }
        if let Some(ref home) = self.home {
            let trimmed = home.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base
                    .join(".local")
                    .join("share")
                    .join("altior")
                    .join("data"));
            }
        }
        if let Some(ref xdg_runtime) = self.xdg_runtime_dir {
            let trimmed = xdg_runtime.trim();
            if !trimmed.is_empty() {
                let base = PathBuf::from(trimmed);
                validate_data_dir(&base)?;
                return Ok(base.join("altior").join("data"));
            }
        }
        Err(DataDirError::ResolutionFailed)
    }

    /// Resolves the platform-appropriate per-user writable application data directory.
    ///
    /// # Errors
    ///
    /// Returns [`DataDirError`] if resolution fails or input paths are invalid.
    pub fn resolve(&self) -> Result<PathBuf, DataDirError> {
        if cfg!(windows) {
            self.resolve_for_windows()
        } else if cfg!(target_os = "macos") {
            self.resolve_for_macos()
        } else {
            self.resolve_for_unix()
        }
    }
}

/// Derives the default per-user writable application data directory from process environment.
///
/// # Errors
///
/// Returns [`DataDirError`] if the per-user data directory cannot be securely resolved.
pub fn default_data_dir() -> Result<PathBuf, DataDirError> {
    DataDirEnv::from_env().resolve()
}
