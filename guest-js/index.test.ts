// Copyright 2019-2025 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

// Contract tests pinning the IPC surface (command names, event names and
// payload forwarding) of the guest JS bindings.

import { beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn()
const listenMock = vi.fn()
const unlistenMock = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args)
}))

import {
  getPowerState,
  onBatteryChanged,
  onPowerSourceChanged,
  onResume,
  onSuspend,
  type PowerState
} from './index'

const sampleState: PowerState = {
  powerSource: 'ac',
  battery: {
    isPresent: true,
    percentage: 87,
    isCharging: true,
    timeToEmpty: null,
    timeToFull: 2340
  }
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockReset()
  listenMock.mockResolvedValue(unlistenMock)
})

describe('getPowerState', () => {
  it('invokes the plugin command with the right name', async () => {
    invokeMock.mockResolvedValue(sampleState)
    const state = await getPowerState()
    expect(invokeMock).toHaveBeenCalledWith('plugin:power-monitor|get_power_state')
    expect(state).toEqual(sampleState)
  })
})

describe('onPowerSourceChanged', () => {
  it('listens to the power-source-changed event and forwards the payload', async () => {
    const handler = vi.fn()
    await onPowerSourceChanged(handler)

    expect(listenMock).toHaveBeenCalledWith(
      'power-monitor://power-source-changed',
      expect.any(Function)
    )

    const listener = listenMock.mock.calls[0]![1] as (event: { payload: PowerState }) => void
    listener({ payload: sampleState })
    expect(handler).toHaveBeenCalledWith(sampleState)
  })

  it('resolves to the unlisten function', async () => {
    const unlisten = await onPowerSourceChanged(() => {})
    expect(unlisten).toBe(unlistenMock)
  })
})

describe('onBatteryChanged', () => {
  it('listens to the battery-changed event and forwards the payload', async () => {
    const handler = vi.fn()
    await onBatteryChanged(handler)

    expect(listenMock).toHaveBeenCalledWith(
      'power-monitor://battery-changed',
      expect.any(Function)
    )

    const listener = listenMock.mock.calls[0]![1] as (event: { payload: PowerState }) => void
    listener({ payload: sampleState })
    expect(handler).toHaveBeenCalledWith(sampleState)
  })
})

describe('onSuspend', () => {
  it('listens to the suspend event without payload', async () => {
    const handler = vi.fn()
    await onSuspend(handler)

    expect(listenMock).toHaveBeenCalledWith('power-monitor://suspend', expect.any(Function))

    const listener = listenMock.mock.calls[0]![1] as () => void
    listener()
    expect(handler).toHaveBeenCalledOnce()
  })
})

describe('onResume', () => {
  it('listens to the resume event without payload', async () => {
    const handler = vi.fn()
    await onResume(handler)

    expect(listenMock).toHaveBeenCalledWith('power-monitor://resume', expect.any(Function))

    const listener = listenMock.mock.calls[0]![1] as () => void
    listener()
    expect(handler).toHaveBeenCalledOnce()
  })
})
