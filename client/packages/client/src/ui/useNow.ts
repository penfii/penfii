import { useEffect, useState } from 'react'

/**
 * The current time as epoch milliseconds, read again every `intervalMs` and when a hidden tab comes
 * back. A component that shows "today", "overdue" or "in the past" takes its clock from here instead of
 * calling `Date.now()` while it renders: the value then stays the same within one render, changes only
 * when the clock is meant to move, and a test moves it with fake timers.
 */
export function useNow(intervalMs = 60_000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const tick = () => setNow(Date.now())
    const timer = window.setInterval(tick, intervalMs)
    const visible = () => {
      if (document.visibilityState === 'visible') tick()
    }
    document.addEventListener('visibilitychange', visible)
    return () => {
      window.clearInterval(timer)
      document.removeEventListener('visibilitychange', visible)
    }
  }, [intervalMs])
  return now
}
