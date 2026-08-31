//! Tauri command bridge handlers matching frontend `TauriCoreTransport` contracts (P1.3).

use altior_protocol::{CommandEnvelope, NegotiatedHandshake};
use serde_json::Value;
use tauri::State;

use crate::error::BridgeError;
use crate::state::{AppIpcState, ReconnectCursor};

/// Connects to Core, performs version negotiation & authentication, and returns handshake.
#[tauri::command]
pub async fn core_handshake(
    state: State<'_, AppIpcState>,
    client: Option<String>,
) -> Result<NegotiatedHandshake, BridgeError> {
    state.handshake(client).await
}

/// Dispatches a command envelope to Core and returns the result.
#[tauri::command]
pub async fn core_command(
    state: State<'_, AppIpcState>,
    envelope: CommandEnvelope,
) -> Result<Value, BridgeError> {
    state.command(envelope).await
}

/// Reconnects with an optional sequence cursor and resumes event streaming.
#[tauri::command]
pub async fn core_reconnect(
    state: State<'_, AppIpcState>,
    cursor: Option<ReconnectCursor>,
) -> Result<NegotiatedHandshake, BridgeError> {
    state.reconnect(cursor).await
}

/// Gracefully closes the Core IPC connection without killing the Core process.
#[tauri::command]
pub async fn core_close(state: State<'_, AppIpcState>) -> Result<(), BridgeError> {
    state.close().await
}

/// Queries current transport lifecycle status.
#[tauri::command]
pub async fn core_status(state: State<'_, AppIpcState>) -> Result<String, BridgeError> {
    Ok(state.status_string())
}
