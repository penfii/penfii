import { useCallback, useEffect, useRef, type PointerEvent } from 'react'

const HOLD_MS = 450
const SLOP_PX = 10

/** A touch held still on an element for a moment. Returns the handlers to spread on it; a mouse never triggers it. */
export function useLongPress(onPress: () => void, enabled: boolean) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const origin = useRef<{ x: number; y: number } | null>(null)
  const cancel = useCallback(() => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
    origin.current = null
  }, [])
  // A hold that is still running when the element goes away, or when the gesture is switched off, must not fire.
  useEffect(() => {
    if (!enabled) cancel()
    return cancel
  }, [enabled, cancel])
  if (!enabled) return {}
  return {
    onPointerDown: (event: PointerEvent) => {
      if (event.pointerType !== 'touch') return
      cancel()
      origin.current = { x: event.clientX, y: event.clientY }
      timer.current = setTimeout(() => {
        timer.current = null
        onPress()
      }, HOLD_MS)
    },
    onPointerMove: (event: PointerEvent) => {
      const start = origin.current
      if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) > SLOP_PX) cancel()
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    // The system menu a long press opens on some phones would cover the sheet.
    onContextMenu: (event: { preventDefault: () => void }) => event.preventDefault(),
  }
}
