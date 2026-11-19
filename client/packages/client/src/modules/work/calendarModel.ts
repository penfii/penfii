import type { GanttItem } from '@penfii/contracts'
import { planBar } from './ganttModel'
import { instantToLocalInput } from './zonedTime'

/*
 * What the month calendar draws, worked out from the Gantt list. Pure: no drawing, no requests.
 *
 * A date here is always a plain `YYYY-MM-DD` day. A due date (`dueOn`) already is one, so it is used as it is. A plan
 * is two instants; they are read as days on the Project's calendar zone (never the browser's), and a plan that ends at
 * exactly midnight ends on the day before, because midnight belongs to the next day.
 */

export type Day = string

const DAY_MS = 86_400_000

export function addDays(day: Day, count: number): Day {
  return new Date(Date.parse(`${day}T00:00:00Z`) + count * DAY_MS).toISOString().slice(0, 10)
}

/** The day an instant falls on, on the wall clock of `zone`. */
export function dayInZone(iso: string, zone: string): Day {
  return instantToLocalInput(iso, zone).slice(0, 10)
}

export type PlanSpan = { start: Day; end: Day; milestone: boolean }

/**
 * The days a plan covers, end day included. A schedule milestone is one day. An end at exactly 00:00 on the zone's clock
 * is the previous day, so "Mon 09:00 to Wed 00:00" ends Tuesday; the end never comes before the start.
 */
export function planSpan(item: GanttItem, zone: string): PlanSpan | null {
  const bar = planBar(item)
  if (!bar) return null
  const start = dayInZone(bar.start.toISOString(), zone)
  if (bar.milestone) {
    const day = dayInZone(bar.end.toISOString(), zone)
    return { start: day, end: day, milestone: true }
  }
  const endLocal = instantToLocalInput(bar.end.toISOString(), zone)
  const endDay = endLocal.slice(0, 10)
  const midnight = endLocal.slice(11) === '00:00' && bar.end.getTime() > bar.start.getTime()
  const end = midnight ? addDays(endDay, -1) : endDay
  return { start, end: end < start ? start : end, milestone: false }
}

export type Entry = {
  id: string
  key: string
  title: string
  kind: GanttItem['ticket']['hierarchyKind']
  category: GanttItem['statusCategory']
  /** The Ticket's own due date, as the plain day it is. */
  due: Day | null
  /** The planned period in the Project's zone; shown only when "plan" is on. */
  plan: PlanSpan | null
}

export function entriesOf(items: GanttItem[], zone: string): Entry[] {
  return items.map((item) => ({
    id: item.ticket.id,
    key: item.ticket.key,
    title: item.ticket.title,
    kind: item.ticket.hierarchyKind,
    category: item.statusCategory,
    due: item.dueOn ?? null,
    plan: planSpan(item, zone),
  }))
}

export type Filters = { hideDone: boolean; text: string }

/**
 * The conditions the Gantt and the calendar share, with one meaning in both: Completed and Canceled work can be hidden,
 * and a text matches the key or the title. Nothing else is offered in either, so a switch never changes what is shown.
 */
export function filterItems(items: GanttItem[], filters: Filters): GanttItem[] {
  const needle = filters.text.trim().toLowerCase()
  return items.filter((item) => {
    if (filters.hideDone && (item.statusCategory === 'DONE' || item.statusCategory === 'CANCELED')) return false
    if (needle === '') return true
    return item.ticket.key.toLowerCase().includes(needle) || item.ticket.title.toLowerCase().includes(needle)
  })
}

export type UnscheduledReason = 'NO_SCHEDULE' | 'NO_START' | 'NO_DURATION' | 'NO_CALENDAR' | 'BEYOND'

export type Unscheduled = { item: GanttItem; reason: UnscheduledReason }

/**
 * Why a Ticket has nothing to put on the calendar, or null when it has a date: a due date, or a plan whose end could be worked out.
 * This is the calendar's own rule, separate from the Gantt's "unplanned" list: it says nothing about what the Gantt draws, and it
 * does not look at the State (Completed and Canceled work is hidden or shown by the shared filter, like everything else).
 */
export function unscheduledReason(item: GanttItem, hasCalendar: boolean): UnscheduledReason | null {
  if (item.dueOn != null) return null
  const schedule = item.schedule
  if (!schedule) return 'NO_SCHEDULE'
  if (!schedule.plannedStartAt) return 'NO_START'
  if (!schedule.isScheduleMilestone && schedule.durationMinutes === null) return 'NO_DURATION'
  if (planBar(item)) return null
  return hasCalendar ? 'BEYOND' : 'NO_CALENDAR'
}

/** Tasks and Subtasks with no date (an Epic only summarizes its children), each with the reason. */
export function unscheduled(items: GanttItem[], hasCalendar: boolean): Unscheduled[] {
  return items.flatMap((item) => {
    if (item.ticket.hierarchyKind === 'EPIC') return []
    const reason = unscheduledReason(item, hasCalendar)
    return reason ? [{ item, reason }] : []
  })
}

