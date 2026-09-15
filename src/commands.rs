//! IPC commands.

use tauri::{Runtime, State};

use crate::{models::PowerState, PowerMonitor, Result};

/// Returns the current power state of the system (power source and battery
/// information).
///
/// The JavaScript binding is `getPowerState()` from
/// `@marrionesa/plugin-power-monitor` (or `window.__TAURI__.powerMonitor`).
#[tauri::command]
pub(crate) fn get_power_state<R: Runtime>(
    _app: tauri::AppHandle<R>,
    monitor: State<'_, PowerMonitor<R>>,
) -> Result<PowerState> {
    monitor.state()
}
