//! Platform backends.
//!
//! Each supported OS implements [`PowerBackend`]. The pure mapping helpers at
//! the bottom of this module translate raw platform values into the unified
//! model; they are intentionally kept free of any FFI so they can be
//! unit-tested on **every** platform (the same tests run on the Linux,
//! Windows and macOS CI jobs).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::mpsc::Sender;

#[cfg(any(target_os = "linux", test))]
use crate::error::Error;
use crate::error::Result;
use crate::events::NativeEvent;
#[cfg(any(target_os = "windows", target_os = "linux", test))]
use crate::models::BatteryInfo;
use crate::models::{PowerSource, PowerState};

/// Read-only access to the system power state, plus native event listeners.
///
/// Implementations must be cheap to clone or share and must make [`start`]
/// and [`stop`] idempotent: calling [`start`] twice is a no-op, and [`stop`]
/// must fully join all listener threads before returning.
///
/// [`start`]: PowerBackend::start
/// [`stop`]: PowerBackend::stop
pub(crate) trait PowerBackend: Send + Sync {
    /// Queries the current power state.
    fn state(&self) -> Result<PowerState>;

    /// Starts the native event listeners, forwarding signals to `events`.
    ///
    /// A failure to start listeners only means *no events*; queries through
    /// [`state`](PowerBackend::state) keep working. Callers are expected to log
    /// the error and continue.
    fn start(&self, events: Sender<NativeEvent>) -> Result<()>;

    /// Stops all native listeners and joins their threads.
    ///
    /// After this returns, no further events may be sent through previously
    /// provided senders.
    fn stop(&self);
}

/// Creates the platform backend for the current operating system.
pub(crate) fn backend() -> Box<dyn PowerBackend> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPowerBackend::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosPowerBackend::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPowerBackend::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Box::new(UnsupportedPowerBackend)
    }
}

/// Placeholder backend for operating systems without a power backend.
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
struct UnsupportedPowerBackend;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
impl PowerBackend for UnsupportedPowerBackend {
    fn state(&self) -> Result<PowerState> {
        Err(Error::UnsupportedPlatform)
    }

    fn start(&self, _events: Sender<NativeEvent>) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }

    fn stop(&self) {}
}

// ---------------------------------------------------------------------------
// Windows mapping (`SYSTEM_POWER_STATUS`), per learn.microsoft.com:
//   ACLineStatus: 0 offline, 1 online, 255 unknown.
//   BatteryFlag: 1 high, 2 low, 4 critical, 8 charging, 128 no battery,
//                255 unknown; other bits are reserved.
//   BatteryLifePercent: 0-100, 255 unknown.
//   BatteryLifeTime / BatteryFullLifeTime: seconds, 0xFFFFFFFF unknown.
// ---------------------------------------------------------------------------

/// `SYSTEM_POWER_STATUS.BatteryFlag` bit indicating that no battery exists.
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_BATTERY_FLAG_NO_BATTERY: u8 = 0x80;
/// `SYSTEM_POWER_STATUS.BatteryFlag` bit indicating that the battery is charging.
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_BATTERY_FLAG_CHARGING: u8 = 0x08;
/// `SYSTEM_POWER_STATUS` sentinel meaning "unknown value".
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_UNKNOWN_U8: u8 = 0xFF;
#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_UNKNOWN_U32: u32 = 0xFFFF_FFFF;

/// Maps a raw `SYSTEM_POWER_STATUS` snapshot to the unified model.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn state_from_system_power_status(
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
) -> PowerState {
    let power_source = match ac_line_status {
        0 => PowerSource::Battery,
        1 => PowerSource::Ac,
        _ => PowerSource::Unknown,
    };

    // 128 explicitly means "no battery". 255 means the flags themselves are
    // unknown: ACPI-only desktops and some VMs report it, so we treat it as
    // "no battery" rather than fabricating battery data.
    let is_present =
        battery_flag & WINDOWS_BATTERY_FLAG_NO_BATTERY == 0 && battery_flag != WINDOWS_UNKNOWN_U8;

    let battery = if is_present {
        Some(BatteryInfo {
            is_present: true,
            percentage: match battery_life_percent {
                WINDOWS_UNKNOWN_U8 => None,
                percentage => Some(percentage.min(100)),
            },
            is_charging: match battery_flag {
                WINDOWS_UNKNOWN_U8 => None,
                flags => Some(flags & WINDOWS_BATTERY_FLAG_CHARGING != 0),
            },
            time_to_empty: seconds_from_windows_dword(battery_life_time),
            time_to_full: seconds_from_windows_dword(battery_full_life_time),
        })
    } else {
        None
    };

    PowerState {
        power_source,
        battery,
    }
}

