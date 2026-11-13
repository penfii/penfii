import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useI18n } from '../i18n/I18nProvider'
import './notices.css'

/**
 * One place for the result of an action that has no row of its own to show it in. The latest
 * notice replaces the one before. A plain result closes by itself after SUCCESS_MS, one with an
 * action (Undo) after UNDO_MS, and an `error` or a `sticky` one stays until it is dismissed: a result that matters
 * (a failed save, a partly failed bulk change, a permission change) is never left to a timer.
 * The timer waits while the pointer or keyboard focus is on the notice.
 */
export const SUCCESS_MS = 4000
export const UNDO_MS = 8000

export type NoticeInput = {
  kind?: 'success' | 'info' | 'error'
  message: string
  /** Stays until dismissed, like an error: for a result the person must not miss (a permission change, a bulk outcome). */
  sticky?: boolean
  /** A button on the notice, reachable by keyboard; the notice closes first, then `run` is called. */
  action?: { label: string; run: () => void }
}
type Notice = NoticeInput & { id: number }

let current: Notice | null = null
let counter = 0
const listeners = new Set<() => void>()
const emit = () => listeners.forEach((fn) => fn())
const subscribe = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn) } }

export function notify(input: NoticeInput): void {
  counter += 1
  current = { ...input, id: counter }
  emit()
}

export function dismissNotice(): void {
  if (!current) return
  current = null
  emit()
}

/** How long a notice stays before it closes on its own; null means until dismissed. */
export function noticeDuration(notice: NoticeInput): number | null {
  if (notice.kind === 'error' || notice.sticky) return null
  return notice.action ? UNDO_MS : SUCCESS_MS
}

export function NoticeHost() {
  const { t } = useI18n()
  const notice = useSyncExternalStore(subscribe, () => current, () => null)
  const [held, setHeld] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const id = notice?.id
  const duration = notice ? noticeDuration(notice) : null

  // A new notice is held only if the pointer or focus is already on the host (the old one may have just been removed under it); the timer restarts whenever the hold ends.
  useEffect(() => {
    const host = root.current
    setHeld(Boolean(host && (host.matches(':hover') || host.contains(document.activeElement))))
  }, [id])
  useEffect(() => {
    if (id === undefined || duration === null || held) return
    const timer = setTimeout(dismissNotice, duration)
    return () => clearTimeout(timer)
  }, [id, duration, held])
  // With a ticket panel open, a notice sits inside the panel just above its comment box, so it never covers what is being typed.
  useEffect(() => {
    const host = root.current
    if (!host) return
    for (const name of ['--notice-bottom', '--notice-left', '--notice-right']) host.style.removeProperty(name)
    if (id === undefined || window.matchMedia?.('(max-width: 640px)').matches) return
    const composer = document.querySelector('.ticket-panel .comment-composer')
    const panel = composer?.closest('.ticket-panel')
    if (!composer || !panel) return
    const box = composer.getBoundingClientRect()
    const area = panel.getBoundingClientRect()
    host.style.setProperty('--notice-bottom', `${String(Math.round(window.innerHeight - box.top + 8))}px`)
    host.style.setProperty('--notice-left', `${String(Math.round(area.left))}px`)
    host.style.setProperty('--notice-right', `${String(Math.round(window.innerWidth - area.right))}px`)
  }, [id])
  useEffect(() => () => { current = null }, [])

  return (
    <div
      ref={root}
      className="notice-host"
      role="status"
      aria-live="polite"
      onMouseEnter={() => setHeld(true)}
      onMouseLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={(event) => { if (!root.current?.contains(event.relatedTarget as Node | null)) setHeld(false) }}
      onKeyDown={(event) => { if (event.key === 'Escape') dismissNotice() }}
    >
      {notice ? (
        <div key={notice.id} className={`notice notice-${notice.kind ?? 'success'}`}>
          <span className="notice-text">{notice.message}</span>
          {notice.action ? (
            <button type="button" className="notice-action" onClick={() => { const run = notice.action?.run; dismissNotice(); run?.() }}>{notice.action.label}</button>
          ) : null}
          {duration === null ? <button type="button" className="notice-action" onClick={dismissNotice}>{t('notice.dismiss')}</button> : null}
        </div>
      ) : null}
    </div>
  )
}

export type ToggleMessages = { on: string; off: string; error: string; undo: string }

/**
 * Applies an on/off change and says what happened. Turning something on is a plain result; turning
 * it off offers Undo, which switches it back on (and is itself not undoable). A failure is kept
 * until dismissed. `apply` is the same mutation both ways, so it keeps working if the caller has
 * been unmounted by the time Undo is pressed.
 */
export async function announceToggle(next: boolean, apply: (next: boolean) => Promise<unknown>, messages: ToggleMessages, undoable = true): Promise<void> {
  try {
    await apply(next)
  } catch {
    notify({ kind: 'error', message: messages.error })
    return
  }
  if (next) notify({ message: messages.on })
  else notify({ message: messages.off, action: undoable ? { label: messages.undo, run: () => void announceToggle(true, apply, messages, false) } : undefined })
}
