//! Linux backend.
//!
//! Queries and events are served by **UPower** over D-Bus (through `zbus`).
//! Suspend/resume events come from **systemd-logind**'s `PrepareForSleep`
//! signal. When D-Bus or UPower are unavailable (e.g. minimal headless
//! systems), queries fall back to reading the `/sys/class/power_supply`
//! kernel ABI, and events are unavailable.
//!
//! The listener runs on a dedicated thread: `async_io::block_on` drives a
//! `select!` loop over the signal streams plus an `event_listener::Event`
//! stop signal. The zbus blocking `SignalIterator` has no documented
//! cancellation mechanism, which is why the async API is used here instead —
//! it allows a fully deterministic shutdown.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use event_listener::Event;
use futures_util::future::FutureExt;
use futures_util::select;
use futures_util::stream::StreamExt;

use super::{charging_from_upower_state, power_source_from_upower, PowerBackend};
use crate::error::{Error, Result};
use crate::events::{NativeEvent, NativeEventSender};
use crate::models::{BatteryInfo, PowerSource, PowerState};

/// `org.freedesktop.UPower.Device.Type` value for batteries.
const UPOWER_DEVICE_TYPE_BATTERY: u32 = 2;

/// The running Linux event listener.
struct LinuxListener {
    stop: Arc<Event>,
    thread: Option<JoinHandle<()>>,
}

/// Linux backend handle.
pub(crate) struct LinuxPowerBackend {
    /// Lazily created D-Bus system connection shared by queries and the
    /// listener. A failed attempt is *not* cached: a connection error may be
    /// transient (e.g. the bus restarting), so each query retries.
    connection: Mutex<Option<Arc<zbus::Connection>>>,
    /// Root of the sysfs power supply tree; configurable for tests.
    sysfs_root: PathBuf,
    listener: Mutex<Option<LinuxListener>>,
}

impl LinuxPowerBackend {
    pub(crate) fn new() -> Self {
        Self {
            connection: Mutex::new(None),
            sysfs_root: PathBuf::from("/sys/class/power_supply"),
            listener: Mutex::new(None),
        }
    }

    fn connection(&self) -> Result<Arc<zbus::Connection>> {
        let mut connection = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(connection) = connection.as_ref() {
            return Ok(connection.clone());
        }
        let conn = async_io::block_on(zbus::Connection::system())
            .map_err(|e| Error::query(format!("failed to connect to the system D-Bus: {e}")))?;
        let conn = Arc::new(conn);
        *connection = Some(conn.clone());
        Ok(conn)
    }
}

impl Default for LinuxPowerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerBackend for LinuxPowerBackend {
    fn state(&self) -> Result<PowerState> {
        let connection = self.connection();
        if let Ok(connection) = connection {
            match async_io::block_on(query_upower_state(&connection)) {
                Ok(state) => return Ok(state),
                // The connection works but UPower does not (missing service,
                // crashed daemon, …): fall through to sysfs.
                Err(e) => log::debug!("UPower query failed, falling back to sysfs: {e}"),
            }
        }
        super::read_state_from_sysfs(&self.sysfs_root)
    }

    fn start(&self, events: NativeEventSender) -> Result<()> {
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        if listener.is_some() {
            return Ok(());
        }

        // A missing D-Bus means no events; the caller logs a warning and the
        // plugin keeps serving queries through the sysfs fallback.
        let connection = self.connection()?;

        let (ready_tx, ready_rx) = channel::<std::result::Result<(), String>>();
        let stop = Arc::new(Event::new());
        let thread_stop = stop.clone();

        let thread = std::thread::Builder::new()
            .name("tauri-plugin-power-monitor".into())
            .spawn(move || {
                async_io::block_on(listen(connection, thread_stop, events, ready_tx));
            })
            .map_err(Error::listener)?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                *listener = Some(LinuxListener {
                    stop,
                    thread: Some(thread),
                });
                Ok(())
            }
            // The thread reported a setup failure (or died): join it before
            // surfacing the error so no thread is left behind.
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(Error::listener(e))
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::listener(
                    "the power event listener thread exited unexpectedly",
                ))
            }
        }
    }

    fn stop(&self) {
        let mut listener = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut listener) = listener.take() {
            // Wakes the stop signal inside the select! loop; the thread then
            // drops all proxies, streams and the D-Bus connection before
            // exiting, which is what the join below waits for.
            listener.stop.notify(1);
            if let Some(thread) = listener.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Queries UPower's aggregated `DisplayDevice` plus the daemon-level
/// `OnBattery` property.
async fn query_upower_state(connection: &zbus::Connection) -> Result<PowerState> {
    let device = zbus::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower/devices/DisplayDevice",
        "org.freedesktop.UPower.Device",
    )
    .await
    .map_err(Error::query)?;

    let device_type: u32 = device.get_property("Type").await.map_err(Error::query)?;
    let is_present: bool = device
        .get_property("IsPresent")
        .await
        .map_err(Error::query)?;

    let mut battery_state: Option<u32> = None;
    let battery = if is_present && device_type == UPOWER_DEVICE_TYPE_BATTERY {
        let percentage: f64 = device
            .get_property("Percentage")
            .await
            .map_err(Error::query)?;
        let state: u32 = device.get_property("State").await.map_err(Error::query)?;
        let time_to_empty: i64 = device
            .get_property("TimeToEmpty")
            .await
            .map_err(Error::query)?;
        let time_to_full: i64 = device
            .get_property("TimeToFull")
            .await
            .map_err(Error::query)?;

        battery_state = Some(state);
        Some(BatteryInfo {
            is_present: true,
            percentage: upower_percentage(percentage),
            is_charging: charging_from_upower_state(state),
            time_to_empty: upower_seconds(time_to_empty),
            time_to_full: upower_seconds(time_to_full),
        })
    } else {
        None
    };

    // The daemon-level OnBattery property is the canonical power source
    // signal. Very old UPower versions that don't expose it fall back to
    // deriving the source from the battery state.
    let daemon = zbus::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await
    .map_err(Error::query)?;

    let power_source = match daemon.get_property::<bool>("OnBattery").await {
        Ok(on_battery) => power_source_from_upower(on_battery),
        Err(e) => {
            log::debug!("UPower OnBattery unavailable ({e}); deriving from device state");
            match battery_state {
                Some(2) => PowerSource::Battery, // discharging
                _ => PowerSource::Ac,
            }
        }
    };

    Ok(PowerState {
        power_source,
        battery,
    })
}

