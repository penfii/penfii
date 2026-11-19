import { useSyncExternalStore } from 'react'

/** Below this width a Table cannot hold its columns without a sideways scroll, so it draws its compact rows. */
export const COMPACT_TABLE_MAX_WIDTH = 900
const QUERY = `(max-width: ${String(COMPACT_TABLE_MAX_WIDTH)}px)`

const probe = (): MediaQueryList | null => (typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(QUERY) : null)

function subscribe(onChange: () => void): () => void {
  const list = probe()
  list?.addEventListener('change', onChange)
  return () => list?.removeEventListener('change', onChange)
}

export function useCompactWidth(): boolean {
  return useSyncExternalStore(subscribe, () => probe()?.matches ?? false, () => false)
}
