import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { useTicketPanel } from './panelParam'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { GanttItem, ProjectSchedule } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getProjectSchedule: vi.fn(), getProjectCalendar: vi.fn(), getProjectPlanning: vi.fn(), updateTicket: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { TimelinePage } from './Timeline'
import { renderWithProviders } from '../../test/renderWithProviders'

const item = (id: string, schedule: GanttItem['schedule'], partial: Partial<GanttItem> = {}): GanttItem => ({
  ticket: { id, key: `PFI-${id}`, title: `Task ${id}`, hierarchyKind: 'TASK' },
  parentId: null, channelId: null, statusCategory: 'NOT_STARTED', assigneeName: null, revisionNo: 3, schedule, ...partial,
})
const plan = (start: string, end: string, minutes: number) => ({ plannedStartAt: start, durationMinutes: minutes, isScheduleMilestone: false, plannedEndAt: end })

const schedule = (partial: Partial<ProjectSchedule> = {}): ProjectSchedule => ({
  calendar: { version: 1, timeZone: 'Asia/Seoul' },
  canEdit: true,
  items: [
    item('1', plan('2026-11-02T00:00:00Z', '2026-11-02T09:00:00Z', 540)),
    item('2', plan('2026-11-03T00:00:00Z', '2026-11-03T09:00:00Z', 540)),
    item('3', null),
  ],
  edges: [{ id: 'e1', predecessorId: '1', successorId: '2', waived: false, predecessorCategory: 'NOT_STARTED' }],
  nextCursor: null,
  ...partial,
})

function ClosePanel() {
  const { close, selectedId } = useTicketPanel()
  return selectedId ? <button type="button" onClick={close}>close panel</button> : null
}