/// UPower reports `-1.0` when the percentage is unknown.
fn upower_percentage(percentage: f64) -> Option<u8> {
    if percentage < 0.0 {
        None
    } else {
        Some(percentage.round().clamp(0.0, 100.0) as u8)
    }
}

/// UPower reports `0` when a time estimate is unknown.
fn upower_seconds(seconds: i64) -> Option<u64> {
    u64::try_from(seconds).ok().filter(|seconds| *seconds > 0)
}

/// Subscribes to all relevant signals, signals readiness, then services the
/// loop until the stop event fires.
async fn listen(
    connection: Arc<zbus::Connection>,
    stop: Arc<Event>,
    events: NativeEventSender,
    ready: Sender<std::result::Result<(), String>>,
) {
    let subscriptions = async {
        let display_device_properties = zbus::Proxy::new(
            &connection,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.DBus.Properties",
        )
        .await
        .map_err(|e| e.to_string())?;

        let daemon_properties = zbus::Proxy::new(
            &connection,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.DBus.Properties",
        )
        .await
        .map_err(|e| e.to_string())?;

        let daemon = zbus::Proxy::new(
            &connection,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
        )
        .await
        .map_err(|e| e.to_string())?;

        // The legacy `Changed()` signal was removed in UPower 1.90;
        // `PropertiesChanged` is the current mechanism.
        let display_changed = display_device_properties
            .receive_signal("PropertiesChanged")
            .await
            .map_err(|e| e.to_string())?;

        let daemon_changed = daemon_properties
            .receive_signal("PropertiesChanged")
            .await
            .map_err(|e| e.to_string())?;

        let device_added = daemon
            .receive_signal("DeviceAdded")
            .await
            .map_err(|e| e.to_string())?;

        let device_removed = daemon
            .receive_signal("DeviceRemoved")
            .await
            .map_err(|e| e.to_string())?;

        let login_manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        .map_err(|e| e.to_string())?;

        // On non-systemd systems this match rule simply never fires: no
        // error, just no suspend/resume events (documented limitation).
        let prepare_for_sleep = login_manager
            .receive_signal("PrepareForSleep")
            .await
            .map_err(|e| e.to_string())?;

        Ok::<_, String>((
            display_changed,
            daemon_changed,
            device_added,
            device_removed,
            prepare_for_sleep,
        ))
    };

    let (
        mut display_changed,
        mut daemon_changed,
        mut device_added,
        mut device_removed,
        mut prepare_for_sleep,
    ) = match subscriptions.await {
        Ok(streams) => streams,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let _ = ready.send(Ok(()));

    // Registered from the moment of creation: a stop notification is never
    // lost even if it arrives while another branch of the loop runs.
    let mut stop_listener = stop.listen().fuse();

    loop {
        select! {
            _ = stop_listener => break,

            message = display_changed.next() => {
                if message.is_none() { break; }
                let _ = events.send(NativeEvent::Requery);
            }
            message = daemon_changed.next() => {
                if message.is_none() { break; }
                let _ = events.send(NativeEvent::Requery);
            }
            message = device_added.next() => {
                if message.is_none() { break; }
                let _ = events.send(NativeEvent::Requery);
            }
            message = device_removed.next() => {
                if message.is_none() { break; }
                let _ = events.send(NativeEvent::Requery);
            }
            message = prepare_for_sleep.next() => {
                // PrepareForSleep(boolean start): true right before the
                // system sleeps, false right after it wakes. `None` means the
                // stream ended (bus connection gone): stop listening.
                match message {
                    Some(message) => match message.body().deserialize::<(bool,)>() {
                        Ok((true,)) => { let _ = events.send(NativeEvent::Suspend); }
                        Ok((false,)) => { let _ = events.send(NativeEvent::Resume); }
                        Err(e) => log::debug!("invalid PrepareForSleep payload: {e}"),
                    },
                    None => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upower_percentage_maps_unknown_negative() {
        assert_eq!(upower_percentage(-1.0), None);
        assert_eq!(upower_percentage(0.0), Some(0));
        assert_eq!(upower_percentage(87.4), Some(87));
        assert_eq!(upower_percentage(87.5), Some(88));
        assert_eq!(upower_percentage(100.0), Some(100));
        assert_eq!(upower_percentage(120.0), Some(100));
    }

    #[test]
    fn upower_seconds_maps_unknown_and_invalid() {
        assert_eq!(upower_seconds(0), None);
        assert_eq!(upower_seconds(-1), None);
        assert_eq!(upower_seconds(3600), Some(3600));
    }
}
