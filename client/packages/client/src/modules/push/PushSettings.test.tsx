import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getPushPublicKey: vi.fn(), listPushSubscriptions: vi.fn(), subscribePush: vi.fn(), unsubscribePush: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { QueryWrapper } from '../../test/queryWrapper'
import { PushSettings } from './PushSettings'

function mount() {
  return render(<QueryWrapper><I18nProvider><PushSettings /></I18nProvider></QueryWrapper>)
}

const original = { sw: Object.getOwnPropertyDescriptor(navigator, 'serviceWorker'), ua: navigator.userAgent }

function fakeBrowser(options: { permission?: NotificationPermission; subscription?: { endpoint: string } | null; ua?: string; standalone?: boolean }) {
  const subscription = options.subscription === undefined ? null : options.subscription
  const created = { endpoint: 'https://push.test/new', toJSON: () => ({ endpoint: 'https://push.test/new', keys: { p256dh: 'p', auth: 'a' } }), unsubscribe: vi.fn().mockResolvedValue(true) }
  const pushManager = {
    getSubscription: vi.fn().mockResolvedValue(subscription ? { ...created, endpoint: subscription.endpoint, unsubscribe: created.unsubscribe } : null),
    subscribe: vi.fn().mockResolvedValue(created),
  }
  Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: { ready: Promise.resolve({ pushManager }), addEventListener: vi.fn(), removeEventListener: vi.fn() } })
  Object.defineProperty(window, 'PushManager', { configurable: true, value: function PushManager() {} })
  const request = vi.fn().mockResolvedValue('granted')
  Object.defineProperty(window, 'Notification', { configurable: true, value: Object.assign(function Notification() {}, { permission: options.permission ?? 'default', requestPermission: request }) })
  if (options.ua) Object.defineProperty(navigator, 'userAgent', { configurable: true, value: options.ua })
  if (options.standalone) Object.defineProperty(navigator, 'standalone', { configurable: true, value: true })
  return { pushManager, request, created }
}

beforeEach(() => {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.listPushSubscriptions.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  Object.defineProperty(navigator, 'userAgent', { configurable: true, value: original.ua })
  Reflect.deleteProperty(navigator, 'standalone')
  Reflect.deleteProperty(window, 'PushManager')
  Reflect.deleteProperty(window, 'Notification')
  if (original.sw) Object.defineProperty(navigator, 'serviceWorker', original.sw)
  else Reflect.deleteProperty(navigator, 'serviceWorker')
})

describe('Push settings', () => {
  it('is hidden while the server has no VAPID key', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: false, publicKey: null })
    fakeBrowser({})
    const { container } = mount()
    await waitFor(() => expect(api.getPushPublicKey).toHaveBeenCalled())
    await new Promise((r) => setTimeout(r, 20))
    expect(container.querySelector('.push-settings')).toBeNull()
  })

  it('turns push on from the click: permission, subscribe with the server key, register', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: true, publicKey: 'AQID' })
    api.subscribePush.mockResolvedValue({ id: 'd1' })
    const browser = fakeBrowser({})
    mount()
    const toggle = await screen.findByRole('switch', { name: 'Receive push notifications on this device' })
    expect((toggle as HTMLInputElement).checked).toBe(false)
    fireEvent.click(toggle)
    await waitFor(() => expect(api.subscribePush).toHaveBeenCalled())
    expect(browser.request).toHaveBeenCalledTimes(1)
    expect(browser.pushManager.subscribe).toHaveBeenCalledWith({ userVisibleOnly: true, applicationServerKey: expect.any(Uint8Array) })
    expect(api.subscribePush).toHaveBeenCalledWith({ endpoint: 'https://push.test/new', keys: { p256dh: 'p', auth: 'a' } }, expect.objectContaining({ label: expect.any(String) }))
    // Nothing per kind to choose.
    expect(screen.queryAllByRole('checkbox').length + screen.queryAllByRole('switch').length).toBe(1)
  })

  it('turns it off: the browser subscription and the server row both go', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: true, publicKey: 'AQID' })
    api.listPushSubscriptions.mockResolvedValue([{ id: 'd1', endpoint: 'https://push.test/mine', label: 'Chrome · Windows', userAgent: null, createdAt: '2026-11-01T00:00:00Z', lastSuccessAt: null, lastFailureAt: '2026-11-02T00:00:00Z' }])
    api.unsubscribePush.mockResolvedValue(undefined)
    const browser = fakeBrowser({ permission: 'granted', subscription: { endpoint: 'https://push.test/mine' } })
    mount()
    const toggle = await screen.findByRole('switch')
    await waitFor(() => expect((toggle as HTMLInputElement).checked).toBe(true))
    expect(screen.getAllByText(/this device/).length).toBeGreaterThan(0)
    // What the push service did with the last pushes, so a missing push can be placed.
    expect(screen.getByText(/no push sent yet/)).toBeTruthy()
    expect(screen.getByText(/send failed since/)).toBeTruthy()
    fireEvent.click(toggle)
    await waitFor(() => expect(api.unsubscribePush).toHaveBeenCalledWith('d1'))
    expect(browser.created.unsubscribe).toHaveBeenCalled()
  })

  it('shows when the push service last accepted a push, and a failure only while no later push was accepted', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: true, publicKey: 'AQID' })
    const device = { userAgent: null, createdAt: '2026-11-01T00:00:00Z' }
    api.listPushSubscriptions.mockResolvedValue([
      { ...device, id: 'd1', endpoint: 'https://push.test/a', label: 'Edge · Windows', lastSuccessAt: '2026-11-03T00:00:00Z', lastFailureAt: '2026-11-02T00:00:00Z' },
      { ...device, id: 'd2', endpoint: 'https://push.test/b', label: 'Safari · iPhone', lastSuccessAt: '2026-11-02T00:00:00Z', lastFailureAt: '2026-11-04T00:00:00Z' },
    ])
    fakeBrowser({ permission: 'granted', subscription: { endpoint: 'https://push.test/a' } })
    mount()
    expect(await screen.findAllByText(/push service last accepted one/)).toHaveLength(2)
    // Only the device whose last failure is newer than its last accepted push shows one.
    expect(screen.getAllByText(/send failed since/)).toHaveLength(1)
  })

  it('says how to unblock a denied permission', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: true, publicKey: 'AQID' })
    fakeBrowser({ permission: 'denied' })
    mount()
    expect(await screen.findByText(/blocked in this browser/)).toBeTruthy()
    expect(screen.queryByRole('switch')).toBeNull()
  })

  it('on iPhone in a Safari tab it shows the Home Screen steps and no switch; installed, it shows the switch', async () => {
    api.getPushPublicKey.mockResolvedValue({ configured: true, publicKey: 'AQID' })
    fakeBrowser({ ua: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Safari/604' })
    Reflect.deleteProperty(window, 'PushManager')
    mount()
    expect(await screen.findByText(/Add to Home Screen/)).toBeTruthy()
    expect(screen.queryByRole('switch')).toBeNull()
    cleanup()
    fakeBrowser({ ua: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Safari/604', standalone: true })
    mount()
    expect(await screen.findByRole('switch')).toBeTruthy()
  })
})