#[cfg(any(target_os = "windows", test))]
fn seconds_from_windows_dword(value: u32) -> Option<u64> {
    if value == WINDOWS_UNKNOWN_U32 {
        None
    } else {
        Some(u64::from(value))
    }
}

// ---------------------------------------------------------------------------
// macOS IOPowerSources mapping (verified against IOPSKeys.h):
//   "Power Source State": "AC Power" / "Battery Power" / "Off Line"
//   "Is Present": boolean
//   "Current Capacity" / "Max Capacity": numbers (percent-based)
//   "Time to Empty" / "Time to Full Charge": signed minutes, -1 = calculating
// ---------------------------------------------------------------------------

/// Maps an IOPowerSources `"Power Source State"` string to the unified model.
///
/// Returns `None` for `"Off Line"` or unrecognized values (mapped to
/// `PowerSource::Unknown` by the caller).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn power_source_from_iops_state(state: &str) -> Option<PowerSource> {
    match state {
        "AC Power" => Some(PowerSource::Ac),
        "Battery Power" => Some(PowerSource::Battery),
        _ => None,
    }
}

/// Converts IOPowerSources minutes (with `-1` meaning "still calculating")
/// into seconds.
///
/// `0` is also treated as unknown: a zero estimate is degenerate filler
/// (e.g. on some models while plugged in) rather than a real prediction.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn seconds_from_iops_minutes(minutes: i64) -> Option<u64> {
    if minutes <= 0 {
        None
    } else {
        Some(minutes as u64 * 60)
    }
}

/// Computes a percentage from IOPowerSources capacity numbers.
///
/// Returns `None` when either value is missing or non-positive.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn percentage_from_iops_capacities(
    current: Option<i64>,
    max: Option<i64>,
) -> Option<u8> {
    let (current, max) = (current?, max?);
    if current < 0 || max <= 0 {
        return None;
    }
    let percentage = (current as f64 / max as f64 * 100.0).clamp(0.0, 100.0);
    Some(percentage.round() as u8)
}

// ---------------------------------------------------------------------------
// Linux UPower mapping (per upower.freedesktop.org/docs/):
//   Device.State: 0 unknown, 1 charging, 2 discharging, 3 empty,
//                4 fully charged, 5 pending charge, 6 pending discharge
// ---------------------------------------------------------------------------

/// Maps a UPower `Device.State` value to a charging flag.
///
/// Pending states (5/6) and unknown (0) map to `None` — their meaning is
/// scheduling-dependent, so we do not fabricate a value.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn charging_from_upower_state(state: u32) -> Option<bool> {
    match state {
        1 => Some(true),
        2..=4 => Some(false),
        0 | 5 | 6 => None,
        _ => None,
    }
}

/// Maps the UPower daemon `OnBattery` property.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn power_source_from_upower(on_battery: bool) -> PowerSource {
    if on_battery {
        PowerSource::Battery
    } else {
        PowerSource::Ac
    }
}

// ---------------------------------------------------------------------------
// Linux sysfs mapping (per Documentation/ABI/testing/sysfs-class-power):
//   type: "Mains" / "Battery" / "UPS" / "USB"
//   online: 0 offline, 1 online (2 is a PD-related quirk on some kernels,
//          documented as "still powering the system")
//   capacity: 0-100 (may be missing)
//   status: "Charging" / "Discharging" / "Not charging" / "Full" / "Unknown"
//   present: 0/1, file may be absent (kernel default: present)
// ---------------------------------------------------------------------------

/// Maps a sysfs `status` value to a charging flag.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn charging_from_sysfs_status(status: &str) -> Option<bool> {
    match status.trim() {
        "Charging" => Some(true),
        "Discharging" | "Not charging" | "Full" => Some(false),
        _ => None,
    }
}

