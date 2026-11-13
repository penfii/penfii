import { useCallback, useState } from 'react'
import { DEFAULT_VIEW_OPTIONS, type InboxViewOptions } from './inboxModel'

export const VIEW_OPTIONS_KEY = 'penfii.inbox.viewOptions'

function read(): InboxViewOptions {
  try {
    const raw = localStorage.getItem(VIEW_OPTIONS_KEY)
    if (!raw) return DEFAULT_VIEW_OPTIONS
    const parsed = JSON.parse(raw) as Partial<InboxViewOptions>
    return {
      showRead: typeof parsed.showRead === 'boolean' ? parsed.showRead : DEFAULT_VIEW_OPTIONS.showRead,
      showSnoozed: typeof parsed.showSnoozed === 'boolean' ? parsed.showSnoozed : DEFAULT_VIEW_OPTIONS.showSnoozed,
      groupByDay: typeof parsed.groupByDay === 'boolean' ? parsed.groupByDay : DEFAULT_VIEW_OPTIONS.groupByDay,
    }
  } catch {
    // Blocked or damaged storage: the defaults apply for this visit.
    return DEFAULT_VIEW_OPTIONS
  }
}

/** The Inbox View Options, kept in this browser only (not on the server, not in the address). One setting for the Inbox and the archive. */
export function useInboxViewOptions(): [InboxViewOptions, (patch: Partial<InboxViewOptions>) => void] {
  const [options, setOptions] = useState<InboxViewOptions>(read)
  const update = useCallback((patch: Partial<InboxViewOptions>) => {
    setOptions((current) => {
      const next = { ...current, ...patch }
      try {
        localStorage.setItem(VIEW_OPTIONS_KEY, JSON.stringify(next))
      } catch {
        // The choice still applies for this visit.
      }
      return next
    })
  }, [])
  return [options, update]
}
