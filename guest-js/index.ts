// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

import { invoke } from '@tauri-apps/api/core'
import { type UnlistenFn, listen } from '@tauri-apps/api/event'

/**
 * The source the system is currently drawing power from.
 *
 * @since 0.1.0
 */
export type PowerSource = 'ac' | 'battery' | 'unknown'

/**
 * Battery information. Fields are `null` when the underlying platform cannot
 * report them; absence of data is never converted into `false` or `0`.
 *
 * @since 0.1.0
 */
export interface BatteryInfo {
  /** Whether a battery is present in the system. */
  isPresent: boolean
  /** Battery charge percentage (0–100), or `null` when unknown. */
  percentage: number | null
  /** Whether the battery is currently charging, or `null` when unknown. */
  isCharging: boolean | null
  /** Estimated seconds until the battery is empty, or `null` when unknown. */
  timeToEmpty: number | null
  /** Estimated seconds until the battery is fully charged, or `null` when unknown. */
  timeToFull: number | null
}

/**
 * A full snapshot of the system's power state.
 *
 * @since 0.1.0
 */
export interface PowerState {
  /** The power source the system is currently running on. */
  powerSource: PowerSource
  /** Battery details, or `null` when no battery is present. */
  battery: BatteryInfo | null
}

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
export async function getPowerState(): Promise<PowerState> {
  return await invoke<PowerState>('plugin:power-monitor|get_power_state')
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
export async function onPowerSourceChanged(
  handler: (state: PowerState) => void
): Promise<UnlistenFn> {
  return await listen<PowerState>('power-monitor://power-source-changed', (event) => {
    handler(event.payload)
  })
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
export async function onBatteryChanged(
  handler: (state: PowerState) => void
): Promise<UnlistenFn> {
  return await listen<PowerState>('power-monitor://battery-changed', (event) => {
    handler(event.payload)
  })
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
export async function onSuspend(handler: () => void): Promise<UnlistenFn> {
  return await listen<null>('power-monitor://suspend', () => {
    handler()
  })
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
export async function onResume(handler: () => void): Promise<UnlistenFn> {
  return await listen<null>('power-monitor://resume', () => {
    handler()
  })
}