/// Maps a sysfs `online` value (any non-"0" counts as online, including the
/// documented `2` PD quirk).
#[cfg(any(target_os = "linux", test))]
pub(crate) fn sysfs_adapter_is_online(online: &str) -> bool {
    !online.trim().is_empty() && online.trim() != "0"
}

// ---------------------------------------------------------------------------
// Linux sysfs fallback reader.
//
// Reads the `/sys/class/power_supply` tree (the documented kernel ABI, see
// Documentation/ABI/testing/sysfs-class-power). This is a *fallback* used when
// D-Bus or UPower are unavailable; it powers queries only, never events.
// The root is parameterized so tests can feed fixture trees.
// ---------------------------------------------------------------------------

/// Reads the power state from a sysfs power supply tree.
///
/// The first present battery (in lexicographic order, e.g. `BAT0` before
/// `BAT1`) provides the battery details. Systems with several batteries are
/// rare; the choice is documented rather than silently aggregated.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn read_state_from_sysfs(root: &std::path::Path) -> Result<PowerState> {
    let entries = std::fs::read_dir(root).map_err(Error::query)?;

    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();

    let mut ac_adapter_present = false;
    let mut ac_adapter_online = false;
    let mut any_battery_present = false;
    let mut battery: Option<BatteryInfo> = None;

    for path in paths {
        let kind = read_sysfs_file(&path.join("type")).unwrap_or_default();
        match kind.as_str() {
            "Mains" => {
                ac_adapter_present = true;
                if let Some(online) = read_sysfs_file(&path.join("online")) {
                    if sysfs_adapter_is_online(&online) {
                        ac_adapter_online = true;
                    }
                }
            }
            "Battery" => {
                // The `present` file is optional; the kernel default is
                // "present".
                let present = read_sysfs_file(&path.join("present"))
                    .map(|p| p != "0")
                    .unwrap_or(true);
                if !present {
                    continue;
                }
                any_battery_present = true;
                if battery.is_none() {
                    battery = Some(BatteryInfo {
                        is_present: true,
                        percentage: read_sysfs_file(&path.join("capacity"))
                            .and_then(|c| c.trim().parse::<u8>().ok())
                            .map(|p| p.min(100)),
                        is_charging: read_sysfs_file(&path.join("status"))
                            .map(|s| charging_from_sysfs_status(&s))
                            .unwrap_or(None),
                        // sysfs does not expose time estimates.
                        time_to_empty: None,
                        time_to_full: None,
                    });
                }
            }
            // UPS, USB and wireless supplies are out of scope for v1.
            _ => {}
        }
    }

    let power_source = if ac_adapter_online {
        PowerSource::Ac
    } else if any_battery_present {
        PowerSource::Battery
    } else {
        // No battery and an absent/offline adapter (or no supplies at all,
        // e.g. VMs and most servers).
        let _ = ac_adapter_present;
        PowerSource::Unknown
    };

    Ok(PowerState {
        power_source,
        battery,
    })
}

