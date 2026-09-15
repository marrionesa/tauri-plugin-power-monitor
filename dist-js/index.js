import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT
/**
 * Returns the current power state of the system (power source and battery
 * information).
 *
 * Requires the `power-monitor:allow-get-power-state` permission.
 *
 * @example
 * ```typescript
 * import { getPowerState } from '@marrionesa/plugin-power-monitor';
 * const state = await getPowerState();
 * console.log(state.powerSource); // 'ac' | 'battery' | 'unknown'
 * ```
 *
 * #### Platform-specific
 *
 * - **Linux:** Uses UPower over D-Bus and falls back to reading
 *   `/sys/class/power_supply` when UPower or D-Bus are unavailable (in that
 *   case `timeToEmpty`/`timeToFull` are always `null`).
 *
 * @since 0.1.0
 */
async function getPowerState() {
    return await invoke('plugin:power-monitor|get_power_state');
}
/**
 * Listen to power source changes (system switched between AC and battery).
 *
 * The payload is the full power state after the change.
 *
 * @example
 * ```typescript
 * import { onPowerSourceChanged } from '@marrionesa/plugin-power-monitor';
 * const unlisten = await onPowerSourceChanged((state) => {
 *   console.log(`now running on ${state.powerSource}`);
 * });
 * ```
 *
 * @since 0.1.0
 */
async function onPowerSourceChanged(handler) {
    return await listen('power-monitor://power-source-changed', (event) => {
        handler(event.payload);
    });
}
/**
 * Listen to battery changes: percentage, charging state or presence.
 *
 * The payload is the full power state after the change. Time estimates
 * (`timeToEmpty`/`timeToFull`) change roughly every minute while
 * discharging and do **not** trigger this event; query
 * {@link getPowerState} for fresh estimates.
 *
 * @example
 * ```typescript
 * import { onBatteryChanged } from '@marrionesa/plugin-power-monitor';
 * const unlisten = await onBatteryChanged((state) => {
 *   console.log(`battery: ${state.battery?.percentage}%`);
 * });
 * ```
 *
 * @since 0.1.0
 */
async function onBatteryChanged(handler) {
    return await listen('power-monitor://battery-changed', (event) => {
        handler(event.payload);
    });
}
/**
 * Listen to system suspend events. The system is about to sleep.
 *
 * @example
 * ```typescript
 * import { onSuspend } from '@marrionesa/plugin-power-monitor';
 * await onSuspend(() => console.log('suspending'));
 * ```
 *
 * #### Platform-specific
 *
 * - **Linux:** Requires systemd-logind; on non-systemd systems this event
 *   never fires.
 *
 * @since 0.1.0
 */
async function onSuspend(handler) {
    return await listen('power-monitor://suspend', () => {
        handler();
    });
}
/**
 * Listen to system resume events. The system woke up from sleep.
 *
 * @example
 * ```typescript
 * import { onResume } from '@marrionesa/plugin-power-monitor';
 * await onResume(() => console.log('resumed'));
 * ```
 *
 * #### Platform-specific
 *
 * - **Linux:** Requires systemd-logind; on non-systemd systems this event
 *   never fires.
 *
 * @since 0.1.0
 */
async function onResume(handler) {
    return await listen('power-monitor://resume', () => {
        handler();
    });
}

export { getPowerState, onBatteryChanged, onPowerSourceChanged, onResume, onSuspend };
