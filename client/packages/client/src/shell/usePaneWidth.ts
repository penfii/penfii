import { useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent } from 'react'
import {
  TICKET_DEFAULT,
  TICKET_MIN,
  TICKET_WIDTH_KEY,
  NAV_DEFAULT,
  NAV_MIN,
  NAV_WIDTH_KEY,
  THREAD_DEFAULT,
  THREAD_MIN,
  THREAD_WIDTH_KEY,
  STACKED_MAX_WIDTH,
  clampWidth,
  clearStoredWidth,
  detailColumnWidth,
  dragSample,
  fittedThreadDefault,
  isSecondPress,
  maxTicketWidth,
  navTrackWidth,
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

/**
 * The shown width of the thread and of the Ticket panel, for the panes beside them: the shell decides whether the context nav
 * steps aside from the thread, and the thread's room depends on the Ticket panel. Both have to follow a width while it is
 * dragged, not only the value stored when the drag ends.
 */
type LiveKind = 'thread' | 'ticket'
const liveWidths: Record<LiveKind, number | null> = { thread: null, ticket: null }
const liveListeners = new Set<() => void>()
function publishWidth(kind: LiveKind, width: number | null) {
  if (liveWidths[kind] === width) return
  liveWidths[kind] = width
  liveListeners.forEach((listener) => listener())
}
function subscribeWidths(listener: () => void) {
  liveListeners.add(listener)
  return () => {
    liveListeners.delete(listener)
  }
}
/** The width a mounted thread or Ticket pane shows, or null when none is mounted (or `kind` is null: nothing is followed). */
export function useLiveWidth(kind: LiveKind | null): number | null {
  return useSyncExternalStore(subscribeWidths, () => (kind ? liveWidths[kind] : null), () => null)
}

/**
 * Width of a resizable pane. The wanted width is kept (and remembered) as dragged; the shown width
 * is that wish clamped to the room the window leaves, so shrinking and growing the window restores it.
 * A second press on the handle within half a second restores the pane's default width (and forgets the stored one).
 * Until the person sets a width, the thread opens at its fitted default (`fittedThreadDefault`): narrower than
 * THREAD_DEFAULT where the window would otherwise leave the main column under MAIN_MIN beside it.
 * Thread and Ticket panes close when the drag passes the close-button floor, without storing that width.
 * `panel` is the open Detail Panel (the `panel` search parameter); a Ticket panel takes `ticketWidth`, or the width the
 * Ticket pane shows (its stored width when none is mounted).
 */
export function usePaneWidth(
  kind: PaneKind,
  options: { threadOpen: boolean; panel: string | null; ticketWidth?: number; onTooNarrow?: () => void; onUserResize?: (width: number) => void },
) {
  const { key, min, fallback } = PANES[kind]
  const preferred = useRef(readStoredWidth(key, fallback, min))
  // False while the pane keeps its default: nothing stored, or the default itself (earlier versions stored it on a reset).
  const chosen = useRef(preferred.current !== fallback)
  const activeDrag = useRef<(() => void) | null>(null)
  // No earlier press yet. Not 0: `timeStamp` counts from page load, so a first press within half a second of it would look doubled.
  const lastPointerDown = useRef(Number.NEGATIVE_INFINITY)
  const onTooNarrow = useRef(options.onTooNarrow)
  onTooNarrow.current = options.onTooNarrow
  // Told the width the person is setting (drag or keys) before it is shown, so the page can keep the handle in view.
  const onUserResize = useRef(options.onUserResize)
  onUserResize.current = options.onUserResize
  const [width, setWidth] = useState(preferred.current)
  // Only the thread follows the Ticket panel; the other panes would re-render on every change for nothing.
  const liveTicket = useLiveWidth(kind === 'thread' ? 'ticket' : null)
  const ticketWidth = options.ticketWidth ?? liveTicket ?? readStoredWidth(TICKET_WIDTH_KEY, TICKET_DEFAULT, TICKET_MIN)

  const clampToViewport = useCallback((value: number, asDefault = false) => {
    const viewport = window.innerWidth
    const detail = detailColumnWidth(viewport, options.panel, ticketWidth)
    const storedNav = readStoredWidth(NAV_WIDTH_KEY, NAV_DEFAULT, NAV_MIN)
    if (kind === 'ticket') {
      const nav = navTrackWidth(storedNav, shouldCollapseNav(viewport, storedNav, value))
      return clampWidth(value, min, maxTicketWidth(viewport, nav))
    }
    if (kind === 'nav') {
      const thread = options.threadOpen
        ? readStoredWidth(THREAD_WIDTH_KEY, THREAD_DEFAULT, THREAD_MIN)
        : 0
      // A nav that a side pane puts away opens over the page, so only the window limits it.
      const room = shouldCollapseNav(viewport, value, thread + detail) ? paneRoom('nav', viewport, 0, 0) : paneRoom('nav', viewport, thread, detail)
      return clampWidth(value, min, room)
    }
    const nav = navTrackWidth(storedNav, shouldCollapseNav(viewport, storedNav, value + detail))
    const wanted = asDefault && viewport > STACKED_MAX_WIDTH ? Math.min(value, fittedThreadDefault(viewport, nav, detail)) : value
    return clampWidth(wanted, min, paneRoom('thread', viewport, nav, detail))
  }, [kind, min, options.panel, ticketWidth, options.threadOpen])

  /** What the pane shows for `value` now: clamped to the room, and fitted while the thread keeps its default. */
  const shownWidth = useCallback((value: number) => clampToViewport(value, kind === 'thread' && !chosen.current), [clampToViewport, kind])

  useEffect(() => {
    const apply = () => setWidth(shownWidth(preferred.current))
    apply()
    window.addEventListener('resize', apply)
    return () => window.removeEventListener('resize', apply)
  }, [shownWidth])

  useEffect(() => () => activeDrag.current?.(), [])

  useLayoutEffect(() => {
    if (kind !== 'nav') publishWidth(kind, width)
  }, [kind, width])
  useEffect(() => (kind !== 'nav' ? () => publishWidth(kind, null) : undefined), [kind])

  function beginDrag(event: ReactPointerEvent<HTMLElement>, direction: 1 | -1) {
    const now = event.timeStamp
    const doubled = isSecondPress(lastPointerDown.current, now)
    lastPointerDown.current = now
    if (doubled) {
      event.preventDefault()
      activeDrag.current?.()
      chosen.current = false
      clearStoredWidth(key)
      preferred.current = fallback
      setWidth(shownWidth(fallback))
      return
    }
    event.preventDefault()
    activeDrag.current?.()
    const startX = event.clientX
    const pointerId = event.pointerId
    const before = { preferred: preferred.current, chosen: chosen.current }
    // From the width on screen, so the edge follows the pointer even where the stored or default width is cut down.
    const startWidth = shownWidth(preferred.current)
    // A press without a move (a click, the first press of a double click) changes and stores nothing.
    let moved = false
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
      preferred.current = before.preferred
      chosen.current = before.chosen
      setWidth(shownWidth(before.preferred))
      cleanup()
      onTooNarrow.current?.()
    }
    document.body.classList.add('is-resizing')
    const move = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return
      if (!moved && pointer.clientX === startX) return
      moved = true
      chosen.current = true
      const next = sample(pointer.clientX)
      if (next.close && kind !== 'nav') {
        closePane()
        return
      }
      const shown = next.close ? clampToViewport(min) : next.width
      preferred.current = shown
      onUserResize.current?.(shown)
      setWidth(shown)
    }
    const finish = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return
      if (!moved && pointer.clientX === startX) {
        cleanup()
        return
      }
      chosen.current = true
      const next = sample(pointer.clientX)
      if (next.close && kind !== 'nav') {
        closePane()
        return
      }
      const shown = next.close ? clampToViewport(min) : next.width
      preferred.current = shown
      onUserResize.current?.(shown)
      setWidth(shown)
      writeStoredWidth(key, shown)
      cleanup()
    }
    const cancel = () => {
      if (!moved) {
        cleanup()
        return
      }
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

  /** Keyboard resizing: `delta` px wider (negative: narrower), kept inside the pane's limits and remembered. Never closes the pane. */
  function nudge(delta: number) {
    const next = clampToViewport(shownWidth(preferred.current) + delta)
    chosen.current = true
    preferred.current = next
    onUserResize.current?.(next)
    setWidth(next)
    writeStoredWidth(key, next)
  }

  return { width, min, max: clampToViewport(1_000_000), beginDrag, nudge }
}
