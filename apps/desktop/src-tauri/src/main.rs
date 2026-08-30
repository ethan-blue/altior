//! Thin Tauri shell for the Altior Desktop workbench (ADR 0008 §6).
//!
//! No plugin commands, no custom IPC, no global Tauri API: the webview
//! loads the Vite bundle and everything product-facing flows through the
//! Altior IPC contracts, not Tauri APIs. Capability creep fails the
//! static allowlist test in `src/platform/tauri/`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("the Altior shell window starts");
}
