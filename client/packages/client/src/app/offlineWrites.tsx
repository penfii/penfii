import { useEffect, useSyncExternalStore } from 'react'
import { setWriteBlocker } from '@penfii/api-client'
import { useI18n } from '../i18n/I18nProvider'
import './offlineWrites.css'

/**
 * One policy for every screen: offline the app only reads. A write is refused in the API client at once
 * (and mutations are never queued, see queryClient), and this notice says why. Nothing is kept to be
 * sent later; after reconnecting the person does it again.
 */
const NOTICE_MS = 4000
let visible = false
let timer: ReturnType<typeof setTimeout> | undefined
const listeners = new Set<() => void>()

function show() {
  visible = true
  clearTimeout(timer)
  timer = setTimeout(() => { visible = false; listeners.forEach((fn) => fn()) }, NOTICE_MS)
  listeners.forEach((fn) => fn())
}

function hide() {
  clearTimeout(timer)
  if (!visible) return
  visible = false
  listeners.forEach((fn) => fn())
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => { listeners.delete(fn) }
}

export function OfflineWrites() {
  const { t } = useI18n()
  const open = useSyncExternalStore(subscribe, () => visible, () => false)
  useEffect(() => {
    setWriteBlocker(() => !navigator.onLine, show)
    // Back online the notice goes: it was about a change that was not made, and nothing is waiting to be sent.
    window.addEventListener('online', hide)
    return () => {
      window.removeEventListener('online', hide)
      setWriteBlocker(null)
      clearTimeout(timer)
      visible = false
    }
  }, [])
  return <div className="offline-write-notice" role="status" aria-live="polite">{open ? t('offline.write') : ''}</div>
}
