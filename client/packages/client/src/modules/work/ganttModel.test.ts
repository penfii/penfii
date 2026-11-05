import { describe, expect, it } from 'vitest'
import type { CalendarDefinition, GanttItem, TicketSchedule } from '@penfii/contracts'
import { buildAxis, edgePath, ganttRows, nonWorkingSpans, planBar, planConflict, resizeDuration, scheduleBody, shiftStart, unplannedReason, workDayMinutes } from './ganttModel'

const DAY = 540
const item = (id: string, partial: Partial<GanttItem> = {}, schedule: TicketSchedule | null = null): GanttItem => ({
  ticket: { id, key: `PFI-${id}`, title: `Task ${id}`, hierarchyKind: 'TASK' },
  parentId: null,
  channelId: null,
  statusCategory: 'NOT_STARTED',
  assigneeName: null,
  revisionNo: 1,
  schedule,
  ...partial,
})
const planned = (partial: Partial<TicketSchedule> = {}): TicketSchedule => ({
  plannedStartAt: '2026-11-02T00:00:00Z',
  durationMinutes: DAY,
  isScheduleMilestone: false,
  plannedEndAt: '2026-11-02T09:00:00Z',
  ...partial,
})
const week: CalendarDefinition = { version: 1, week: { mon: [{ from: '09:00', to: '18:00' }], tue: [{ from: '09:00', to: '12:00' }, { from: '13:00', to: '15:00' }] }, holidays: [] }

