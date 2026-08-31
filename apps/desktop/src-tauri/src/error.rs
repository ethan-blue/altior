//! Typed serializable error types for the Tauri IPC bridge and Core client (P1.3 / ADR 0006).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Serialized failure kinds for the Tauri command boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum BridgeError {
    /// Core process or IPC transport endpoint is unavailable.
    #[error("Core transport unavailable: {0}")]
    TransportUnavailable(String),

    /// Launch capability token was rejected or missing.
    #[error("Authentication rejected: {0}")]
    AuthenticationFailed(String),

    /// Discovery information was stale or referred to a terminated Core launch.
    #[error("Stale discovery: {0}")]
    StaleDiscovery(String),

    /// Handshake negotiation failed (e.g. no common protocol version).
    #[error("Handshake negotiation failed: {0}")]
    HandshakeFailed(String),

    /// Core rejected command or returned execution error.
    #[error("Command error ({code}): {message}")]
    CommandFailed {
        /// Stable error code.
        code: String,
        /// Bounded human-readable error description.
        message: String,
    },

    /// The bridge connection has been closed by the client.
    #[error("Bridge connection is closed")]
    ConnectionClosed,

    /// Failed to spawn detached Core process.
    #[error("Failed to spawn Core process: {0}")]
    SpawnFailed(String),

    /// Envelope encoding, decoding, or JSON serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Operation or connection probe timed out.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Internal error in bridge state machine.
    #[error("Internal bridge error: {0}")]
    Internal(String),
}

impl From<altior_ipc::IpcError> for BridgeError {
    fn from(err: altior_ipc::IpcError) -> Self {
        match err {
            altior_ipc::IpcError::AuthenticationRejected => {
                Self::AuthenticationFailed("launch token rejected by Core".to_string())
            }
            altior_ipc::IpcError::EndpointUnavailable { endpoint } => {
                Self::TransportUnavailable(format!("no live Core at endpoint: {endpoint}"))
            }
            altior_ipc::IpcError::StaleEndpoint { expected, found } => Self::StaleDiscovery(
                format!("stale Core endpoint: expected {expected}, found {found}"),
            ),
            altior_ipc::IpcError::Protocol { source } => Self::HandshakeFailed(source.to_string()),
            altior_ipc::IpcError::FrameTooLarge {
                size_bytes,
                limit_bytes,
            } => Self::Serialization(format!(
                "frame size {size_bytes} exceeds limit {limit_bytes}"
            )),
            altior_ipc::IpcError::FrameNotUtf8 => {
                Self::Serialization("frame is not valid UTF-8".to_string())
            }
            altior_ipc::IpcError::SessionOrder { attempted } => {
                Self::Internal(format!("session order violation: {attempted}"))
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<altior_protocol::ProtocolError> for BridgeError {
    fn from(err: altior_protocol::ProtocolError) -> Self {
        Self::HandshakeFailed(err.to_string())
    }
}

impl From<serde_json::Error> for BridgeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

/// Typed error occurring during Core discovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Error)]
pub enum DiscoveryError {
    /// Token file was not found.
    #[error("Discovery token file not found at {0}")]
    NotFound(String),

    /// Token file could not be read or decoded.
    #[error("Failed to parse discovery token file: {0}")]
    DecodeFailed(String),

    /// Endpoint derivation failed.
    #[error("Failed to derive endpoint: {0}")]
    EndpointDerivation(String),

    /// I/O error during discovery.
    #[error("I/O error during discovery: {0}")]
    Io(String),
}

/// Typed error occurring during Core process spawning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Error)]
pub enum SpawnError {
    /// Core executable binary not found.
    #[error("Core executable binary not found: {0}")]
    BinaryNotFound(String),

    /// Detached spawn OS call failed.
    #[error("OS failed to spawn detached process: {0}")]
    ProcessSpawn(String),

    /// Environment preparation failed.
    #[error("Environment error: {0}")]
    Environment(String),
}

/// Typed error occurring during transport read/write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Error)]
pub enum TransportError {
    /// Transport channel is disconnected.
    #[error("Transport disconnected")]
    Disconnected,

    /// Frame encoding or length prefix error.
    #[error("Frame encoding/decoding error: {0}")]
    Frame(String),

    /// Connection failed.
    #[error("Connection error: {0}")]
    Connect(String),

    /// Transport I/O failure.
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<TransportError> for BridgeError {
    fn from(err: TransportError) -> Self {
        match err {
            TransportError::Disconnected => Self::ConnectionClosed,
            TransportError::Connect(msg) => Self::TransportUnavailable(msg),
            TransportError::Frame(msg) | TransportError::Io(msg) => Self::Internal(msg),
        }
    }
}