/// Reads a single sysfs attribute file, trimmed.
#[cfg(any(target_os = "linux", test))]
fn read_sysfs_file(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Windows -----------------------------------------------------------

    #[test]
    fn windows_maps_ac_online_with_battery() {
        let state = state_from_system_power_status(1, 0x01, 87, 0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(state.power_source, PowerSource::Ac);
        let battery = state.battery.unwrap();
        assert!(battery.is_present);
        assert_eq!(battery.percentage, Some(87));
        assert_eq!(battery.is_charging, Some(false));
        assert_eq!(battery.time_to_empty, None);
        assert_eq!(battery.time_to_full, None);
    }

    #[test]
    fn windows_maps_battery_with_charging_flag() {
        let state = state_from_system_power_status(0, 0x08, 42, 5400, 0xFFFF_FFFF);
        assert_eq!(state.power_source, PowerSource::Battery);
        let battery = state.battery.unwrap();
        assert_eq!(battery.is_charging, Some(true));
        assert_eq!(battery.percentage, Some(42));
        assert_eq!(battery.time_to_empty, Some(5400));
        assert_eq!(battery.time_to_full, None);
    }

    #[test]
    fn windows_desktop_has_no_battery() {
        let state = state_from_system_power_status(1, 0x80, 0xFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(state.power_source, PowerSource::Ac);
        assert!(state.battery.is_none());
    }

    #[test]
    fn windows_unknown_ac_line_status_maps_to_unknown() {
        let state = state_from_system_power_status(0xFF, 0x80, 0xFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(state.power_source, PowerSource::Unknown);
    }

    #[test]
    fn windows_unknown_battery_flags_map_to_none_charging() {
        let state = state_from_system_power_status(0, 0xFF, 0xFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
        // 255 = unknown flags: treated as no battery (documented limitation).
        assert!(state.battery.is_none());
    }

    #[test]
    fn windows_clamps_bogus_percentage() {
        let state = state_from_system_power_status(1, 0x01, 200, 0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(state.battery.unwrap().percentage, Some(100));
    }

    // -- macOS --------------------------------------------------------------

    #[test]
    fn iops_state_strings_map_to_power_source() {
        assert_eq!(
            power_source_from_iops_state("AC Power"),
            Some(PowerSource::Ac)
        );
        assert_eq!(
            power_source_from_iops_state("Battery Power"),
            Some(PowerSource::Battery)
        );
        assert_eq!(power_source_from_iops_state("Off Line"), None);
        assert_eq!(power_source_from_iops_state(""), None);
    }

    #[test]
    fn iops_minutes_handle_calculating_sentinel() {
        assert_eq!(seconds_from_iops_minutes(-1), None);
        assert_eq!(seconds_from_iops_minutes(0), None);
        assert_eq!(seconds_from_iops_minutes(59), Some(59 * 60));
    }

    #[test]
    fn iops_percentage_is_computed_from_capacities() {
        assert_eq!(
            percentage_from_iops_capacities(Some(87), Some(100)),
            Some(87)
        );
        assert_eq!(
            percentage_from_iops_capacities(Some(45), Some(90)),
            Some(50)
        );
        assert_eq!(percentage_from_iops_capacities(None, Some(100)), None);
        assert_eq!(percentage_from_iops_capacities(Some(50), None), None);
        assert_eq!(percentage_from_iops_capacities(Some(-1), Some(100)), None);
        assert_eq!(percentage_from_iops_capacities(Some(100), Some(0)), None);
        assert_eq!(
            percentage_from_iops_capacities(Some(120), Some(100)),
            Some(100)
        );
    }

    // -- Linux UPower -------------------------------------------------------

    #[test]
    fn upower_state_maps_charging() {
        assert_eq!(charging_from_upower_state(1), Some(true));
        assert_eq!(charging_from_upower_state(2), Some(false));
        assert_eq!(charging_from_upower_state(3), Some(false));
        assert_eq!(charging_from_upower_state(4), Some(false));
        assert_eq!(charging_from_upower_state(0), None);
        assert_eq!(charging_from_upower_state(5), None);
        assert_eq!(charging_from_upower_state(6), None);
        assert_eq!(charging_from_upower_state(99), None);
    }

    #[test]
    fn upower_on_battery_maps_power_source() {
        assert_eq!(power_source_from_upower(true), PowerSource::Battery);
        assert_eq!(power_source_from_upower(false), PowerSource::Ac);
    }

    // -- Linux sysfs --------------------------------------------------------

    #[test]
    fn sysfs_status_maps_charging() {
        assert_eq!(charging_from_sysfs_status("Charging"), Some(true));
        assert_eq!(charging_from_sysfs_status("Discharging\n"), Some(false));
        assert_eq!(charging_from_sysfs_status("Not charging"), Some(false));
        assert_eq!(charging_from_sysfs_status("Full"), Some(false));
        assert_eq!(charging_from_sysfs_status("Unknown"), None);
        assert_eq!(charging_from_sysfs_status(""), None);
    }

    #[test]
    fn sysfs_online_maps_adapter_state() {
        assert!(sysfs_adapter_is_online("1"));
        assert!(sysfs_adapter_is_online("2\n")); // PD quirk still powers the system
        assert!(!sysfs_adapter_is_online("0"));
        assert!(!sysfs_adapter_is_online(""));
    }

    // -- sysfs tree reader --------------------------------------------------
    //
    // Fixture-based tests exercising the actual reader on any platform.

    /// A guard that removes the fixture directory when dropped.
    struct SysfsFixture(std::path::PathBuf);

    impl SysfsFixture {
        fn create(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tauri-power-monitor-sysfs-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("failed to create sysfs fixture root");
            SysfsFixture(root)
        }

        fn supply(&self, name: &str, files: &[(&str, &str)]) -> &Self {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).expect("failed to create supply dir");
            for (file, contents) in files {
                std::fs::write(dir.join(file), contents).expect("failed to write fixture file");
            }
            self
        }
    }

    impl Drop for SysfsFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sysfs_reader_parses_laptop_on_ac() {
        let fixture = SysfsFixture::create("ac");
        fixture.supply("ADP1", &[("type", "Mains"), ("online", "1")]);
        fixture.supply(
            "BAT0",
            &[
                ("type", "Battery"),
                ("present", "1"),
                ("capacity", "64"),
                ("status", "Charging"),
            ],
        );

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Ac);
        let battery = state.battery.expect("battery expected");
        assert_eq!(battery.percentage, Some(64));
        assert_eq!(battery.is_charging, Some(true));
        assert_eq!(battery.time_to_empty, None);
        assert_eq!(battery.time_to_full, None);
    }

    #[test]
    fn sysfs_reader_parses_laptop_on_battery() {
        let fixture = SysfsFixture::create("battery");
        fixture.supply("ADP1", &[("type", "Mains"), ("online", "0")]);
        fixture.supply(
            "BAT0",
            &[
                ("type", "Battery"),
                ("present", "1"),
                ("capacity", "23"),
                ("status", "Discharging"),
            ],
        );

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Battery);
        assert_eq!(state.battery.unwrap().is_charging, Some(false));
    }

    #[test]
    fn sysfs_reader_parses_desktop_without_battery() {
        let fixture = SysfsFixture::create("desktop");
        fixture.supply("AC0", &[("type", "Mains"), ("online", "1")]);

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Ac);
        assert!(state.battery.is_none());
    }

    #[test]
    fn sysfs_reader_reports_unknown_without_supplies() {
        let fixture = SysfsFixture::create("empty");
        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Unknown);
        assert!(state.battery.is_none());
    }

    #[test]
    fn sysfs_reader_handles_missing_present_file_and_offline_adapter() {
        // `present` is optional and defaults to present; capacity missing →
        // percentage unknown.
        let fixture = SysfsFixture::create("quirks");
        fixture.supply("ADP1", &[("type", "Mains"), ("online", "0")]);
        fixture.supply("BAT0", &[("type", "Battery"), ("status", "Full")]);

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Battery);
        let battery = state.battery.expect("battery expected");
        assert!(battery.is_present);
        assert_eq!(battery.percentage, None);
        assert_eq!(battery.is_charging, Some(false));
    }

    #[test]
    fn sysfs_reader_picks_the_first_present_battery_deterministically() {
        let fixture = SysfsFixture::create("multi-battery");
        fixture.supply("ADP1", &[("type", "Mains"), ("online", "1")]);
        fixture.supply("BAT0", &[("type", "Battery"), ("capacity", "50")]);
        fixture.supply(
            "BAT1",
            &[
                ("type", "Battery"),
                ("capacity", "90"),
                ("status", "Charging"),
            ],
        );

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Ac);
        assert_eq!(state.battery.unwrap().percentage, Some(50));
    }

    #[test]
    fn sysfs_reader_ignores_absent_battery() {
        let fixture = SysfsFixture::create("absent-battery");
        fixture.supply("ADP1", &[("type", "Mains"), ("online", "1")]);
        fixture.supply(
            "BAT0",
            &[("type", "Battery"), ("present", "0"), ("capacity", "10")],
        );

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Ac);
        assert!(state.battery.is_none());
    }

    #[test]
    fn sysfs_reader_ignores_out_of_scope_supply_kinds() {
        let fixture = SysfsFixture::create("ups");
        fixture.supply(
            "UPS0",
            &[("type", "UPS"), ("online", "1"), ("capacity", "99")],
        );

        let state = read_state_from_sysfs(&fixture.0).expect("sysfs read");
        assert_eq!(state.power_source, PowerSource::Unknown);
        assert!(state.battery.is_none());
    }

    #[test]
    fn sysfs_reader_errors_on_missing_root() {
        let missing = std::env::temp_dir().join("tauri-power-monitor-does-not-exist");
        assert!(read_state_from_sysfs(&missing).is_err());
    }
}
