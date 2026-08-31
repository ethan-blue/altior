//! Altior Desktop Tauri backend: Core IPC client, process supervision, and command bridge (ADR 0008 §6).

pub mod adapter;
pub mod commands;
pub mod discovery;
pub mod error;
pub mod manager;
pub mod session;
pub mod spawner;
pub mod state;

#[cfg(test)]
mod tests;

pub use adapter::{
    CoreChannel, CoreConnector, MockChannelState, MockCoreChannel, MockCoreConnector,
};
pub use commands::{core_close, core_command, core_handshake, core_reconnect, core_status};
pub use discovery::{CoreDiscovery, FsCoreDiscovery};
pub use error::BridgeError;
pub use manager::SpawnOrAttachManager;
pub use session::BridgeSession;
pub use spawner::{CoreSpawner, DetachedCoreSpawner};
pub use state::{AppIpcState, ReconnectCursor, TransportStatus};
