// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Manual verification app for the Tauri Power Monitor community plugin.
//!
//! Run with `cargo tauri dev` from `examples/api` (after building the
//! frontend with `npm install && npm run build` in that folder) and watch
//! the webview: unplug the charger, let the battery discharge, suspend and
//! resume the machine.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_power_monitor::init())
        .run(tauri::generate_context!())
        .expect("error while running the power monitor example");
}
