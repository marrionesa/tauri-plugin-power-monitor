//! Monitor the battery, power source, and sleep/wake state of the system.
//!
//! - Supported platforms: Windows, Linux and macOS.
//!
//! # Querying
//!
//! Use [`PowerMonitorExt::power_monitor`] from any Tauri handle to access the
//! [`PowerState`]:
//!
//! ```no_run
//! use tauri_plugin_power_monitor::{PowerMonitorExt, PowerSource};
//!
//! fn read_power_state<R: tauri::Runtime>(
//!     app: &tauri::App<R>,
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     let state = app.power_monitor().state()?;
//!     assert_eq!(state.power_source, PowerSource::Ac);
//!     Ok(())
//! }
//! ```
//!
//! # Events
//!
//! The plugin emits events through the Tauri event system for the frontend:
//!
//! | Event | Payload | When |
//! |---|---|---|
//! | `power-monitor://power-source-changed` | `PowerState` | the system switched between AC and battery |
//! | `power-monitor://battery-changed` | `PowerState` | battery percentage, charging state or presence changed |
//! | `power-monitor://suspend` | `null` | the system is about to suspend |
//! | `power-monitor://resume` | `null` | the system resumed from sleep |
//!
//! This plugin only *observes* the system; it never controls power.

#![doc(
    html_logo_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png",
    html_favicon_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png"
)]
#![cfg(not(any(target_os = "android", target_os = "ios")))]

mod commands;
mod error;
mod events;
mod models;
mod platform;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};

pub use error::{Error, Result};
pub use models::{BatteryInfo, PowerSource, PowerState};

use events::{
    NativeEvent, BATTERY_CHANGED_EVENT, POWER_SOURCE_CHANGED_EVENT, RESUME_EVENT, SUSPEND_EVENT,
};

/// The power monitor, accessible through [`PowerMonitorExt`].
pub struct PowerMonitor<R: Runtime>(Arc<PowerMonitorInner<R>>);

impl<R: Runtime> PowerMonitor<R> {
    fn new(app: AppHandle<R>) -> Self {
        let backend: Arc<dyn platform::PowerBackend> = Arc::from(platform::backend());

        // The dispatcher thread owns the native event receiver and the last
        // known state; native listeners only push lightweight signals.
        let (sender, receiver) = channel::<NativeEvent>();

        let inner = Arc::new(PowerMonitorInner {
            app: app.clone(),
            backend: backend.clone(),
            dispatcher: Mutex::new(None),
            stopped: AtomicBool::new(false),
        });

        let dispatcher = {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name("power-monitor-dispatcher".into())
                .spawn(move || inner.run(receiver))
                .map_err(|e| {
                    log::error!("failed to spawn the power monitor dispatcher: {e}");
                    e
                })
                .ok()
        };

        // A listener failure only means no events: queries keep working, so
        // log and continue instead of failing the app.
        if let Err(e) = backend.start(sender) {
            log::warn!("failed to start the native power event listeners: {e}");
        }

        *inner.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = dispatcher;

        Self(inner)
    }

    /// Queries the current power state.
    pub fn state(&self) -> Result<PowerState> {
        self.0.backend.state()
    }
}

/// Inner state shared by the plugin, the dispatcher thread and the app handle.
struct PowerMonitorInner<R: Runtime> {
    app: AppHandle<R>,
    backend: Arc<dyn platform::PowerBackend>,
    /// Handle to the dispatcher thread; joined on shutdown.
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    /// Set once shutdown has run; makes `shutdown` idempotent (explicit exit
    /// hook and `Drop` can both reach it).
    stopped: AtomicBool,
}

impl<R: Runtime> PowerMonitorInner<R> {
    /// The event dispatcher loop.
    ///
    /// Receives lightweight signals from the native listeners, re-queries the
    /// backend, diffs against the last known state and emits granular Tauri
    /// events. Exits when the channel closes, i.e. once every native listener
    /// has stopped and dropped its sender.
    fn run(&self, receiver: Receiver<NativeEvent>) {
        // Seed the last known state. A failure here (e.g. headless system
        // without power information) only means the first native change has
        // nothing to diff against; no events are fabricated.
        let mut last: Option<PowerState> = match self.backend.state() {
            Ok(state) => Some(state),
            Err(e) => {
                log::warn!("failed to read the initial power state: {e}");
                None
            }
        };

        while let Ok(event) = receiver.recv() {
            match event {
                NativeEvent::Suspend => {
                    if let Err(e) = self.app.emit(SUSPEND_EVENT, ()) {
                        log::error!("failed to emit the suspend event: {e}");
                    }
                }
                NativeEvent::Resume => {
                    if let Err(e) = self.app.emit(RESUME_EVENT, ()) {
                        log::error!("failed to emit the resume event: {e}");
                    }
                }
                NativeEvent::Requery => {
                    let next = match self.backend.state() {
                        Ok(state) => state,
                        Err(e) => {
                            log::warn!("failed to query the power state after a change: {e}");
                            continue;
                        }
                    };
                    if let Some(previous) = &last {
                        let diff = events::diff_states(previous, &next);
                        if diff.power_source_changed {
                            if let Err(e) = self.app.emit(POWER_SOURCE_CHANGED_EVENT, &next) {
                                log::error!("failed to emit the power source changed event: {e}");
                            }
                        }
                        if diff.battery_changed {
                            if let Err(e) = self.app.emit(BATTERY_CHANGED_EVENT, &next) {
                                log::error!("failed to emit the battery changed event: {e}");
                            }
                        }
                    }
                    last = Some(next);
                }
            }
        }
    }

    /// Stops the native listeners and joins every thread owned by the plugin.
    fn shutdown(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // 1. Stopping the backend joins the listener threads; their senders
        //    are dropped, which closes the channel.
        self.backend.stop();
        // 2. The dispatcher exits as soon as the channel is closed.
        let dispatcher = self
            .dispatcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(dispatcher) = dispatcher {
            let _ = dispatcher.join();
        }
    }
}

impl<R: Runtime> Drop for PowerMonitorInner<R> {
    fn drop(&mut self) {
        // Safety net: Tauri emits `RunEvent::Exit` on normal shutdowns, which
        // already ran `shutdown`; this covers any other teardown path.
        self.shutdown();
    }
}

/// Extensions to `tauri::App`, `tauri::AppHandle`, `tauri::Window`, etc.
pub trait PowerMonitorExt<R: Runtime> {
    /// The power monitor.
    fn power_monitor(&self) -> &PowerMonitor<R>;
}

impl<R: Runtime, T: Manager<R>> PowerMonitorExt<R> for T {
    fn power_monitor(&self) -> &PowerMonitor<R> {
        self.state::<PowerMonitor<R>>().inner()
    }
}

/// Initializes the plugin.
///
/// The plugin is read-only: it only queries and reports system power state,
/// and requires the `power-monitor:default` (or
/// `power-monitor:allow-get-power-state`) permission for the query command.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::new("power-monitor")
        .invoke_handler(tauri::generate_handler![commands::get_power_state])
        .setup(|app, _api| {
            app.manage(PowerMonitor::new(app.clone()));
            Ok(())
        })
        .on_event(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(monitor) = app.try_state::<PowerMonitor<R>>() {
                    monitor.0.shutdown();
                }
            }
        })
        .build()
}
