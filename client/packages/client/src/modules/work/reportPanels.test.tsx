import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectStatusSummary, ReportAmount, Sprint, SprintReport, VelocityReport } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  getSprintReport: vi.fn(), getProjectVelocity: vi.fn(), getProjectStatusSummary: vi.fn(), getProjectPlanning: vi.fn(), getProjectBoard: vi.fn(),
  getSprintResults: vi.fn(), getProjectLabels: vi.fn(), getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { PlanningPage } from './PlanningPage'
import { SprintReportPanel, StatusSummary, VelocityPanel } from './ReportPanels'
import { fixtureBoard, fixtureTicket } from './testFixtures'
import { renderWithProviders } from '../../test/renderWithProviders'

/** Text of the cells of each body row of a table. */
function cells(table: HTMLElement): string[][] {
  return [...table.querySelectorAll('tbody tr')].map((row) => [...row.querySelectorAll('th,td')].map((cell) => cell.textContent ?? ''))
}

/** The `dd` and notes of a figure, found by its `dt` label. */
function figureOf(panel: HTMLElement, label: string): HTMLElement {
  return within(panel).getByText(label, { selector: 'dt' }).parentElement as HTMLElement
}

const amount = (value: string, tasks: number, unestimated = 0, otherUnit = 0): ReportAmount => ({ value, tasks, unestimated, otherUnit })

const sprint = (partial: Partial<Sprint> = {}): Sprint => ({
  id: 'sp1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'COMPLETED', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 4,
  startedAt: '2026-11-02T00:00:00Z', endedAt: '2026-11-13T09:00:00Z', timeZone: 'Asia/Seoul', estimateUnit: 'POINTS', ...partial,
})

const ref = (id: string, key: string, title: string) => ({ id, key, title, hierarchyKind: 'TASK' as const })

/** Example: plan 8, completed 7, 3 of 8 = 37.5 %. */
const scn019 = (partial: Partial<SprintReport> = {}): SprintReport => ({
  sprint: sprint(),
  availability: { available: true, reason: null },
  provisional: false,
  generatedAt: '2026-11-13T09:00:00Z',
  scope: 'VISIBLE_TICKETS',
  startPlan: amount('8', 2),
  completed: amount('7', 2),
  remaining: amount('0', 0),
  adherence: { completedFromPlan: amount('3', 1), percent: '37.5', notApplicable: false },
  burndown: [
    { at: '2026-11-02T00:00:00Z', day: null, remaining: amount('8', 2) },
    { at: '2026-11-05T00:00:00Z', day: '2026-11-04', remaining: amount('12', 3) },
    { at: '2026-11-13T09:00:00Z', day: '2026-11-12', remaining: amount('0', 0) },
  ],
  changes: [
    { at: '2026-11-03T12:00:00Z', kind: 'ADDED', ticket: ref('c', 'PFI-3', 'Task C'), estimateFrom: null, estimateTo: '2', otherUnit: false },
    { at: '2026-11-04T00:00:00Z', kind: 'ESTIMATE_CHANGED', ticket: ref('a', 'PFI-1', 'Task A'), estimateFrom: '3', estimateTo: '5', otherUnit: false },
    { at: '2026-11-06T00:00:00Z', kind: 'REMOVED', ticket: ref('b', 'PFI-2', 'Task B'), estimateFrom: '5', estimateTo: null, otherUnit: false },
  ],
  tickets: [
    { ticket: ref('a', 'PFI-1', 'Task A'), inBaseline: true, addedAt: null, startCategory: 'NOT_STARTED', startEstimate: '3', endCategory: 'DONE', endEstimate: '5', otherUnit: false, outcome: 'DONE', counted: true, reason: 'COUNTED' },
    { ticket: ref('b', 'PFI-2', 'Task B'), inBaseline: true, addedAt: null, startCategory: 'NOT_STARTED', startEstimate: '5', endCategory: 'NOT_STARTED', endEstimate: '5', otherUnit: false, outcome: 'REMOVED', counted: false, reason: 'REMOVED' },
    { ticket: ref('c', 'PFI-3', 'Task C'), inBaseline: false, addedAt: '2026-11-03T12:00:00Z', startCategory: null, startEstimate: null, endCategory: 'DONE', endEstimate: '2', otherUnit: false, outcome: 'DONE', counted: true, reason: 'COUNTED' },
  ],
  ...partial,
})

