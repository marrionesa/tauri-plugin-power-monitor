# Changelog

## \[0.1.0\]

### Added

- Initial community release of the desktop, read-only power monitor.

- `get_power_state` IPC command returning the unified `PowerState` model.
- `power-monitor://power-source-changed`, `power-monitor://battery-changed`,
  `power-monitor://suspend` and `power-monitor://resume` events.
- Windows backend: `GetSystemPowerStatus` queries; `WM_POWERBROADCAST` events
  on a hidden window with `RegisterSuspendResumeNotification` /
  `RegisterPowerSettingNotification`.
- macOS backend: IOPowerSources queries and notifications; sleep/wake through
  `IORegisterForSystemPower` with correct `IOAllowPowerChange` handling.
- Linux backend: UPower + systemd-logind over D-Bus (zbus), sysfs fallback
  for queries.
- Guest JS (`getPowerState`, `onPowerSourceChanged`, `onBatteryChanged`,
  `onSuspend`, `onResume`) with generated `dist-js` and `api-iife.js`.
- `power-monitor:default` permission set (read-only).
- Manual verification example app under `examples/api`.
- 38 Rust unit tests, 1 Rust doctest, and 6 TypeScript tests.
