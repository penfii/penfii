import { useCallback, useEffect, useState } from 'react'
import { queryKeys } from '../../queryKeys'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { PushSubscriptionInfo } from '@penfii/contracts'
import { api } from '../../api'
import { currentEnvironment, deviceLabel, pushSupport, urlBase64ToBytes, type PushSupport } from './pushSupport'

export type PushState =
  | { kind: 'loading' }
  | { kind: 'unconfigured' }
  | { kind: 'unsupported' }
  | { kind: 'ios-needs-install' }
  | { kind: 'denied' }
  | { kind: 'off' }
  | { kind: 'on' }

const KEY = queryKeys.pushSubscriptions

/**
 * Whether this device receives Web Push, and the switch for it. The switch is per device: turning
 * it on asks the browser for permission (from the click that calls `turnOn`), subscribes with the
 * server's public key and registers the subscription; turning it off removes both.
 */
export function usePushSubscription() {
  const queryClient = useQueryClient()
  const support: PushSupport = pushSupport(currentEnvironment())
  const key = useQuery({ queryKey: queryKeys.pushPublicKey, queryFn: () => api.getPushPublicKey(), staleTime: 5 * 60_000, retry: false })
  const devices = useQuery({ queryKey: KEY, queryFn: () => api.listPushSubscriptions(), enabled: key.data?.configured === true })
  const [endpoint, setEndpoint] = useState<string | null>(null)
  const [permission, setPermission] = useState<NotificationPermission>(() => (typeof Notification === 'undefined' ? 'default' : Notification.permission))
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)

  // This browser's own subscription, if it has one.
  const refreshEndpoint = useCallback(async () => {
    if (support !== 'supported') return
    try {
      const registration = await navigator.serviceWorker.ready
      const current = await registration.pushManager.getSubscription()
      setEndpoint(current?.endpoint ?? null)
    } catch {
      setEndpoint(null)
    }
  }, [support])
  useEffect(() => {
    void refreshEndpoint()
  }, [refreshEndpoint])

  const mine: PushSubscriptionInfo | undefined = devices.data?.find((device) => device.endpoint === endpoint)
  let state: PushState
  if (key.isPending) state = { kind: 'loading' }
  else if (!key.data?.configured) state = { kind: 'unconfigured' }
  else if (support === 'unsupported') state = { kind: 'unsupported' }
  else if (support === 'ios-needs-install') state = { kind: 'ios-needs-install' }
  else if (permission === 'denied') state = { kind: 'denied' }
  else if (permission === 'granted' && mine) state = { kind: 'on' }
  else state = devices.isPending && key.data.configured ? { kind: 'loading' } : { kind: 'off' }

  const turnOn = useCallback(async () => {
    const publicKey = key.data?.publicKey
    if (!publicKey || support !== 'supported') return
    setBusy(true)
    setFailed(false)
    try {
      // Called straight from the click: browsers only ask for permission from a user gesture.
      const result = await Notification.requestPermission()
      setPermission(result)
      if (result !== 'granted') return
      const registration = await navigator.serviceWorker.ready
      const existing = await registration.pushManager.getSubscription()
      const subscription = existing ?? (await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: urlBase64ToBytes(publicKey) }))
      const json = subscription.toJSON()
      if (!json.endpoint || !json.keys?.p256dh || !json.keys.auth) throw new Error('incomplete subscription')
      await api.subscribePush(
        { endpoint: json.endpoint, keys: { p256dh: json.keys.p256dh, auth: json.keys.auth } },
        { userAgent: navigator.userAgent.slice(0, 400), label: deviceLabel(navigator.userAgent) },
      )
      setEndpoint(subscription.endpoint)
      await queryClient.invalidateQueries({ queryKey: KEY })
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }, [key.data?.publicKey, queryClient, support])

  const removeDevice = useCallback(
    async (device: PushSubscriptionInfo) => {
      setBusy(true)
      setFailed(false)
      try {
        if (device.endpoint === endpoint) {
          const registration = await navigator.serviceWorker.ready
          const current = await registration.pushManager.getSubscription()
          await current?.unsubscribe()
          setEndpoint(null)
        }
        await api.unsubscribePush(device.id)
        await queryClient.invalidateQueries({ queryKey: KEY })
      } catch {
        setFailed(true)
      } finally {
        setBusy(false)
      }
    },
    [endpoint, queryClient],
  )

  const turnOff = useCallback(async () => {
    if (mine) await removeDevice(mine)
  }, [mine, removeDevice])

  return { state, busy, failed, devices: devices.data ?? [], thisEndpoint: endpoint, turnOn, turnOff, removeDevice }
}
