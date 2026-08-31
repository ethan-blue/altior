//! Core process spawner: detached process launch and path resolution (ADR 0006).
//!
//! Process model:
//! - Core runs detached from Desktop UI (`--daemon` mode).
//! - Windows creation flags ensure the child process is not killed when Desktop closes.
//! - Tauri sidecar lifecycle is explicitly NOT used (ADR 0006 §1 / ADR 0008 §6).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::error::SpawnError;

/// Trait for querying and spawning the Core process in detached mode.
pub trait CoreSpawner: Send + Sync {
    /// Checks whether the Core process appears to be actively running.
    fn is_running(&self) -> bool;

    /// Spawns `altior-core` in detached mode with the given arguments.
    /// Returns the OS process ID on success.
    fn spawn_detached(&self, args: &[String]) -> Result<u32, SpawnError>;

    /// Resolves the absolute path to the Core binary.
    fn resolve_binary_path(&self) -> Result<PathBuf, SpawnError>;
}

/// Production Core spawner launching `altior-core.exe` (or `altior-core`) detached.
#[derive(Debug, Default)]
pub struct DetachedCoreSpawner {
    custom_binary_path: Option<PathBuf>,
}

impl DetachedCoreSpawner {
    /// Creates a new detached spawner using automatic binary path discovery.
    #[must_use]
    pub fn new() -> Self {
        Self {
            custom_binary_path: std::env::var("ALTIOR_CORE_BIN").ok().map(PathBuf::from),
        }
    }

    /// Creates a spawner with an explicitly specified binary path.
    #[must_use]
    pub fn with_binary_path(path: PathBuf) -> Self {
        Self {
            custom_binary_path: Some(path),
        }
    }

    /// Finds candidate paths for the Core executable.
    fn candidate_paths() -> Vec<PathBuf> {
        let binary_name = if cfg!(windows) {
            "altior-core.exe"
        } else {
            "altior-core"
        };

        let mut candidates = Vec::new();

        // 1. Current executable directory (production packaged layout)
        if let Ok(current_exe) = std::env::current_exe() {
            if let Some(parent) = current_exe.parent() {
                candidates.push(parent.join(binary_name));
            }
        }

        // 2. Relative cargo target paths (development layout)
        let dev_relatives = [
            format!("./target/debug/{binary_name}"),
            format!("./target/release/{binary_name}"),
            format!("../../target/debug/{binary_name}"),
            format!("../../target/release/{binary_name}"),
            format!("../../../target/debug/{binary_name}"),
            format!("../../../target/release/{binary_name}"),
        ];
        for rel in dev_relatives {
            candidates.push(PathBuf::from(rel));
        }

        // 3. Current working directory
        candidates.push(PathBuf::from(binary_name));

        candidates
    }
}

impl CoreSpawner for DetachedCoreSpawner {
    fn is_running(&self) -> bool {
        // Detailed check is delegated to discovery probe and IPC handshake
        false
    }

    fn resolve_binary_path(&self) -> Result<PathBuf, SpawnError> {
        if let Some(ref path) = self.custom_binary_path {
            if path.exists() {
                return Ok(path.clone());
            }
            return Err(SpawnError::BinaryNotFound(format!(
                "Custom binary path does not exist: {}",
                path.display()
            )));
        }

        for candidate in Self::candidate_paths() {
            if candidate.exists() {
                if let Ok(canonical) = candidate.canonicalize() {
                    return Ok(canonical);
                }
                return Ok(candidate);
            }
        }

        // Check if executable is in PATH
        let binary_name = if cfg!(windows) {
            "altior-core.exe"
        } else {
            "altior-core"
        };
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let p = dir.join(binary_name);
                if p.exists() {
                    return Ok(p);
                }
            }
        }

        Err(SpawnError::BinaryNotFound(format!(
            "Could not locate '{binary_name}' in standard paths or PATH"
        )))
    }

    fn spawn_detached(&self, args: &[String]) -> Result<u32, SpawnError> {
        let binary = self.resolve_binary_path()?;
        let mut cmd = Command::new(&binary);

        cmd.args(args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Windows creation flags:
            // DETACHED_PROCESS (0x00000008): process is detached from parent console
            // CREATE_NEW_PROCESS_GROUP (0x00000200): creates a new process group
            // CREATE_NO_WINDOW (0x08000000): suppresses console window creation
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }

        let child = cmd
            .spawn()
            .map_err(|e| SpawnError::ProcessSpawn(format!("{}: {}", binary.display(), e)))?;

        let pid = child.id();
        // Dropping `child` does NOT kill the spawned process.
        // Core continues running detached independently of Tauri shell lifetime.
        Ok(pid)
    }
}