function mount() {
  return renderWithProviders(<><TimelinePage workspaceId="w" projectId="p" /><ClosePanel /></>, { route: '/w/w/projects/p/timeline', path: '/w/:workspaceId/projects/:projectId/timeline' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getProjectSchedule.mockResolvedValue(schedule())
  api.getProjectCalendar.mockResolvedValue({ calendar: null, suggestion: null, canManage: true })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
})
afterEach(cleanup)

describe('Timeline', () => {
  it('draws a bar for each planned Ticket, an arrow for each dependency, and lists the unplanned work with its reason', async () => {
    mount()
    expect(await screen.findAllByRole('button', { name: /^PFI-1 Task 1, / })).toHaveLength(1)
    expect(screen.getByRole('button', { name: /^PFI-2 Task 2, / })).toBeTruthy()
    expect(document.querySelectorAll('path.edge')).toHaveLength(1)
    const unplanned = screen.getByRole('region', { name: 'Unplanned' })
    expect(within(unplanned).getByText('No work schedule')).toBeTruthy()
    expect(screen.getByText(/In Asia\/Seoul/)).toBeTruthy()
  })

  it('offers to create the calendar when there is none, to someone who can manage it', async () => {
    api.getProjectSchedule.mockResolvedValue(schedule({ calendar: null }))
    mount()
    expect(await screen.findByRole('link', { name: /calendar/i })).toBeTruthy()
  })

  it('moves a start by a day with Alt+Right and sends the revision it was drawn from', async () => {
    api.updateTicket.mockResolvedValue({})
    mount()
    const bar = await screen.findByRole('button', { name: /^PFI-1 Task 1, / })
    fireEvent.keyDown(bar, { key: 'ArrowRight', altKey: true })
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledTimes(1))
    const [id, body] = api.updateTicket.mock.calls[0] as [string, { expectedRevision: number; schedule: Record<string, unknown> }]
    expect(id).toBe('1')
    expect(body.expectedRevision).toBe(3)
    expect(body.schedule.plannedStartAt).toBe('2026-11-03T00:00:00Z')
    expect(body.schedule).not.toHaveProperty('plannedEndAt')
  })

  it('lengthens by a work day with Alt+Shift+Right', async () => {
    api.updateTicket.mockResolvedValue({})
    mount()
    fireEvent.keyDown(await screen.findByRole('button', { name: /^PFI-1 Task 1, / }), { key: 'ArrowRight', altKey: true, shiftKey: true })
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledTimes(1))
    expect((api.updateTicket.mock.calls[0][1] as { schedule: { durationMinutes: number } }).schedule.durationMinutes).toBe(540 + 480)
  })

  it('does not write when the person cannot edit, and says so', async () => {
    api.getProjectSchedule.mockResolvedValue(schedule({ canEdit: false }))
    mount()
    fireEvent.keyDown(await screen.findByRole('button', { name: /^PFI-1 Task 1, / }), { key: 'ArrowRight', altKey: true })
    expect(api.updateTicket).not.toHaveBeenCalled()
    expect(screen.getAllByText(/Read only/).length).toBeGreaterThan(0)
  })

  it('reports a revision conflict and reloads instead of overwriting', async () => {
    api.updateTicket.mockRejectedValue(Object.assign(new Error('conflict'), { code: 'version_conflict' }))
    mount()
    fireEvent.keyDown(await screen.findByRole('button', { name: /^PFI-1 Task 1, / }), { key: 'ArrowRight', altKey: true })
    await screen.findAllByText(/changed|conflict/i)
    await waitFor(() => expect(api.getProjectSchedule.mock.calls.length).toBeGreaterThan(1))
  })

  it('reads every page of the list', async () => {
    api.getProjectSchedule.mockResolvedValueOnce(schedule({ items: [item('1', null)], edges: [], nextCursor: 'c2' })).mockResolvedValueOnce(schedule({ items: [item('2', null)], edges: [], nextCursor: null }))
    mount()
    await screen.findAllByText('Task 2')
    expect(api.getProjectSchedule).toHaveBeenCalledTimes(2)
    expect(api.getProjectSchedule.mock.calls[1][1]).toMatchObject({ cursor: 'c2' })
  })

  it('gives the focus back to the bar the Ticket was opened from when the panel closes', async () => {
    mount()
    const bar = await screen.findByRole('button', { name: /^PFI-1 Task 1, / })
    bar.focus()
    fireEvent.keyDown(bar, { key: 'Enter' })
    fireEvent.click(await screen.findByRole('button', { name: 'close panel' }))
    await waitFor(() => expect(document.activeElement).toBe(bar))
  })

  it('shows a plan that contradicts a dependency as a conflict, explains it, and moves nothing', async () => {
    api.getProjectSchedule.mockResolvedValue(
      schedule({ items: [item('1', plan('2026-11-03T00:00:00Z', '2026-11-03T09:00:00Z', 540)), item('2', plan('2026-11-02T00:00:00Z', '2026-11-02T09:00:00Z', 540))] }),
    )
    mount()
    expect(await screen.findByText(/1 schedule conflict\(s\)/)).toBeTruthy()
    expect(document.querySelectorAll('path.edge.conflict')).toHaveLength(1)
    expect(within(screen.getByRole('list', { name: 'Legend' })).getByText('Schedule conflict')).toBeTruthy()
    expect(api.updateTicket).not.toHaveBeenCalled()
  })

  it('has no critical path on it: no calculation, switch, table or marks, and a legend of the plan only', async () => {
    mount()
    const legend = await screen.findByRole('list', { name: 'Legend' })
    expect(within(legend).getByText('Today')).toBeTruthy()
    expect(within(legend).queryByText('Critical')).toBeNull()
    expect(within(legend).queryByText('Slack')).toBeNull()
    expect(screen.queryByRole('switch')).toBeNull()
    expect(screen.queryByRole('button', { name: /Calculate/ })).toBeNull()
    expect(screen.queryByText(/critical path/i)).toBeNull()
    expect(document.querySelectorAll('.edge.critical, .bar.critical, .cpm-bar, .cpm-table')).toHaveLength(0)
  })

  it('names who is assigned in the schedule table, and leaves a dash when nobody is', async () => {
    api.getProjectSchedule.mockResolvedValue(schedule({ items: [item('1', plan('2026-11-02T00:00:00Z', '2026-11-02T09:00:00Z', 540), { assigneeName: 'Alice Kim' }), item('2', null)], edges: [] }))
    mount()
    const table = await screen.findByRole('region', { name: 'Schedule table' })
    expect(within(table).getByRole('columnheader', { name: 'Assignee' })).toBeTruthy()
    expect(within(table).getByText('Alice Kim')).toBeTruthy()
    const rows = within(table).getAllByRole('row')
    expect(within(rows[2]).getAllByText('—').length).toBeGreaterThanOrEqual(4)
  })

  it('opens either end of a schedule conflict from the banner', async () => {
    api.getProjectSchedule.mockResolvedValue(
      schedule({ items: [item('1', plan('2026-11-03T00:00:00Z', '2026-11-03T09:00:00Z', 540)), item('2', plan('2026-11-02T00:00:00Z', '2026-11-02T09:00:00Z', 540))] }),
    )
    mount()
    const note = (await screen.findByText(/1 schedule conflict\(s\)/)).closest('[role="note"]') as HTMLElement
    expect(within(note).getByRole('button', { name: 'Open PFI-1' })).toBeTruthy()
    expect(within(note).getByRole('button', { name: 'Open PFI-2' })).toBeTruthy()
  })

  it('brings today back into view from the Timeline toolbar', async () => {
    mount()
    const group = await screen.findByRole('group', { name: 'Gantt chart' })
    const box = group.querySelector('.gantt-scroll') as HTMLElement
    const scrollTo = vi.fn()
    box.scrollTo = scrollTo as unknown as typeof box.scrollTo
    fireEvent.click(screen.getByRole('button', { name: 'Today' }))
    expect(scrollTo).toHaveBeenCalledTimes(1)
    expect((scrollTo.mock.calls[0][0] as { left: number }).left).toBeGreaterThanOrEqual(0)
  })

  it('moves the "today" line on as days pass, without any other change to the page', async () => {
    // Only the clock and its interval are faked; the data still arrives through ordinary timers.
    vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'], now: new Date('2026-11-03T09:00:00Z') })
    try {
      mount()
      await screen.findAllByRole('button', { name: /^PFI-1 Task 1, / })
      const x = () => Number(document.querySelector('line.today')?.getAttribute('x1'))
      const first = x()
      expect(Number.isFinite(first)).toBe(true)
      act(() => { vi.advanceTimersByTime(24 * 3_600_000) })
      expect(x()).toBeGreaterThan(first)
    } finally {
      vi.useRealTimers()
    }
  })

  it('says a Project has no Tickets in its own words, not the chat\'s', async () => {
    api.getProjectSchedule.mockResolvedValue(schedule({ items: [], edges: [] }))
    mount()
    expect(await screen.findByText('This Project has no Tickets yet.')).toBeTruthy()
    expect(screen.queryByText(/No messages/)).toBeNull()
  })

  it('offers a retry when the timeline cannot be loaded', async () => {
    api.getProjectSchedule.mockRejectedValueOnce(new Error('down'))
    mount()
    expect((await screen.findByRole('alert')).textContent).toContain('could not be loaded')
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(await screen.findByRole('button', { name: /^PFI-1 Task 1, / })).toBeTruthy()
  })

  it('tells that only the first part of a very long list is drawn instead of offering a button that is not there', async () => {
    // Each page holds other Tickets, as the server's cursor pages do; the last page still has a next cursor.
    let page = 0
    api.getProjectSchedule.mockImplementation(async () => {
      page += 1
      return schedule({ items: [item(`p${page}`, null)], edges: [], nextCursor: 'more' })
    })
    mount()
    expect(await screen.findByText(/only the first 5 are drawn/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: /Load more/ })).toBeNull()
  })

  it('names the length of a working day where the keys step by one', async () => {
    mount()
    expect(await screen.findByText(/changes the work time by a working day \(/)).toBeTruthy()
  })
})
