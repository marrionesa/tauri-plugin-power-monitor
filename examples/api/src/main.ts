// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

// Manual verification app for the Tauri Power Monitor community plugin.
//
// Renders the live power state and every event the plugin emits. Verify by:
// 1. Plugging / unplugging the AC adapter → power-source-changed
// 2. Waiting for a percentage change    → battery-changed
// 3. Suspending and resuming the system → suspend + resume

import {
  getPowerState,
  onBatteryChanged,
  onPowerSourceChanged,
  onSuspend,
  onResume,
  type PowerState,
} from '@marrionesa/plugin-power-monitor'

const el = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id)
  if (element === null) throw new Error(`missing element #${id}`)
  return element as T
}

function seconds(value: number | null): string {
  if (value === null) return 'unknown'

  const minutes = Math.round(value / 60)

  if (minutes < 60) {
    return `${minutes} min`
  }

  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`
}

function render(state: PowerState): void {
  el<HTMLTableCellElement>('power-source').textContent = state.powerSource

  el<HTMLTableCellElement>('battery-present').textContent =
    state.battery === null ? 'false' : 'true'

  if (state.battery === null) {
    el<HTMLTableCellElement>('battery-percentage').textContent = '—'
    el<HTMLTableCellElement>('battery-charging').textContent = '—'
    el<HTMLTableCellElement>('time-to-empty').textContent = '—'
    el<HTMLTableCellElement>('time-to-full').textContent = '—'
    return
  }

  const {
    percentage,
    isCharging,
    timeToEmpty,
    timeToFull,
  } = state.battery

  el<HTMLTableCellElement>('battery-percentage').textContent =
    percentage === null ? 'unknown' : `${percentage}%`

  el<HTMLTableCellElement>('battery-charging').textContent =
    isCharging === null ? 'unknown' : String(isCharging)

  el<HTMLTableCellElement>('time-to-empty').textContent =
    seconds(timeToEmpty)

  el<HTMLTableCellElement>('time-to-full').textContent =
    seconds(timeToFull)
}

const logEl = el<HTMLUListElement>('log')

function logEvent(kind: string, detail: string): void {
  const entry = document.createElement('li')
  const time = document.createElement('span')

  time.className = 'time'
  time.textContent = new Date().toLocaleTimeString()

  entry.className = kind
  entry.appendChild(time)
  entry.appendChild(document.createTextNode(`${kind} — ${detail}`))

  logEl.prepend(entry)

  while (logEl.childElementCount > 50) {
    logEl.removeChild(logEl.lastElementChild as HTMLElement)
  }
}

async function init(): Promise<void> {
  // Load the initial power state.
  try {
    const state = await getPowerState()
    render(state)
  } catch (error: unknown) {
    logEvent('error', `initial state: ${String(error)}`)
  }

  // Manual refresh button.
  el<HTMLButtonElement>('refresh').addEventListener('click', () => {
    getPowerState()
      .then(render)
      .catch((error: unknown) => {
        logEvent('error', String(error))
      })
  })

  // Power source changes: AC ↔ battery.
  await onPowerSourceChanged((state) => {
    render(state)
    logEvent('power-source-changed', state.powerSource)
  })

  // Battery presence, percentage or charging state changes.
  await onBatteryChanged((state) => {
    render(state)

    const battery = state.battery

    const detail =
      battery === null
        ? 'battery removed'
        : `${battery.percentage ?? '?'}% charging=${String(battery.isCharging)}`

    logEvent('battery-changed', detail)
  })

  // System is about to suspend.
  await onSuspend(() => {
    logEvent('suspend', 'the system is about to sleep')
  })

  // System resumed.
  await onResume(() => {
    logEvent('resume', 'the system woke up')
  })
}

void init().catch((error: unknown) => {
  console.error('Failed to initialize power monitor example:', error)
  logEvent('error', `initialization: ${String(error)}`)
})