// ---- Months and weeks ------------------------------------------------------------------------

/** `YYYY-MM` of a day. */
export const monthOf = (day: Day): string => day.slice(0, 7)

export function shiftMonth(month: string, count: number): string {
  const [year, number] = month.split('-').map(Number) as [number, number]
  const moved = new Date(Date.UTC(year, number - 1 + count, 1))
  return `${String(moved.getUTCFullYear()).padStart(4, '0')}-${String(moved.getUTCMonth() + 1).padStart(2, '0')}`
}

/** A real calendar day written `YYYY-MM-DD`. */
export function isDay(value: string | null): value is Day {
  if (value === null || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false
  const parsed = new Date(`${value}T00:00:00Z`)
  return !Number.isNaN(parsed.getTime()) && parsed.toISOString().slice(0, 10) === value
}

/** The day both views start from when only a month is known: its 15th, the middle of the month. */
export const middleOf = (month: string): Day => `${month}-15`

export const isMonth = (value: string | null): value is string => value !== null && /^\d{4}-(0[1-9]|1[0-2])$/.test(value)

/**
 * The first weekday of the week: Monday for Korean, otherwise what the locale says (Intl's week info), otherwise Sunday.
 * 0 is Sunday, 1 is Monday.
 */
export function weekStartFor(locale: string): 0 | 1 {
  if (locale.toLowerCase().startsWith('ko')) return 1
  try {
    const info = (new Intl.Locale(locale) as unknown as { getWeekInfo?: () => { firstDay: number }; weekInfo?: { firstDay: number } })
    const first = info.getWeekInfo?.().firstDay ?? info.weekInfo?.firstDay
    return first === 1 ? 1 : 0
  } catch {
    return 0
  }
}

/** The weeks that show a month: whole weeks from the one holding the 1st to the one holding the last day. */
export function monthWeeks(month: string, weekStart: 0 | 1): Day[][] {
  const first = `${month}-01`
  const weekday = new Date(`${first}T00:00:00Z`).getUTCDay()
  const lead = (weekday - weekStart + 7) % 7
  const length = new Date(Date.UTC(Number(month.slice(0, 4)), Number(month.slice(5, 7)), 0)).getUTCDate()
  const total = Math.ceil((lead + length) / 7) * 7
  const days = Array.from({ length: total }, (_, index) => addDays(first, index - lead))
  return Array.from({ length: total / 7 }, (_, row) => days.slice(row * 7, row * 7 + 7))
}

export type Piece = {
  entry: Entry
  lane: number
  /** Column (0-6) of the first and last day of this piece inside the week. */
  from: number
  to: number
  /** The plan began before this week / goes on after it. */
  openLeft: boolean
  openRight: boolean
}

/** The plans that touch a week as bars, each on a lane of its own where they overlap (earlier start, then longer, first). */
export function weekPieces(week: Day[], entries: Entry[]): { pieces: Piece[]; lanes: number } {
  const first = week[0] as Day
  const last = week[6] as Day
  const touching = entries
    .filter((entry) => entry.plan && entry.plan.start <= last && entry.plan.end >= first)
    .sort((a, b) => (a.plan as PlanSpan).start.localeCompare((b.plan as PlanSpan).start) || (b.plan as PlanSpan).end.localeCompare((a.plan as PlanSpan).end) || a.key.localeCompare(b.key))
  const laneEnds: number[] = []
  const pieces: Piece[] = []
  for (const entry of touching) {
    const plan = entry.plan as PlanSpan
    const from = plan.start >= first ? week.indexOf(plan.start) : 0
    const to = week.reduce((found, day, index) => (day <= plan.end ? index : found), 0)
    let lane = laneEnds.findIndex((end) => end < from)
    if (lane === -1) {
      lane = laneEnds.length
      laneEnds.push(to)
    } else {
      laneEnds[lane] = to
    }
    pieces.push({ entry, lane, from, to, openLeft: plan.start < first, openRight: plan.end > last })
  }
  return { pieces, lanes: laneEnds.length }
}

/** The Tickets due on each day, by key. */
export function dueByDay(entries: Entry[]): Map<Day, Entry[]> {
  const map = new Map<Day, Entry[]>()
  for (const entry of entries) {
    if (!entry.due) continue
    map.set(entry.due, [...(map.get(entry.due) ?? []), entry])
  }
  for (const list of map.values()) list.sort((a, b) => a.key.localeCompare(b.key, undefined, { numeric: true }))
  return map
}

/** What happens on a day: the Tickets due, and (when plans are shown) the plans that cover it. */
export function dayEntries(day: Day, entries: Entry[], withPlans: boolean): { due: Entry[]; plans: Entry[] } {
  const byKey = (a: Entry, b: Entry) => a.key.localeCompare(b.key, undefined, { numeric: true })
  return {
    due: entries.filter((entry) => entry.due === day).sort(byKey),
    plans: withPlans ? entries.filter((entry) => entry.plan && entry.plan.start <= day && entry.plan.end >= day).sort(byKey) : [],
  }
}
