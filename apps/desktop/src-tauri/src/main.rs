//! Thin Tauri shell for the Altior Desktop workbench (ADR 0008 §6).
//!
//! No plugin commands, no custom IPC, no global Tauri API: the webview
//! loads the Vite bundle and everything product-facing flows through the
//! Altior IPC contracts, not Tauri APIs. Capability creep fails the
//! static allowlist test in `src/platform/tauri/`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use altior_desktop_shell::commands::*;
use altior_desktop_shell::state::AppIpcState;
use tauri::{Emitter, Manager};

fn main() {
    let ipc_state = AppIpcState::new_default();

    tauri::Builder::default()
        .manage(ipc_state)
        .setup(|app| {
            let handle = app.handle().clone();
            let state: tauri::State<AppIpcState> = app.state();
            state.subscribe_events(move |event| {
                let _ = handle.emit("core_event", &event);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_handshake,
            core_command,
            core_reconnect,
            core_close,
            core_status
        ])
        .run(tauri::generate_context!())
        .expect("the Altior shell window starts");
}
