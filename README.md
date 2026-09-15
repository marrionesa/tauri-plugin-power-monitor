# Tauri Power Monitor

A Tauri 2 plugin for monitoring battery state, charging, power source, and system suspend/resume events across Windows, macOS, and Linux.

This is a desktop-only, read-only monitor. It never shuts down, reboots, hibernates, puts the system to sleep, prevents sleep, or changes power policies.

## Installation

Rust:

```toml
[dependencies]
tauri-plugin-power-monitor = "0.1"
```

JavaScript:

```bash
npm install @marrionesa/plugin-power-monitor
```

Register the plugin in the Rust application:

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_power_monitor::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

During local development, the example app uses the package from the repository with a local `file:../..` dependency.

The current dependency set requires Rust 1.88 or newer. Tauri 2 itself supports
older Rust versions, but the Linux D-Bus and platform dependency graph used by
this release has a higher MSRV.

## JavaScript API

```ts
import {
  getPowerState,
  onPowerSourceChanged,
  onBatteryChanged,
  onSuspend,
  onResume,
  type PowerState,
} from '@marrionesa/plugin-power-monitor'

const state: PowerState = await getPowerState()

const unlistenPower = await onPowerSourceChanged((next) => {
  console.log('power source:', next.powerSource)
})

const unlistenBattery = await onBatteryChanged((next) => {
  console.log('battery:', next.battery)
})

const unlistenSuspend = await onSuspend(() => console.log('suspend'))
const unlistenResume = await onResume(() => console.log('resume'))

// Call the returned UnlistenFn values when the subscriptions are no longer needed.
void unlistenPower
void unlistenBattery
void unlistenSuspend
void unlistenResume
```

- `getPowerState()` returns the current `PowerState`.
- `onPowerSourceChanged()` reports AC/battery source changes.
- `onBatteryChanged()` reports battery presence, percentage, or charging changes.
- `onSuspend()` reports that the system is about to suspend.
- `onResume()` reports that the system resumed.

Subscriptions resolve to Tauri `UnlistenFn` values. `timeToEmpty` and `timeToFull` are optional estimates in seconds and may be `null`. Changes to those estimates alone do not emit `battery-changed`.

## Platform support

| Capability | Windows | macOS | Linux |
| --- | :---: | :---: | :---: |
| Battery query and presence | Yes | Yes | Yes |
| Charging state | Yes | Yes | Yes |
| Power source | Yes | Yes | Yes |
| Time estimates | Yes | Yes | UPower only |
| Power events | Yes | Yes | UPower only |
| Suspend/resume | Yes | Yes | systemd-logind |
| sysfs query fallback | No | No | Yes |

Linux uses UPower for power queries and events, systemd-logind for suspend/resume, and `/sys/class/power_supply` as a query fallback. Without systemd-logind, Linux suspend/resume events are unavailable. The sysfs fallback does not provide time estimates.

Android and iOS are not supported.

## Permissions

The plugin keeps a least-privilege surface:

- `power-monitor:default`
- `power-monitor:allow-get-power-state`
- `power-monitor:deny-get-power-state`

The plugin exposes no event permissions because event subscriptions use the registered plugin API and do not add commands.

## Testing

The project currently has 38 Rust unit tests, 1 Rust doctest, and 6 TypeScript tests. The Tauri example has been manually executed on Linux, including real suspend/resume verification. The Linux desktop used for manual testing has no battery, so AC/battery transitions could not be physically exercised on that machine.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check

pnpm install
pnpm build
pnpm typecheck
pnpm test

cd examples/api
npm install
npm run build
cargo tauri dev
```

The example frontend is static and uses esbuild. It is served by Tauri with `frontendDist = "../"` and does not require a development server.

## Community status

This is an open-source community plugin for Tauri 2. It is not currently part of the official Tauri plugins and is not listed in Awesome Tauri. The project is prepared for distribution through npm and crates.io, with a later goal of proposing it as a community resource in Awesome Tauri.

Repository: https://github.com/marrionesa/tauri-plugin-power-monitor

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE), [LICENSE-MIT](LICENSE-MIT), and [LICENSE-APACHE](LICENSE-APACHE).
