import type { CalendarDefinition, CalendarWeekday, GanttEdge, GanttItem, TicketSchedule } from '@penfii/contracts'
import { instantToLocalInput, localInputToInstant } from './zonedTime'

/*
 * What the Gantt draws, worked out from the list the server gives. Pure: no drawing, no requests.
 * Bars come from the planned schedule (start and the end the server derived from the calendar).
 */

export type GanttRow = { item: GanttItem; depth: number; hasChildren: boolean }

/** Tickets in tree order: each parent followed by its children, in the order the server listed them. */
export function ganttRows(items: GanttItem[]): GanttRow[] {
  const ids = new Set(items.map((item) => item.ticket.id))
  const children = new Map<string, GanttItem[]>()
  const roots: GanttItem[] = []
  for (const item of items) {
    if (item.parentId && ids.has(item.parentId)) children.set(item.parentId, [...(children.get(item.parentId) ?? []), item])
    else roots.push(item)
  }
  const rows: GanttRow[] = []
  const walk = (item: GanttItem, depth: number) => {
    const kids = children.get(item.ticket.id) ?? []
    rows.push({ item, depth, hasChildren: kids.length > 0 })
    for (const kid of kids) walk(kid, depth + 1)
  }
  for (const root of roots) walk(root, 0)
  return rows
}

export type Bar = { start: Date; end: Date; milestone: boolean }

/**
 * The planned bar of a Ticket, or null when it has none to draw (unplanned, or the end cannot be
 * derived). A schedule milestone takes no time and sits at a working instant: when its start is
 * not one (after closing time, on a day off) the server derives the next working instant as its
 * end, and that is where it is drawn, so the Gantt and the schedule table say the same thing.
 */
export function planBar(item: GanttItem): Bar | null {
  const schedule = item.schedule
  if (!schedule?.plannedStartAt) return null
  const start = new Date(schedule.plannedStartAt)
  if (schedule.isScheduleMilestone) {
    const at = schedule.plannedEndAt ? new Date(schedule.plannedEndAt) : start
    return { start: at, end: at, milestone: true }
  }
  if (!schedule.plannedEndAt) return null
  return { start, end: new Date(schedule.plannedEndAt), milestone: false }
}

export type UnplannedReason = 'NO_SCHEDULE' | 'NO_START' | 'NO_DURATION' | 'NO_CALENDAR' | 'BEYOND'

/** Why an open Task or Subtask has no bar; null when it has one or is not planned work. */
export function unplannedReason(item: GanttItem, hasCalendar: boolean): UnplannedReason | null {
  if (!['TASK', 'SUBTASK'].includes(item.ticket.hierarchyKind)) return null
  if (item.statusCategory === 'DONE' || item.statusCategory === 'CANCELED') return null
  const schedule = item.schedule
  if (!schedule) return 'NO_SCHEDULE'
  if (!schedule.plannedStartAt) return 'NO_START'
  if (!schedule.isScheduleMilestone && schedule.durationMinutes === null) return 'NO_DURATION'
  if (planBar(item)) return null
  return hasCalendar ? 'BEYOND' : 'NO_CALENDAR'
}

export type Axis = {
  from: number
  to: number
  /** Pixels per day. */
  dayWidth: number
  width: number
  days: Array<{ label: string; x: number; first: boolean }>
  x: (instant: Date | number | string) => number
}

const DAY_MS = 86_400_000

/** A day axis in the calendar's zone over every bar, with a day of room either side. */
export function buildAxis(dates: Array<Date | string>, timeZone: string, dayWidth = 28): Axis | null {
  const times = dates.map((d) => new Date(d).getTime()).filter((t) => Number.isFinite(t))
  if (times.length === 0) return null
  const startDay = instantToLocalInput(new Date(Math.min(...times)).toISOString(), timeZone).slice(0, 10)
  const endDay = instantToLocalInput(new Date(Math.max(...times)).toISOString(), timeZone).slice(0, 10)
  const addDays = (day: string, n: number) => new Date(Date.parse(`${day}T00:00:00Z`) + n * DAY_MS).toISOString().slice(0, 10)
  const first = addDays(startDay, -1)
  const last = addDays(endDay, 2)
  const origin = localInputToInstant(`${first}T00:00`, timeZone)
  if (!origin) return null
  const from = Date.parse(origin)
  const days: Axis['days'] = []
  let x = 0
  for (let day = first; day <= last && days.length < 800; day = addDays(day, 1)) {
    const at = localInputToInstant(`${day}T00:00`, timeZone)
    const left = at ? ((Date.parse(at) - from) / DAY_MS) * dayWidth : x
    days.push({ label: day, x: left, first: day.endsWith('-01') || days.length === 0 })
    x = left
  }
  const endInstant = localInputToInstant(`${last}T00:00`, timeZone)
  const to = endInstant ? Date.parse(endInstant) : from + DAY_MS
  return { from, to, dayWidth, width: ((to - from) / DAY_MS) * dayWidth, days, x: (instant) => ((new Date(instant).getTime() - from) / DAY_MS) * dayWidth }
}

