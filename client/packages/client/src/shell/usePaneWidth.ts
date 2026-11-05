import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  TICKET_DEFAULT,
  TICKET_MIN,
  TICKET_WIDTH_KEY,
  DETAIL_WIDTH,
  NAV_DEFAULT,
  NAV_MIN,
  NAV_WIDTH_KEY,
  THREAD_DEFAULT,
  THREAD_MIN,
  THREAD_WIDTH_KEY,
  clampWidth,
  dragSample,
  maxTicketWidth,
  paneRoom,
  readStoredWidth,
  shouldCollapseNav,
  writeStoredWidth,
} from './paneWidth'

type PaneKind = 'nav' | 'thread' | 'ticket'

const PANES = {
  nav: { key: NAV_WIDTH_KEY, min: NAV_MIN, fallback: NAV_DEFAULT },
  thread: { key: THREAD_WIDTH_KEY, min: THREAD_MIN, fallback: THREAD_DEFAULT },
  ticket: { key: TICKET_WIDTH_KEY, min: TICKET_MIN, fallback: TICKET_DEFAULT },
} as const

const DOUBLE_CLICK_MS = 500

/**
 * Width of a resizable pane. The wanted width is kept (and remembered) as dragged; the shown width
 * is that wish clamped to the room the window leaves, so shrinking and growing the window restores it.
 * A second press on the handle within half a second restores the pane's default width.
 * Thread and Ticket panes close when the drag passes the close-button floor, without storing that width.
 * `detailWidth` is the width of the detail column when something other than the default 320px is open.
 */
export function usePaneWidth(
  kind: PaneKind,
  options: { threadOpen: boolean; detailOpen: boolean; detailWidth?: number; onTooNarrow?: () => void },
) {
  const { key, min, fallback } = PANES[kind]
  const preferred = useRef(readStoredWidth(key, fallback, min))
  const activeDrag = useRef<(() => void) | null>(null)
  const lastPointerDown = useRef(0)
  const onTooNarrow = useRef(options.onTooNarrow)
  onTooNarrow.current = options.onTooNarrow
  const [width, setWidth] = useState(preferred.current)

  const clampToViewport = useCallback((value: number) => {
    const viewport = window.innerWidth
    const detail = options.detailWidth ?? (options.detailOpen ? DETAIL_WIDTH : 0)
    const storedNav = readStoredWidth(NAV_WIDTH_KEY, NAV_DEFAULT, NAV_MIN)
    if (kind === 'ticket') {
      const nav = shouldCollapseNav(viewport, storedNav, value) ? 0 : storedNav
      return clampWidth(value, min, maxTicketWidth(viewport, nav))
    }
    if (kind === 'nav') {
      const thread = options.threadOpen
        ? readStoredWidth(THREAD_WIDTH_KEY, THREAD_DEFAULT, THREAD_MIN)
        : 0
      return clampWidth(value, min, paneRoom('nav', viewport, thread, detail))
    }
    const nav = shouldCollapseNav(viewport, storedNav, value + detail) ? 0 : storedNav
    return clampWidth(value, min, paneRoom('thread', viewport, nav, detail))
  }, [kind, min, options.detailOpen, options.detailWidth, options.threadOpen])

  const applyWidth = useCallback((value: number, persist: boolean) => {
    const next = clampToViewport(value)
    preferred.current = next
    setWidth(next)
    if (persist) writeStoredWidth(key, next)
  }, [clampToViewport, key])

  useEffect(() => {
    const apply = () => setWidth(clampToViewport(preferred.current))
    apply()
    window.addEventListener('resize', apply)
    return () => window.removeEventListener('resize', apply)
  }, [clampToViewport])

  useEffect(() => () => activeDrag.current?.(), [])

  function beginDrag(event: ReactPointerEvent<HTMLElement>, direction: 1 | -1) {
    const now = event.timeStamp
    const doubled = now - lastPointerDown.current > 0 && now - lastPointerDown.current < DOUBLE_CLICK_MS
    lastPointerDown.current = now
    if (doubled) {
      event.preventDefault()
      activeDrag.current?.()
      applyWidth(fallback, true)
      return
    }
    event.preventDefault()
    activeDrag.current?.()
    const startX = event.clientX
    const pointerId = event.pointerId
    const startWidth = preferred.current
    let finished = false
    const cleanup = () => {
      if (finished) return
      finished = true
      document.body.classList.remove('is-resizing')
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', finish)
      window.removeEventListener('pointercancel', cancel)
      window.removeEventListener('blur', cancel)
      if (activeDrag.current === cleanup) activeDrag.current = null
    }
    const sample = (clientX: number) => dragSample(startWidth, (clientX - startX) * direction, min, clampToViewport(1_000_000))
    const closePane = () => {
      preferred.current = startWidth
      setWidth(clampToViewport(startWidth))
      cleanup()
      onTooNarrow.current?.()
    }
    document.body.classList.add('is-resizing')
    const move = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return
      const next = sample(pointer.clientX)
      if (next.close && kind !== 'nav') {
        closePane()
        return
      }
      const shown = next.close ? clampToViewport(min) : next.width
      preferred.current = shown
      setWidth(shown)
    }
    const finish = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return
      const next = sample(pointer.clientX)
      if (next.close && kind !== 'nav') {
        closePane()
        return
      }
      const shown = next.close ? clampToViewport(min) : next.width
      preferred.current = shown
      setWidth(shown)
      writeStoredWidth(key, shown)
      cleanup()
    }
    const cancel = () => {
      const next = clampToViewport(preferred.current)
      preferred.current = next
      setWidth(next)
      writeStoredWidth(key, next)
      cleanup()
    }
    activeDrag.current = cleanup
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('blur', cancel)
  }

  return { width, min, max: clampToViewport(1_000_000), beginDrag }
}
