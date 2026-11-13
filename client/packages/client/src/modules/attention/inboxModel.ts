import type { InboxItem, InboxType, InboxView } from '@penfii/contracts'

/** The types a person can filter by, in chip order. A notification has one kind; `watching` is not one of them. */
export const INBOX_TYPES: readonly InboxType[] = ['mentions', 'replies', 'assignments', 'reminders', 'invitations']
/** How many type chips show before "More". `읽지 않음` is its own chip next to them. */
export const VISIBLE_TYPE_CHIPS = 4

/**
 * What the address says about the Inbox screen: the View, the type Filter (several, OR) and the
 * unread Filter. View Options (show read, show snoozed, group by day) are a local preference and are
 * not part of it.
 *
 *   ?view=inbox|archived    the View (absent: the Inbox; always written by the app)
 *   ?type=mentions,replies  types, OR; absent: every type
 *   ?unread=1               only unread, AND with the types
 *
 * `?box=` is what links used before: a type, `unread`, `all`, or the retired `watching` (opened
 * without a type, so the link still works).
 */
export type InboxLocation = { view: InboxView; types: InboxType[]; unread: boolean }

const TYPE_SET = new Set<string>(INBOX_TYPES)

export function isInboxType(value: string): value is InboxType {
  return TYPE_SET.has(value)
}

function orderedTypes(values: Iterable<string>): InboxType[] {
  const wanted = new Set(values)
  return INBOX_TYPES.filter((type) => wanted.has(type))
}

export function parseInboxLocation(params: URLSearchParams): InboxLocation {
  const view: InboxView = params.get('view') === 'archived' ? 'archived' : 'inbox'
  const hasNewKeys = params.has('type') || params.has('unread') || params.has('view')
  if (!hasNewKeys) {
    const box = params.get('box')
    if (box === 'unread') return { view, types: [], unread: true }
    if (box && isInboxType(box)) return { view, types: [box], unread: false }
    return { view, types: [], unread: false }
  }
  const types = orderedTypes((params.get('type') ?? '').split(',').map((v) => v.trim()).filter(isInboxType))
  const unread = params.get('unread') === '1'
  return { view, types, unread }
}

/** The query string (with the leading `?`, or empty) for a location. */
export function inboxSearch(location: InboxLocation): string {
  const params = new URLSearchParams()
  // Always written: on the phone a bare /inbox is the Nav list, and `view` is what opens the content.
  params.set('view', location.view)
  if (location.types.length > 0) params.set('type', orderedTypes(location.types).join(','))
  if (location.unread && location.view === 'inbox') params.set('unread', '1')
  const text = params.toString()
  return text ? `?${text}` : ''
}

export function toggleType(location: InboxLocation, type: InboxType): InboxLocation {
  const has = location.types.includes(type)
  const next = has ? location.types.filter((t) => t !== type) : [...location.types, type]
  return { ...location, types: orderedTypes(next) }
}

/** The View Options. Kept on this device; they never reach the server or the address. */
export type InboxViewOptions = { showRead: boolean; showSnoozed: boolean; groupByDay: boolean }
export const DEFAULT_VIEW_OPTIONS: InboxViewOptions = { showRead: true, showSnoozed: false, groupByDay: true }

/** The server has one read axis, `state`. The unread Filter wins over "show read". */
export function readState(unreadFilter: boolean, showRead: boolean): 'all' | 'unread' {
  return unreadFilter || !showRead ? 'unread' : 'all'
}

/**
 * Which empty screen, if the list has no item: nothing was ever filtered out (the Inbox is empty), or
 * a condition that narrows the result is on (no match). Grouping by day does not change the result.
 */
export function emptyKind(location: InboxLocation, options: InboxViewOptions): 'none' | 'filtered' {
  if (location.view === 'archived') return location.types.length > 0 ? 'filtered' : 'none'
  const narrowed = location.types.length > 0 || location.unread || !options.showRead || options.showSnoozed
  return narrowed ? 'filtered' : 'none'
}

export type DayGroup = { key: string; day: 'today' | 'yesterday' | 'date'; date: string; items: InboxItem[] }

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

/**
 * Groups consecutive items by calendar day in the person's time zone. `at` is `createdAt` in the
 * Inbox and `archivedAt` in the archive. Items stay in the order given, so a page loaded later
 * continues the last group instead of repeating its title.
 */
export function groupByDay(items: InboxItem[], at: (item: InboxItem) => string, now = new Date()): DayGroup[] {
  const today = dayKey(now)
  const yesterdayDate = new Date(now)
  yesterdayDate.setDate(yesterdayDate.getDate() - 1)
  const yesterday = dayKey(yesterdayDate)
  const groups: DayGroup[] = []
  for (const item of items) {
    const stamp = at(item)
    const parsed = new Date(stamp)
    const key = Number.isNaN(parsed.getTime()) ? stamp : dayKey(parsed)
    const last = groups[groups.length - 1]
    if (last?.key === key) {
      last.items.push(item)
      continue
    }
    groups.push({ key, day: key === today ? 'today' : key === yesterday ? 'yesterday' : 'date', date: stamp, items: [item] })
  }
  return groups
}
