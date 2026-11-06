import { useCallback, useEffect } from 'react'
import { useLocation, useNavigate, useNavigationType } from 'react-router'
import { useIsMobile } from '../design/useIsMobile'

/**
 * Closing a detail on a phone must not leave a copy of it in the history. A Ticket panel or a Thread is a
 * location of its own (`?panel=…`, `/t/…`), opened with a push; closing it with another push would put the
 * list *after* it, and the phone's Back would walk into the closed detail again. So when a detail was
 * opened by a push from the very location it is closed to, closing is a step Back; otherwise (a link
 * from elsewhere, a reload, a shared URL) the entry is replaced by the place it closes to.
 * A wide window keeps the plain navigation it always had.
 */
const trail: { previous: string | null; current: string | null; type: string } = { previous: null, current: null, type: 'POP' }

/** Remembers where the last push came from. Mounted once by the phone shell. */
export function useTrackTrail(): void {
  const { pathname, search } = useLocation()
  const type = useNavigationType()
  useEffect(() => {
    const url = pathname + search
    // A sheet's own history entry (same URL) is not a place the person came from.
    if (trail.current === url) return
    trail.previous = trail.current
    trail.current = url
    trail.type = type
  }, [pathname, search, type])
}

/** Close a detail by going to `to` (a path with its query). */
export function useLeaveOverlay(): (to: string) => void {
  const navigate = useNavigate()
  const mobile = useIsMobile()
  return useCallback(
    (to: string) => {
      if (!mobile) void navigate(to)
      else if (trail.type === 'PUSH' && trail.previous === to) void navigate(-1)
      else void navigate(to, { replace: true })
    },
    [navigate, mobile],
  )
}