describe('gantt model', () => {
  it('lists children right after their parent, and a child whose parent is not listed as a root', () => {
    const rows = ganttRows([item('a'), item('b', { parentId: 'a' }), item('c'), item('d', { parentId: 'gone' })])
    expect(rows.map((row) => [row.item.ticket.id, row.depth, row.hasChildren])).toEqual([['a', 0, true], ['b', 1, false], ['c', 0, false], ['d', 0, false]])
  })

  it('draws a bar from the start to the derived end, a node as a point, and nothing without an end', () => {
    expect(planBar(item('a', {}, planned()))?.end.toISOString()).toBe('2026-11-02T09:00:00.000Z')
    expect(planBar(item('a', {}, planned({ isScheduleMilestone: true, durationMinutes: null, plannedEndAt: null })))?.milestone).toBe(true)
    expect(planBar(item('a', {}, planned({ plannedEndAt: null })))).toBeNull()
    expect(planBar(item('a'))).toBeNull()
  })

  it('says why open planned work has no bar, and nothing about finished or grouping work', () => {
    expect(unplannedReason(item('a'), true)).toBe('NO_SCHEDULE')
    expect(unplannedReason(item('a', {}, planned({ plannedStartAt: null })), true)).toBe('NO_START')
    expect(unplannedReason(item('a', {}, planned({ durationMinutes: null })), true)).toBe('NO_DURATION')
    expect(unplannedReason(item('a', {}, planned({ plannedEndAt: null })), false)).toBe('NO_CALENDAR')
    expect(unplannedReason(item('a', {}, planned({ plannedEndAt: null })), true)).toBe('BEYOND')
    expect(unplannedReason(item('a', {}, planned()), true)).toBeNull()
    expect(unplannedReason(item('a', { statusCategory: 'DONE' }), true)).toBeNull()
    expect(unplannedReason(item('a', { ticket: { id: 'a', key: 'K', title: 'T', hierarchyKind: 'EPIC' } }), true)).toBeNull()
  })

  it('lays the axis out per day of the calendar zone and maps instants onto it', () => {
    const axis = buildAxis(['2026-11-02T00:00:00Z', '2026-11-04T12:00:00Z'], 'UTC', 10)
    expect(axis).not.toBeNull()
    expect(axis!.x('2026-11-03T00:00:00Z') - axis!.x('2026-11-02T00:00:00Z')).toBeCloseTo(10)
    expect(axis!.days.length).toBeGreaterThanOrEqual(3)
    expect(buildAxis([], 'UTC')).toBeNull()
  })

  it('takes a work day as the longest day of the week, and 8 hours without a calendar', () => {
    expect(workDayMinutes(week)).toBe(540)
    expect(workDayMinutes(undefined)).toBe(480)
  })

  it('moves the start by calendar days on the wall clock of the zone, across daylight saving', () => {
    const start = planned({ plannedStartAt: '2026-11-01T00:00:00Z' }) // 09:00 in Seoul
    expect(shiftStart(start, 1, 'Asia/Seoul')?.plannedStartAt).toBe('2026-11-02T00:00:00Z')
    const us = planned({ plannedStartAt: '2026-10-31T13:00:00Z' }) // 09:00 EDT, the day before the clocks go back
    expect(shiftStart(us, 1, 'America/New_York')?.plannedStartAt).toBe('2026-11-01T14:00:00Z')
    expect(shiftStart(planned({ plannedStartAt: null }), 1, 'UTC')).toBeNull()
  })

  it('resizes by whole work days, never below a minute, and leaves nodes alone', () => {
    expect(resizeDuration(planned(), 1, DAY)?.durationMinutes).toBe(2 * DAY)
    expect(resizeDuration(planned(), -5, DAY)?.durationMinutes).toBe(1)
    expect(resizeDuration(planned({ isScheduleMilestone: true, durationMinutes: null }), 1, DAY)).toBeNull()
  })

  it('does not send the derived end', () => {
    expect(scheduleBody(planned())).not.toHaveProperty('plannedEndAt')
  })

  it('draws a schedule milestone at the working instant the server derived, not at a start after closing time', () => {
    const closing = planned({ isScheduleMilestone: true, durationMinutes: 0, plannedStartAt: '2026-10-29T09:00:00Z', plannedEndAt: '2026-11-02T00:00:00Z' })
    expect(planBar(item('a', {}, closing))?.start.toISOString()).toBe('2026-11-02T00:00:00.000Z')
    // With no derived end (no calendar) it stays where it was put.
    expect(planBar(item('a', {}, { ...closing, plannedEndAt: null }))?.start.toISOString()).toBe('2026-10-29T09:00:00.000Z')
  })

  it('finds a dependency whose successor starts before its predecessor ends, and nothing else', () => {
    const rows = ganttRows([
      item('a', {}, planned()),
      item('b', {}, planned({ plannedStartAt: '2026-11-02T03:00:00Z', plannedEndAt: '2026-11-02T12:00:00Z' })),
      item('c', {}, planned({ plannedStartAt: '2026-11-03T00:00:00Z', plannedEndAt: '2026-11-03T09:00:00Z' })),
      item('d', { statusCategory: 'DONE' }, planned()),
    ])
    const edge = (from: string, to: string, waived = false) => ({ id: `${from}${to}`, predecessorId: from, successorId: to, waived, predecessorCategory: 'NOT_STARTED' as const })
    expect(planConflict(edge('a', 'b'), rows)).toBe(true)
    expect(planConflict(edge('a', 'c'), rows)).toBe(false)
    expect(planConflict(edge('a', 'b', true), rows)).toBe(false)
    expect(planConflict(edge('d', 'b'), rows)).toBe(false)
  })

  it('routes an arrow with room right-down-right, and one without room around the rows', () => {
    expect(edgePath(10, 15, 100, 45, 30)).toBe('M10,15 H55 V45 H100')
    const back = edgePath(100, 15, 40, 45, 30)
    expect(back).toBe('M100,15 H108 V30 H32 V45 H40')
    expect(edgePath(100, 45, 40, 15, 30)).toContain('V30 ')
  })

  it('marks the days of the axis the calendar does not work', () => {
    const axis = buildAxis(['2026-11-05T00:00:00Z', '2026-11-10T00:00:00Z'], 'UTC') as NonNullable<ReturnType<typeof buildAxis>>
    const definition: CalendarDefinition = { version: 1, week: { mon: [{ from: '09:00', to: '18:00' }], tue: [{ from: '09:00', to: '18:00' }], wed: [{ from: '09:00', to: '18:00' }], thu: [{ from: '09:00', to: '18:00' }], fri: [{ from: '09:00', to: '18:00' }] }, holidays: [{ date: '2026-11-09', name: 'Day off' }] }
    const spans = nonWorkingSpans(axis, definition)
    // 2026-11-07 and 11-08 are a weekend; 11-09 is a Monday made a holiday.
    expect(spans.filter((span) => !span.holiday).length).toBeGreaterThanOrEqual(2)
    expect(spans.filter((span) => span.holiday).map((span) => span.name)).toEqual(['Day off'])
    expect(nonWorkingSpans(axis, undefined)).toEqual([])
  })
})
