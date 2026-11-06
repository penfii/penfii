import { useSyncExternalStore } from 'react'

/** The width up to which Penfii shows its phone layout: a bottom tab bar and one full screen at a time. */
export const MOBILE_MAX_WIDTH = 640
const MOBILE_QUERY = `(max-width: ${String(MOBILE_MAX_WIDTH)}px)`

// An environment without matchMedia (a test DOM) is never a phone.
const probe = (): MediaQueryList | null => (typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(MOBILE_QUERY) : null)

/**
 * One MediaQueryList and one listener serve every component that asks. It exists only while somebody
 * is subscribed, so a window (or a test) that swaps `matchMedia` is picked up by the next subscriber.
 */
let list: MediaQueryList | null = null
const listeners = new Set<() => void>()
const notify = () => listeners.forEach((listener) => listener())

function subscribe(onChange: () => void): () => void {
  if (listeners.size === 0) {
    list = probe()
    list?.addEventListener('change', notify)
  }
  listeners.add(onChange)
  return () => {
    listeners.delete(onChange)
    if (listeners.size === 0) {
      list?.removeEventListener('change', notify)
      list = null
    }
  }
}

const snapshot = (): boolean => (list ?? probe())?.matches ?? false

export function useIsMobile(): boolean {
  return useSyncExternalStore(subscribe, snapshot, () => false)
}
