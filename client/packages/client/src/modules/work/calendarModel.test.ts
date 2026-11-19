import { describe, expect, it } from 'vitest'
import type { GanttItem } from '@penfii/contracts'
import { addDays, dayEntries, dayInZone, dueByDay, entriesOf, filterItems, isMonth, monthWeeks, planSpan, shiftMonth, unscheduled, weekPieces, weekStartFor } from './calendarModel'

function item(key: string, extra: Partial<GanttItem> & { start?: string; end?: string | null; milestone?: boolean } = {}): GanttItem {
  const { start, end, milestone, ...rest } = extra
  return {
    ticket: { id: key, key, title: `Title ${key}`, hierarchyKind: 'TASK' },
    parentId: null,
    channelId: null,
    statusCategory: 'IN_PROGRESS',
    assigneeName: null,
    revisionNo: 1,
    schedule: start ? { plannedStartAt: start, durationMinutes: 480, isScheduleMilestone: milestone ?? false, plannedEndAt: end ?? null } : null,
    ...rest,
  } as GanttItem
}

describe('days of a plan on the Project zone', () => {
  it('reads instants as days on the zone, not on the browser or in UTC', () => {
    // 2026-11-02T20:00Z is already Tuesday the 3rd in Seoul.
    expect(dayInZone('2026-11-02T20:00:00Z', 'Asia/Seoul')).toBe('2026-11-03')
    expect(dayInZone('2026-11-02T20:00:00Z', 'America/Los_Angeles')).toBe('2026-11-02')
  })

  it('includes the end day, and ends a plan that ends at midnight on the day before', () => {
    // Seoul: Mon 09:00 to Wed 18:00 -> Mon..Wed
    expect(planSpan(item('A', { start: '2026-11-02T00:00:00Z', end: '2026-11-04T09:00:00Z' }), 'Asia/Seoul')).toEqual({ start: '2026-11-02', end: '2026-11-04', milestone: false })
    // Ends exactly at Wednesday 00:00 in Seoul (= Tuesday 15:00Z): the last day is Tuesday.
    expect(planSpan(item('B', { start: '2026-11-02T00:00:00Z', end: '2026-11-03T15:00:00Z' }), 'Asia/Seoul')).toEqual({ start: '2026-11-02', end: '2026-11-03', milestone: false })
    // The same instants are different days in another zone: the Project's zone decides.
    expect(planSpan(item('B', { start: '2026-11-02T00:00:00Z', end: '2026-11-03T15:00:00Z' }), 'America/Los_Angeles')?.end).toBe('2026-11-03')
  })

  it('puts a schedule milestone on one day, and gives nothing for a plan that cannot be drawn', () => {
    expect(planSpan(item('M', { start: '2026-11-06T15:00:00Z', end: '2026-11-06T15:00:00Z', milestone: true }), 'Asia/Seoul')).toEqual({ start: '2026-11-07', end: '2026-11-07', milestone: true })
    expect(planSpan(item('N', { start: '2026-11-02T00:00:00Z', end: null }), 'Asia/Seoul')).toBeNull()
    expect(planSpan(item('O'), 'Asia/Seoul')).toBeNull()
  })

  it('keeps a due date as the plain day it is', () => {
    const [entry] = entriesOf([item('D', { dueOn: '2026-11-21' })], 'Pacific/Kiritimati')
    expect(entry?.due).toBe('2026-11-21')
    expect(entry?.plan).toBeNull()
  })
})

describe('months and weeks', () => {
  it('starts on Monday for Korean and shows whole weeks, with days of the neighbouring months', () => {
    expect(weekStartFor('ko-KR')).toBe(1)
    const weeks = monthWeeks('2026-11', 1)
    expect(weeks[0]?.[0]).toBe('2026-10-26')
    // November 2026 starts on a Sunday, so a Monday-first grid needs six weeks.
    expect(weeks.at(-1)?.[6]).toBe('2026-12-06')
    expect(weeks).toHaveLength(6)
    expect(monthWeeks('2026-11', 0)[0]?.[0]).toBe('2026-11-01')
    expect(monthWeeks('2026-02', 0)).toHaveLength(4)
    expect(weeks.every((week) => week.length === 7)).toBe(true)
  })

  it('moves by month across a year and checks the month text', () => {
    expect(shiftMonth('2026-12', 1)).toBe('2027-01')
    expect(shiftMonth('2026-01', -1)).toBe('2025-12')
    expect(isMonth('2026-13')).toBe(false)
    expect(isMonth('2026-11')).toBe(true)
    expect(addDays('2026-02-28', 1)).toBe('2026-03-01')
  })
})