/** Working minutes in the longest day of the week: what "a day" of duration means on this calendar. */
export function workDayMinutes(definition: CalendarDefinition | undefined): number {
  if (!definition) return 480
  const minutes = (clock: string) => Number(clock.slice(0, 2)) * 60 + Number(clock.slice(3, 5))
  const days: CalendarWeekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']
  const totals = days.map((day) => (definition.week[day] ?? []).reduce((sum, interval) => sum + minutes(interval.to) - minutes(interval.from), 0))
  return Math.max(...totals, 1)
}

/** The schedule with its start moved by whole days on the calendar's wall clock; null when the start is unknown. */
export function shiftStart(schedule: TicketSchedule, days: number, timeZone: string): TicketSchedule | null {
  if (!schedule.plannedStartAt) return null
  const local = instantToLocalInput(schedule.plannedStartAt, timeZone)
  const moved = new Date(Date.parse(`${local.slice(0, 10)}T00:00:00Z`) + days * DAY_MS).toISOString().slice(0, 10)
  const instant = localInputToInstant(`${moved}${local.slice(10)}`, timeZone)
  return instant ? { ...schedule, plannedStartAt: instant } : null
}

/** The schedule with its duration changed by whole working days, never below one minute; a node has no duration. */
export function resizeDuration(schedule: TicketSchedule, days: number, dayMinutes: number): TicketSchedule | null {
  if (schedule.isScheduleMilestone || schedule.durationMinutes === null) return null
  return { ...schedule, durationMinutes: Math.max(1, schedule.durationMinutes + days * dayMinutes) }
}

/** The schedule as the Ticket PATCH takes it: the derived end is not sent. */
export function scheduleBody(schedule: TicketSchedule) {
  return {
    plannedStartAt: schedule.plannedStartAt,
    durationMinutes: schedule.durationMinutes,
    isScheduleMilestone: schedule.isScheduleMilestone,
  }
}

/** A dependency whose successor is planned to start before its predecessor ends. */
export function planConflict(edge: GanttEdge, rows: GanttRow[]): boolean {
  const by = new Map(rows.map((row) => [row.item.ticket.id, row]))
  const from = by.get(edge.predecessorId)
  const to = by.get(edge.successorId)
  if (!from || !to || edge.waived || from.item.statusCategory === 'DONE' || from.item.statusCategory === 'CANCELED') return false
  const a = planBar(from.item)
  const b = planBar(to.item)
  return a !== null && b !== null && b.start.getTime() < a.end.getTime()
}

/** The dependencies whose plans disagree, for the note above the chart. The plan is the person's; nothing moves. */
export function planConflicts(edges: GanttEdge[], rows: GanttRow[]): GanttEdge[] {
  return edges.filter((edge) => planConflict(edge, rows))
}

/**
 * The elbow of a dependency arrow from the end of a predecessor to the start of a successor. With
 * room between them it goes right, down, right; without it (a conflict, or a hand-over on the same
 * day) it leaves the predecessor, turns between the two rows and comes into the successor from the left.
 */
export function edgePath(x1: number, y1: number, x2: number, y2: number, rowHeight: number): string {
  const p = (n: number) => String(Math.round(n * 10) / 10)
  if (x2 - x1 >= 16) {
    const mid = (x1 + x2) / 2
    return `M${p(x1)},${p(y1)} H${p(mid)} V${p(y2)} H${p(x2)}`
  }
  const turn = y1 + (y2 >= y1 ? rowHeight / 2 : -rowHeight / 2)
  return `M${p(x1)},${p(y1)} H${p(x1 + 8)} V${p(turn)} H${p(x2 - 8)} V${p(y2)} H${p(x2)}`
}

export type NonWorkingSpan = { x: number; width: number; holiday: boolean; name: string | null }

const WEEKDAYS: CalendarWeekday[] = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat']

/** The days of the axis on which the calendar has no working time: weekly days off and holidays. */
export function nonWorkingSpans(axis: Axis, definition: CalendarDefinition | undefined): NonWorkingSpan[] {
  if (!definition) return []
  const holidays = new Map(definition.holidays.map((holiday) => [holiday.date, holiday.name]))
  const spans: NonWorkingSpan[] = []
  axis.days.forEach((day, at) => {
    const next = axis.days[at + 1]
    const width = next ? next.x - day.x : axis.width - day.x
    if (width <= 0) return
    const holiday = holidays.has(day.label)
    const weekday = WEEKDAYS[new Date(`${day.label}T00:00:00Z`).getUTCDay()]
    const off = (definition.week[weekday] ?? []).length === 0
    if (holiday || off) spans.push({ x: day.x, width, holiday, name: holidays.get(day.label) ?? null })
  })
  return spans
}
