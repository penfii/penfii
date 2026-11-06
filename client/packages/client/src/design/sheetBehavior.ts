import { useEffect, useRef, type RefObject } from 'react'

/**
 * What every bottom sheet and sheet-style dialog shares, so they behave the same way:
 * the page behind does not scroll (`html.m-scroll-locked`, counted so nested layers unlock only
 * when the last one closes). Focus and Escape are the layer's own job (see `Sheet`, native `<dialog>`).
 */
let locks = 0
const LOCK_CLASS = 'm-scroll-locked'

export function useScrollLock(active: boolean): void {
  useEffect(() => {
    if (!active) return
    locks += 1
    document.documentElement.classList.add(LOCK_CLASS)
    return () => {
      locks -= 1
      if (locks === 0) document.documentElement.classList.remove(LOCK_CLASS)
    }
  }, [active])
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/**
 * Moves focus into the layer when it opens, keeps Tab inside it, and gives focus back to whatever
 * had it before when it closes. With `inertRoot` the page behind is made inert for the keyboard and
 * assistive technology; it is live again before focus returns, because an inert element cannot take it.
 */
export function useFocusLayer(active: boolean, layer: RefObject<HTMLElement | null>, inertRoot?: string): void {
  const opener = useRef<HTMLElement | null>(null)
  useEffect(() => {
    if (!active) return
    const root = layer.current
    if (!root) return
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const behind = inertRoot ? document.getElementById(inertRoot) : null
    behind?.setAttribute('inert', '')
    const first = root.querySelector<HTMLElement>(FOCUSABLE)
    ;(first ?? root).focus({ preventScroll: true })
    function onKey(event: KeyboardEvent) {
      if (event.key !== 'Tab' || !root) return
      const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)]
      if (items.length === 0) {
        event.preventDefault()
        return
      }
      const at = items.indexOf(document.activeElement as HTMLElement)
      const next = event.shiftKey ? (at <= 0 ? items.length - 1 : at - 1) : at === items.length - 1 ? 0 : at + 1
      // Only wrap at the ends; the browser handles the middle.
      if ((event.shiftKey && at <= 0) || (!event.shiftKey && at === items.length - 1)) {
        event.preventDefault()
        items[next].focus()
      }
    }
    root.addEventListener('keydown', onKey)
    return () => {
      root.removeEventListener('keydown', onKey)
      behind?.removeAttribute('inert')
      const back = opener.current
      // The opener may have gone with the screen that closed the layer.
      if (back?.isConnected) back.focus({ preventScroll: true })
    }
  }, [active, layer, inertRoot])
}
