import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { GanttItem, ProjectSchedule } from '@penfii/contracts'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'

const api = vi.hoisted(() => ({ getProjectSchedule: vi.fn(), getProjectCalendar: vi.fn(), getProjectPlanning: vi.fn(), updateTicket: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { TimelinePage } from './Timeline'
import { useTicketPanel } from './panelParam'
import { renderWithProviders } from '../../test/renderWithProviders'

const item = (id: string, partial: Partial<GanttItem> & { start?: string; end?: string }, key = `PFI-${id}`): GanttItem => {
  const { start, end, ...rest } = partial
  return {
    ticket: { id, key, title: `Task ${id}`, hierarchyKind: 'TASK' },
    parentId: null, channelId: null, statusCategory: 'NOT_STARTED', assigneeName: null, revisionNo: 3,
    schedule: start ? { plannedStartAt: start, durationMinutes: 540, isScheduleMilestone: false, plannedEndAt: end ?? null } : null,
    dueOn: null,
    ...rest,
  }
}
const schedule = (items: GanttItem[], partial: Partial<ProjectSchedule> = {}): ProjectSchedule => ({
  calendar: { version: 1, timeZone: 'Asia/Seoul' }, canEdit: true, items, edges: [], nextCursor: null, ...partial,
})

function Probe() {
  const location = useLocation()
  const { selectedId } = useTicketPanel()
  return <output data-testid="where">{`${location.pathname}${location.search}|${selectedId ?? ''}`}</output>
}

function mount(search = '') {
  return renderWithProviders(<><TimelinePage workspaceId="w" projectId="p" /><Probe /></>, { route: `/w/w/projects/p/timeline${search}`, path: '/w/:workspaceId/projects/:projectId/timeline' })
}

const items = [
  item('7', { dueOn: '2026-11-21' }, 'FEA-007'),
  item('1', { start: '2026-11-02T00:00:00Z', end: '2026-11-04T09:00:00Z' }),
  item('2', { statusCategory: 'DONE', dueOn: '2026-11-21' }),
  item('3', {}),
]

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  setViewport(DESKTOP)
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getProjectSchedule.mockResolvedValue(schedule(items))
  api.getProjectCalendar.mockResolvedValue({ calendar: null, suggestion: null, canManage: true })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
})
afterEach(cleanup)

describe('Timeline: Gantt and Calendar (FEA-116)', () => {
  it('opens on the Gantt, switches to the Calendar in the address, and changes nothing that is shared', async () => {
    mount()
    expect(await screen.findByRole('group', { name: 'Gantt chart' })).toBeTruthy()
    expect((screen.getByRole('radio', { name: 'Gantt' }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(screen.getByRole('radio', { name: 'Calendar' }))
    expect(await screen.findByRole('table', { name: /2026/ })).toBeTruthy()
    expect(screen.getByTestId('where').textContent).toContain('mode=calendar')
    expect(api.updateTicket).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('radio', { name: 'Gantt' }))
    expect(await screen.findByRole('group', { name: 'Gantt chart' })).toBeTruthy()
    expect(screen.getByTestId('where').textContent).not.toContain('mode=')
  })

  it('is a native table: weekdays are column headers, each day names its full date, and no grid role promises arrow keys', async () => {
    mount('?mode=calendar&month=2026-11')
    const table = await screen.findByRole('table', { name: 'November 2026' })
    expect(within(table).getAllByRole('columnheader').map((header) => header.textContent)).toEqual(['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'])
    expect(within(table).getAllByRole('row')).toHaveLength(1 + 5)
    expect(document.querySelector('[role="grid"], [role="gridcell"]')).toBeNull()
    // The day's own name is its full date, even for an empty day; the bare number is hidden from assistive technology.
    const empty = within(table).getByRole('cell', { name: /November 3, 2026/ })
    expect(empty.querySelectorAll('button')).toHaveLength(0)
    expect(empty.querySelector('[aria-hidden="true"]')?.textContent).toBe('3')
    // Tab reaches the events in reading order with nothing in between.
    expect(within(table).getAllByRole('button').every((button) => button.tabIndex === 0)).toBe(true)
  })

  it('shows a Ticket that has only a due date on its day, which the Gantt never could', async () => {
    mount('?mode=calendar&month=2026-11')
    const cell = await screen.findByRole('cell', { name: /November 21, 2026/ })
    const due = within(cell).getByRole('button', { name: 'Due: FEA-007 Task 7' })
    expect(due.textContent).toContain('⚑')
    // It is not "no date yet": a due date is a date.
    const pending = screen.getByRole('region', { name: 'No date yet' })
    expect(within(pending).queryByText('Task 7')).toBeNull()
    expect(within(pending).getByText('Task 3')).toBeTruthy()
    expect(within(pending).queryByText('Task 1')).toBeNull()
    fireEvent.click(due)
    expect(screen.getByTestId('where').textContent).toContain('|7')
  })

  it('lists undated finished work when completed work is shown, and says when a filter hides the rest', async () => {
    const dateless = [item('d1', {}), item('d2', { statusCategory: 'DONE' }), item('d3', { statusCategory: 'CANCELED' })]
    api.getProjectSchedule.mockResolvedValue(schedule(dateless))
    mount('?mode=calendar&month=2026-11')
    const pending = await screen.findByRole('region', { name: 'No date yet' })
    expect(within(pending).getAllByRole('listitem')).toHaveLength(3)
    expect(within(pending).getAllByText('No work schedule')).toHaveLength(3)
    cleanup()
    // Hide completed: the month and this list lose the finished ones together; nothing is hidden silently.
    mount('?mode=calendar&month=2026-11&done=hide')
    const hidden = await screen.findByRole('region', { name: 'No date yet' })
    expect(within(hidden).getAllByRole('listitem')).toHaveLength(1)
    cleanup()
    api.getProjectSchedule.mockResolvedValue(schedule([item('d2', { statusCategory: 'DONE' })]))
    mount('?mode=calendar&month=2026-11&done=hide')
    expect(await screen.findByText(/1 are hidden by the current filters/)).toBeTruthy()
    cleanup()
    api.getProjectSchedule.mockResolvedValue(schedule([item('x', { dueOn: '2026-11-21' })]))
    mount('?mode=calendar&month=2026-11')
    expect(await screen.findByText('Every Task and Subtask has a date.')).toBeTruthy()
  })

  it('draws plans as separate, named bars only when asked, on the Project zone days with the end day included', async () => {
    mount('?mode=calendar&month=2026-11')
    await screen.findByRole('table', { name: /2026/ })
    expect(screen.queryByRole('button', { name: /^Plan:/ })).toBeNull()
    fireEvent.click(screen.getByRole('checkbox', { name: 'Show planned work' }))
    expect(screen.getByTestId('where').textContent).toContain('plan=1')
    // Nov 2 09:00 to Nov 4 18:00 in Seoul: Mon through Wed.
    const plan = await screen.findByRole('button', { name: 'Plan: PFI-1 Task 1, 2026-11-02 to 2026-11-04' })
    expect(plan.textContent).toContain('Plan')
    expect(within(screen.getByRole('cell', { name: /November 2, 2026/ })).getByRole('button', { name: /Plan: PFI-1/ })).toBe(plan)
    fireEvent.click(plan)
    expect(screen.getByTestId('where').textContent).toContain('|1')
  })

  it('applies the hide-completed filter to the Calendar and to the Gantt alike', async () => {
    mount('?mode=calendar&month=2026-11')
    const cell = await screen.findByRole('cell', { name: /November 21, 2026/ })
    expect(within(cell).getAllByRole('button')).toHaveLength(2)
    fireEvent.click(screen.getByRole('checkbox', { name: 'Hide completed and canceled' }))
    expect(screen.getByTestId('where').textContent).toContain('done=hide')
    await waitFor(() => expect(within(screen.getByRole('cell', { name: /November 21, 2026/ })).getAllByRole('button')).toHaveLength(1))
    // The same condition, one meaning: the switch keeps it, and the Gantt does not list the finished Ticket either.
    fireEvent.click(screen.getByRole('radio', { name: 'Gantt' }))
    await screen.findByRole('group', { name: 'Gantt chart' })
    expect(screen.queryByText('Task 2')).toBeNull()
  })

  it('lists every Ticket of a crowded day behind "more", never cutting any off', async () => {
    const crowded = Array.from({ length: 6 }, (_, index) => item(`c${index}`, { dueOn: '2026-11-10' }, `PFI-${String(index + 10)}`))
    api.getProjectSchedule.mockResolvedValue(schedule(crowded))
    mount('?mode=calendar&month=2026-11')
    const cell = await screen.findByRole('cell', { name: /November 10, 2026/ })
    expect(within(cell).getAllByRole('button', { name: /^Due:/ })).toHaveLength(3)
    fireEvent.click(within(cell).getByRole('button', { name: '+3 more' }))
    const panel = await screen.findByRole('dialog', { name: '+3 more' })
    expect(within(panel).getAllByRole('button')).toHaveLength(6)
    fireEvent.click(within(panel).getByRole('button', { name: 'Due: PFI-15 Task c5' }))
    expect(screen.getByTestId('where').textContent).toContain('|c5')
  })

  it('moves by month and back to today, keeping the shared day in the address', async () => {
    mount('?mode=calendar&month=2026-11')
    await screen.findByRole('table', { name: /2026/ })
    expect(screen.getByRole('heading', { name: 'November 2026' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Next month' }))
    expect(screen.getByRole('heading', { name: 'December 2026' })).toBeTruthy()
    // A month alone means its 15th: that is the day both views share.
    expect(screen.getByTestId('where').textContent).toContain('date=2026-12-15')
    expect(screen.getByText('Nothing is due or planned this month.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Today' }))
    expect(screen.getByTestId('where').textContent).not.toContain('date=')
  })

  it('says which time zone the dates follow, and notes that the browser differs', async () => {
    mount('?mode=calendar&month=2026-11')
    expect(await screen.findByText(/Dates follow the Project time zone \(Asia\/Seoul\)/)).toBeTruthy()
  })

  it('uses a day-by-day list on a phone, with a plan listed once on its first day', async () => {
    setViewport(PHONE)
    mount('?mode=calendar&month=2026-11&plan=1')
    const agenda = await screen.findByRole('list', { name: 'Schedule by day' })
    expect(screen.queryByRole('table', { name: /2026/ })).toBeNull()
    expect(within(agenda).getAllByRole('button', { name: /Plan: PFI-1/ })).toHaveLength(1)
    expect(within(agenda).getByRole('button', { name: 'Due: FEA-007 Task 7' })).toBeTruthy()
  })
})

describe('Timeline: one shared day for Gantt and Calendar (FEA-123)', () => {
  it('reads an old month link as that month\'s 15th and rewrites the address', async () => {
    mount('?mode=calendar&month=2026-11')
    expect(await screen.findByRole('heading', { name: 'November 2026' })).toBeTruthy()
    await waitFor(() => expect(screen.getByTestId('where').textContent).toContain('date=2026-11-15'))
    expect(screen.getByTestId('where').textContent).not.toContain('month=')
    // An explicit date wins over a stale month.
    cleanup()
    mount('?mode=calendar&month=2026-11&date=2027-02-03')
    expect(await screen.findByRole('heading', { name: 'February 2027' })).toBeTruthy()
  })

  it('keeps the month the calendar was on when switching to the Gantt, and puts that day on the Gantt axis even with nothing planned there', async () => {
    mount('?mode=calendar&date=2027-03-15')
    expect(await screen.findByRole('heading', { name: 'March 2027' })).toBeTruthy()
    fireEvent.click(screen.getByRole('radio', { name: 'Gantt' }))
    await screen.findByRole('group', { name: 'Gantt chart' })
    expect(screen.getByTestId('where').textContent).toContain('date=2027-03-15')
    // The days of that month are on the axis (every day is labelled at the day scale).
    fireEvent.click(screen.getByRole('radio', { name: 'Day' }))
    expect(document.querySelector('svg.gantt-svg')?.textContent).toContain('03-15')
  })

  it('makes the day in the middle of the Gantt, once it stops moving, the shared day, and the Calendar opens on its month', async () => {
    mount()
    const chart = await waitFor(() => {
      const found = document.querySelector<HTMLElement>('.gantt-scroll')
      if (!found) throw new Error('no chart yet')
      return found
    })
    // The chart's own first move (to today) has long settled; what follows is the person.
    await new Promise((resolve) => setTimeout(resolve, 1300))
    // A person moves the chart far to the right (jsdom has no layout: the middle is the scroll position itself).
    chart.scrollLeft = 16 * 300
    fireEvent.scroll(chart)
    await waitFor(() => expect(screen.getByTestId('where').textContent).toMatch(/date=\d{4}-\d{2}-\d{2}/), { timeout: 2000 })
    const shared = /date=(\d{4}-\d{2})-\d{2}/.exec(screen.getByTestId('where').textContent ?? '')?.[1] ?? ''
    fireEvent.click(screen.getByRole('radio', { name: 'Calendar' }))
    const heading = await waitFor(() => document.querySelector('h3.calendar-month') as HTMLElement)
    const [year, month] = shared.split('-').map(Number) as [number, number]
    expect(heading.textContent).toBe(new Intl.DateTimeFormat('en-US', { year: 'numeric', month: 'long', timeZone: 'UTC' }).format(new Date(Date.UTC(year, month - 1, 1))))
  })

  it('goes back to the first-visit behaviour with Today, which removes the shared day', async () => {
    mount('?date=2027-03-15')
    await screen.findByRole('group', { name: 'Gantt chart' })
    fireEvent.click(screen.getByRole('button', { name: 'Today' }))
    expect(screen.getByTestId('where').textContent).not.toContain('date=')
  })

  it('is a personal way of looking: nothing is written to the Project, and another Project\'s address does not carry it', async () => {
    mount('?mode=calendar&date=2027-03-15')
    await screen.findByRole('heading', { name: 'March 2027' })
    expect(api.updateTicket).not.toHaveBeenCalled()
    cleanup()
    // Another Project is another address, and without a date it opens on its own first-visit month.
    renderWithProviders(<><TimelinePage workspaceId="w" projectId="q" /><Probe /></>, { route: '/w/w/projects/q/timeline?mode=calendar', path: '/w/:workspaceId/projects/:projectId/timeline' })
    expect(await screen.findByRole('table')).toBeTruthy()
    expect(screen.getByTestId('where').textContent).not.toContain('date=')
  })
})
