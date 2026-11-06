import { useEffect, useId, useRef } from 'react'
import { useIsMobile } from './useIsMobile'

/**
 * On a phone the system Back closes the layer on top before it leaves the screen: screen → sheet → Back
 * closes the sheet, Back again leaves the screen. An open layer owns one history entry for itself (the same
 * URL, marked with `sheet` in the entry's state, and numbered the way the router numbers its own entries).
 *
 * - Back (popstate onto an entry that is not the layer's) closes the layer.
 * - Closing it from inside (a choice, the close button, the backdrop) steps back off its entry. If the
 *   layer closed because the screen moved on (a link inside it, a choice that navigates), its entry is no
 *   longer on top, so nothing is undone and the move is never cancelled.
 * - An entry left behind that way is skipped when Back reaches it later (no layer owns it any more).
 */
const open = new Set<string>()
/** Layers closed in the commit that is running; their entry may be taken over by the layer opened with them. */
const closing = new Set<string>()

type Mark = { sheet?: string; idx?: number; usr?: unknown; key?: string }
const mark = (): Mark => (window.history.state ?? {}) as Mark

let skipping = false
function installSkipper(): void {
  if (skipping) return
  skipping = true
  window.addEventListener('popstate', () => {
    const id = mark().sheet
    if (id && !open.has(id)) window.history.back()
  })
}

export function useSheetBack(active: boolean, onClose: () => void): void {
  const mobile = useIsMobile()
  const id = useId()
  const close = useRef(onClose)
  close.current = onClose
  const on = active && mobile
  useEffect(() => {
    if (!on) return
    installSkipper()
    const now = mark()
    closing.delete(id)
    if (now.sheet !== id) {
      const entry = { ...now, sheet: id, key: `${id}${String(Date.now())}` }
      // A layer that replaces another one (a sheet's choice opens a confirmation) takes over its entry instead of stacking a second one.
      if (now.sheet && closing.has(now.sheet)) window.history.replaceState(entry, '')
      else window.history.pushState({ ...entry, idx: (now.idx ?? 0) + 1 }, '')
    }
    open.add(id)
    const onPop = () => {
      if (mark().sheet !== id) close.current()
    }
    window.addEventListener('popstate', onPop)
    return () => {
      window.removeEventListener('popstate', onPop)
      open.delete(id)
      closing.add(id)
      // Deferred, so a layer that is closed and opened again in one commit (React re-runs effects) keeps its entry.
      queueMicrotask(() => {
        closing.delete(id)
        if (!open.has(id) && mark().sheet === id) window.history.back()
      })
    }
  }, [on, id])
}