function mount(node: React.ReactNode) {
  return renderWithProviders(node, { route: '/w/w/projects/p/planning', path: '/w/:workspaceId/projects/:projectId/planning' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
})
afterEach(cleanup)

describe('Sprint report', () => {
  it('shows the plan, the completed amount, adherence and what was left, with the numbers of the worked example', async () => {
    api.getSprintReport.mockResolvedValue(scn019())
    mount(<SprintReportPanel sprint={sprint()} />)
    const panel = await screen.findByRole('region', { name: 'Sprint report' })
    const figure = (label: string) => figureOf(panel, label)
    expect(within(figure('Start plan')).getByText('8 pt')).toBeTruthy()
    expect(within(figure('Completed')).getByText('7 pt')).toBeTruthy()
    expect(within(figure('Plan adherence')).getByText('37.5%')).toBeTruthy()
    expect(within(figure('Plan adherence')).getByText('3 pt of 8 pt from the plan completed')).toBeTruthy()
    expect(within(figure('Remaining')).getByText('0 pt')).toBeTruthy()
    expect(within(panel).getByText('Based on the Tickets I can see')).toBeTruthy()
    expect(api.getSprintReport).toHaveBeenCalledWith('sp1')
  })

  it('draws the Burndown and gives the same numbers as a table', async () => {
    api.getSprintReport.mockResolvedValue(scn019())
    mount(<SprintReportPanel sprint={sprint()} />)
    const chart = await screen.findByRole('img', { name: /Burndown chart\. 2 days/ })
    expect(chart.querySelectorAll('circle').length).toBe(3)
    const rows = cells(screen.getAllByRole('table')[0])
    expect(rows.map((r) => r[1])).toEqual(['8 pt', '12 pt', '0 pt'])
    expect(rows.map((r) => r[2])).toEqual(['2', '3', '0'])
    expect(rows[1][0]).toBe('Nov 4, 2026')
  })

  it('lists the scope changes and the result of each Ticket with the reason it was or was not counted', async () => {
    api.getSprintReport.mockResolvedValue(scn019())
    mount(<SprintReportPanel sprint={sprint()} />)
    const changes = await screen.findByRole('region', { name: 'Scope changes' })
    const text = changes.textContent ?? ''
    expect(text).toContain('Added')
    expect(text).toContain('Estimate changed')
    expect(text).toContain('3 pt → 5 pt')
    expect(text).toContain('Removed')
    const results = screen.getByRole('region', { name: 'Result by Ticket' })
    expect(within(results).getAllByText('Counted as completed').length).toBe(2)
    expect(within(results).getByText('Taken out of the Sprint')).toBeTruthy()
    expect(within(results).getByText('Added after the start')).toBeTruthy()
  })

  it('says what a total leaves out: unestimated Tasks and estimates in another unit', async () => {
    api.getSprintReport.mockResolvedValue(scn019({ startPlan: amount('3', 5, 1, 3), completed: amount('3', 3, 1, 1) }))
    mount(<SprintReportPanel sprint={sprint()} />)
    expect(await screen.findByText('1 unestimated · 3 in another unit')).toBeTruthy()
    expect(screen.getByText('1 unestimated · 1 in another unit')).toBeTruthy()
  })

  it('counts Tasks when the Sprint has no unit, and says there is nothing to measure for an empty plan', async () => {
    api.getSprintReport.mockResolvedValue(scn019({ sprint: sprint({ estimateUnit: 'NONE' }), startPlan: amount('0', 0), completed: amount('2', 2), remaining: amount('0', 0), adherence: { completedFromPlan: amount('0', 0), percent: null, notApplicable: true } }))
    mount(<SprintReportPanel sprint={sprint({ estimateUnit: 'NONE' })} />)
    const panel = await screen.findByRole('region', { name: 'Sprint report' })
    expect(within(figureOf(panel, 'Completed')).getByText('2 tasks')).toBeTruthy()
    expect(within(figureOf(panel, 'Start plan')).getByText('0 tasks')).toBeTruthy()
    expect(within(figureOf(panel, 'Plan adherence')).getByText('Nothing to measure')).toBeTruthy()
  })

  it('marks a running Sprint as provisional', async () => {
    api.getSprintReport.mockResolvedValue(scn019({ provisional: true, sprint: sprint({ state: 'ACTIVE', endedAt: null }) }))
    mount(<SprintReportPanel sprint={sprint({ state: 'ACTIVE', endedAt: null })} />)
    expect((await screen.findByText(/provisional figures as of/)).textContent).toContain('Based on the Tickets I can see')
  })

  it('gives a Sprint without a usable baseline the reason and nothing else', async () => {
    api.getSprintReport.mockResolvedValue(scn019({ availability: { available: false, reason: 'LEGACY_BASELINE' }, startPlan: null, completed: null, remaining: null, adherence: null, burndown: [], changes: [], tickets: [] }))
    mount(<SprintReportPanel sprint={sprint()} />)
    expect((await screen.findByRole('note')).textContent).toContain('Past values are not rebuilt')
    expect(screen.queryByRole('table')).toBeNull()
    expect(screen.queryByText('Start plan')).toBeNull()
  })

  it('stays silent for a canceled Sprint that never started and reports a failure to load', async () => {
    api.getSprintReport.mockResolvedValue(scn019({ availability: { available: false, reason: 'NO_BASELINE' }, startPlan: null, completed: null, remaining: null, adherence: null, burndown: [], changes: [], tickets: [] }))
    const { container } = mount(<SprintReportPanel sprint={sprint({ state: 'CANCELED', startedAt: null })} />)
    await waitFor(() => expect(api.getSprintReport).toHaveBeenCalled())
    await waitFor(() => expect(container.querySelector('.sprint-report')).toBeNull())
    cleanup()
    api.getSprintReport.mockRejectedValue(new Error('down'))
    mount(<SprintReportPanel sprint={sprint()} />)
    expect((await screen.findByRole('alert')).textContent).toContain('could not be loaded')
  })
})

describe('Velocity', () => {
  const entry = (name: string, value: string, extra: Partial<VelocityReport['sprints'][number]> = {}): VelocityReport['sprints'][number] => ({
    sprint: sprint({ id: name, name }), included: true, reason: null, completed: amount(value, 1), ...extra,
  })

  it('draws the completed amount of the Sprints in one unit, oldest first, and lists the others apart', async () => {
    const report: VelocityReport = {
      unit: 'POINTS', policyVersion: 1, generatedAt: '2026-11-20T00:00:00Z', scope: 'VISIBLE_TICKETS',
      sprints: [entry('Sprint 3', '8'), entry('Sprint 2', '5'), entry('Sprint 1', '0', { included: false, reason: 'DIFFERENT_UNIT', completed: null }), entry('Sprint 0', '0', { included: false, reason: 'LEGACY_BASELINE', completed: null }), entry('Sprint P', '0', { included: false, reason: 'DIFFERENT_POLICY', completed: null })],
    }
    api.getProjectVelocity.mockResolvedValue(report)
    mount(<VelocityPanel projectId="p" />)
    const panel = await screen.findByRole('region', { name: 'Velocity' })
    expect(within(panel).getByRole('img', { name: /Last 2 Sprints|last 2 Sprints/i })).toBeTruthy()
    const rows = cells(within(panel).getByRole('table'))
    expect(rows.map((row) => row[0])).toEqual(['Sprint 2', 'Sprint 3'])
    expect(rows[0][1]).toContain('5 pt')
    expect(rows[1][1]).toContain('8 pt')
    expect(within(panel).getByText('Left out of the comparison')).toBeTruthy()
    expect(within(panel).getByText(/Sprint 1 · Another estimate unit/)).toBeTruthy()
    expect(within(panel).getByText(/Sprint 0 · Started before estimate baselines/)).toBeTruthy()
    expect(within(panel).getByText(/Sprint P · Another calculation policy version/)).toBeTruthy()
  })

  it('says so when nothing is completed or nothing can be compared', async () => {
    api.getProjectVelocity.mockResolvedValue({ unit: null, generatedAt: '2026-11-20T00:00:00Z', scope: 'VISIBLE_TICKETS', sprints: [] })
    mount(<VelocityPanel projectId="p" />)
    expect(await screen.findByText(/^No Sprint has been completed yet\./)).toBeTruthy()
    cleanup()
    api.getProjectVelocity.mockResolvedValue({ unit: null, generatedAt: '2026-11-20T00:00:00Z', scope: 'VISIBLE_TICKETS', sprints: [entry('Old', '0', { included: false, reason: 'LEGACY_BASELINE', completed: null })] })
    mount(<VelocityPanel projectId="p" />)
    expect(await screen.findByText(/none has a baseline to compare/)).toBeTruthy()
  })
})

describe('Project status', () => {
  it('shows the distribution, the blocked and due-soon counts and what they are based on', async () => {
    const summary: ProjectStatusSummary = {
      generatedAt: '2026-11-20T00:00:00Z', scope: 'VISIBLE_TICKETS', today: '2026-11-20', dueWithinDays: 7,
      distribution: { notStarted: 5, inProgress: 1, done: 1, canceled: 1, total: 8 }, blocked: 1, dueSoon: 3,
    }
    api.getProjectStatusSummary.mockResolvedValue(summary)
    mount(<StatusSummary projectId="p" />)
    const panel = await screen.findByRole('region', { name: 'Project status' })
    expect(within(panel).getByText('Tickets: 8')).toBeTruthy()
    expect(within(panel).getByText('Not started 5 · In progress 1 · Done 1 · Canceled 1')).toBeTruthy()
    expect(within(within(panel).getByText('Blocked').parentElement as HTMLElement).getByText('1')).toBeTruthy()
    expect(within(within(panel).getByText('Due soon').parentElement as HTMLElement).getByText('3')).toBeTruthy()
    expect(within(panel).getByText(/Due within 7 days from today \(2026-11-20\), overdue ones included/)).toBeTruthy()
    expect(within(panel).getByText(/Based on the Tickets I can see/)).toBeTruthy()
  })
})

describe('Planning page', () => {
  function planning(sprints: Sprint[]) {
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints, milestones: [] })
    api.getProjectBoard.mockResolvedValue({ ...fixtureBoard([fixtureTicket({})]), estimateUnit: 'POINTS' })
    api.getProjectLabels.mockResolvedValue([])
    api.getProjectMembers.mockResolvedValue([])
    api.getProjectTicketTypes.mockResolvedValue([])
    api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'P', teamIds: [] }], channels: [], dms: [] })
    api.getSprintResults.mockResolvedValue({ sprint: sprints[0], baselineCount: 2, baselineRecordedAt: null, results: [] })
    api.getProjectVelocity.mockResolvedValue({ unit: null, generatedAt: '2026-11-20T00:00:00Z', scope: 'VISIBLE_TICKETS', sprints: [] })
    api.getSprintReport.mockResolvedValue(scn019({ sprint: sprints[0] }))
  }

  it('puts Velocity at the top and the report in a started Sprint, and none in a planned one', async () => {
    planning([sprint({ id: 'sp1', state: 'COMPLETED' })])
    mount(<PlanningPage workspaceId="w" projectId="p" />)
    expect(await screen.findByRole('region', { name: 'Velocity' })).toBeTruthy()
    fireEvent.click(await screen.findByRole('button', { name: 'Show tickets' }))
    expect(await screen.findByRole('region', { name: 'Sprint report' })).toBeTruthy()
    cleanup()
    api.getSprintReport.mockClear()
    planning([sprint({ id: 'sp2', state: 'PLANNED', startedAt: null, endedAt: null })])
    mount(<PlanningPage workspaceId="w" projectId="p" />)
    fireEvent.click(await screen.findByRole('button', { name: 'Show tickets' }))
    await waitFor(() => expect(api.getSprintResults).toHaveBeenCalled())
    expect(api.getSprintReport).not.toHaveBeenCalled()
    expect(screen.queryByRole('region', { name: 'Sprint report' })).toBeNull()
  })
})