describe('plan bars in a week', () => {
  const week = ['2026-11-02', '2026-11-03', '2026-11-04', '2026-11-05', '2026-11-06', '2026-11-07', '2026-11-08']
  const entries = entriesOf(
    [
      item('A', { start: '2026-10-30T00:00:00Z', end: '2026-11-04T09:00:00Z' }),
      item('B', { start: '2026-11-03T00:00:00Z', end: '2026-11-04T09:00:00Z' }),
      item('C', { start: '2026-11-06T00:00:00Z', end: '2026-11-12T09:00:00Z' }),
    ],
    'Asia/Seoul',
  )

  it('splits a plan at the week edge, marks it as continuing and puts overlaps on their own lanes', () => {
    const { pieces, lanes } = weekPieces(week, entries)
    const by = Object.fromEntries(pieces.map((piece) => [piece.entry.key, piece]))
    expect(by.A).toMatchObject({ from: 0, to: 2, openLeft: true, openRight: false, lane: 0 })
    expect(by.B).toMatchObject({ from: 1, to: 2, lane: 1 })
    expect(by.C).toMatchObject({ from: 4, to: 6, openLeft: false, openRight: true })
    expect(lanes).toBe(2)
  })

  it('lists on a day the Tickets due and, only when plans are shown, the plans that cover it', () => {
    const withDue = entriesOf([item('A', { start: '2026-11-02T00:00:00Z', end: '2026-11-04T09:00:00Z', dueOn: '2026-11-03' }), item('Z', { dueOn: '2026-11-03' })], 'Asia/Seoul')
    expect(dayEntries('2026-11-03', withDue, false)).toMatchObject({ due: [{ key: 'A' }, { key: 'Z' }], plans: [] })
    expect(dayEntries('2026-11-03', withDue, true).plans.map((entry) => entry.key)).toEqual(['A'])
    expect(dueByDay(withDue).get('2026-11-03')).toHaveLength(2)
  })
})

describe('shared filters and unscheduled work', () => {
  const done = item('D', { statusCategory: 'DONE', dueOn: '2026-11-01' })
  const open = item('O', { dueOn: '2026-11-02' })

  it('hides finished work and matches key or title, the same way for both views', () => {
    expect(filterItems([done, open], { hideDone: false, text: '' })).toHaveLength(2)
    expect(filterItems([done, open], { hideDone: true, text: '' })).toEqual([open])
    expect(filterItems([done, open], { hideDone: false, text: 'title o' })).toEqual([open])
    expect(filterItems([done, open], { hideDone: false, text: 'D' }).map((entry) => entry.ticket.key)).toContain('D')
  })

  it('lists Tasks and Subtasks with neither a due date nor a drawable plan whatever their State, with the reason, and not Epics', () => {
    const none = item('N')
    const epic = { ...item('E'), ticket: { ...item('E').ticket, hierarchyKind: 'EPIC' as const } }
    const finished = item('F', { statusCategory: 'DONE' })
    const canceled = item('C', { statusCategory: 'CANCELED' })
    const doneWithDue = item('G', { statusCategory: 'DONE', dueOn: '2026-11-01' })
    const dueOnly = item('U', { dueOn: '2026-11-21' })
    const planOnly = item('P', { start: '2026-11-02T00:00:00Z', end: '2026-11-03T09:00:00Z' })
    const noEnd = item('X', { start: '2026-11-02T00:00:00Z', end: null })
    const all = [none, epic, finished, canceled, doneWithDue, dueOnly, planOnly, noEnd]
    // With a work calendar an end that cannot be worked out is "beyond"; without one it says so.
    expect(unscheduled(all, true).map(({ item: found, reason }) => [found.ticket.key, reason])).toEqual([['N', 'NO_SCHEDULE'], ['F', 'NO_SCHEDULE'], ['C', 'NO_SCHEDULE'], ['X', 'BEYOND']])
    expect(unscheduled([noEnd], false)[0]?.reason).toBe('NO_CALENDAR')
    // Hiding finished work removes them here exactly as it does from the month, because both use the same filter.
    expect(unscheduled(filterItems(all, { hideDone: true, text: '' }), true).map(({ item: found }) => found.ticket.key)).toEqual(['N', 'X'])
  })
})
