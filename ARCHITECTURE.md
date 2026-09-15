# Architecture

`tauri-plugin-power-monitor` is a community Tauri 2 plugin that provides read-only, event-driven power monitoring for desktop applications. It unifies battery state, charging state, power source, and suspend/resume notifications across Windows, macOS, and Linux without controlling the operating system.

## Problem and scope

Desktop applications sometimes need to react to power state changes without depending on platform-specific code in the frontend. Version 0.1.0 exposes one state query and four events. It does not implement shutdown, reboot, hibernate, sleep control, prevent-sleep, power policies, UPS monitoring, thermal monitoring, or telemetry.

## Architecture

The Rust plugin owns the current `PowerState`, registers native listeners during the Tauri lifecycle, and sends native changes to a single dispatcher. The dispatcher queries or receives the updated state, compares it with the previous state, and emits only the relevant Tauri events. This keeps event semantics consistent across platforms and avoids polling.

The guest JavaScript API invokes `get_power_state` and subscribes to the `power-monitor://` events. Each subscription returns Tauri's `UnlistenFn`. The permission surface contains the read-only `power-monitor:default` set and the `allow-get-power-state` and `deny-get-power-state` command permissions.

## Platform backends

### Windows

The backend queries `GetSystemPowerStatus` and listens through a hidden window for `WM_POWERBROADCAST`. It uses `RegisterSuspendResumeNotification` for suspend/resume and `RegisterPowerSettingNotification` for power-source changes and battery notifications.

### macOS

The backend reads IOPowerSources for battery and power-source state. It registers for IOKit power events with `IORegisterForSystemPower` and acknowledges sleep transitions with `IOAllowPowerChange`.

### Linux

UPower over D-Bus, accessed through zbus, provides queries and power/battery change notifications. systemd-logind provides suspend/resume events. When UPower or D-Bus is unavailable, queries fall back to `/sys/class/power_supply`; that fallback cannot provide suspend/resume events or time estimates.

## Lifecycle and limitations

Native listeners are started with the plugin lifecycle and shut down with their platform resources. Unknown native values are represented as `null` rather than fabricated values. Changes to `timeToEmpty` or `timeToFull` alone do not emit `battery-changed`.

The plugin is desktop-only. Android and iOS are intentionally unsupported. Platform event coverage depends on the native facilities available on the host, especially systemd-logind on Linux.

## Validation

The current suite contains 38 Rust unit tests, 1 Rust doctest, and 6 TypeScript tests. Manual verification has been performed with the Tauri example on Linux, including suspend and resume. The Linux verification desktop has no battery, so physical AC/battery transitions were not exercised there.

## Design decisions

The implementation uses native event sources instead of polling, preserves platform-specific resource lifecycles, and keeps the IPC surface small. The plugin follows Tauri 2 conventions for builder setup, permissions, generated API files, and platform-gated dependencies while remaining an independent community project rather than an official Tauri plugin. The current dependency graph has an MSRV of Rust 1.88; this is higher than the MSRV of Tauri itself because of the selected Linux D-Bus and platform dependencies.
