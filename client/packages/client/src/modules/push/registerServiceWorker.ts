import { useEffect } from 'react'
import { useNavigate } from 'react-router'

export const SERVICE_WORKER_URL = '/sw.js'
const NAVIGATE_MESSAGE = 'penfii.navigate'

/**
 * Registers the service worker that shows pushes and handles clicks on them. It has no fetch handler
 * and no cache (see public/sw.js), so it changes nothing about how pages and the API load. A browser
 * without service workers, or a failed registration, leaves the app as it was.
 */
export function registerServiceWorker(): void {
  if (typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return
  const register = () => {
    navigator.serviceWorker.register(SERVICE_WORKER_URL, { scope: '/' }).catch(() => undefined)
  }
  if (document.readyState === 'complete') register()
  else window.addEventListener('load', register, { once: true })
}

/** A tap on a notification asks the open window to move to the item (no reload). */
export function usePushNavigation(): void {
  const navigate = useNavigate()
  useEffect(() => {
    if (typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return
    const container = navigator.serviceWorker
    const onMessage = (event: MessageEvent) => {
      const data = event.data as { type?: string; path?: unknown } | null
      if (data?.type !== NAVIGATE_MESSAGE || typeof data.path !== 'string') return
      // Only a path inside the app.
      if (!data.path.startsWith('/') || data.path.startsWith('//')) return
      void navigate(data.path)
    }
    container.addEventListener('message', onMessage)
    return () => container.removeEventListener('message', onMessage)
  }, [navigate])
}
