import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectBoard, ProjectCalendarView } from '@penfii/contracts'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getProjectWorkflow: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(), getTicketRevisions: vi.fn(), getTicketRevision: vi.fn(), createTicket: vi.fn(), updateTicket: vi.fn(),
  getProjectViews: vi.fn(), getProjectPlanning: vi.fn(), getTicketDependencies: vi.fn(), getProjectCalendar: vi.fn(), putProjectCalendar: vi.fn(), setProjectEstimateUnit: vi.fn(), getProjectPermissions: vi.fn(),
  createSprint: vi.fn(), restoreTicket: vi.fn(), getTicketReferences: vi.fn(), listProjectTrash: vi.fn(), listWorkspaces: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { PlanningPage } from './PlanningPage'
import { ProjectSettings } from '../attention/ProjectSettings'
import { Work } from './Work'
import { DetailPanelHost } from './testHost'
import { renderWithProviders } from '../../test/renderWithProviders'

const BOARD_PATH = '/w/w/projects/p/work'

function mountWork(path: string) {
  return renderWithProviders(
    <><Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} /><DetailPanelHost /></>,
    { route: path, path: '/w/:workspaceId/projects/:projectId/work' },
  )
}

function mountSettings() {
  return renderWithProviders(
    <ProjectSettings workspaceId="w" projectId="p" />,
    { route: '/w/w/projects/p/settings', path: '/w/:workspaceId/projects/:projectId/settings' },
  )
}

const weekdays = { version: 1 as const, week: Object.fromEntries(['mon', 'tue', 'wed', 'thu', 'fri'].map((d) => [d, [{ from: '09:00', to: '18:00' }]])), holidays: [] }
const noCalendar: ProjectCalendarView = { calendar: null, suggestion: { timeZone: 'Asia/Seoul', definition: weekdays }, canManage: true }
const seoulCalendar: ProjectCalendarView = {
  calendar: { projectId: 'p', version: 3, timeZone: 'Asia/Seoul', definition: weekdays, updatedAt: '2026-10-20T00:00:00Z' },
  suggestion: null,
  canManage: true,
}

function board(unit: ProjectBoard['estimateUnit'], tickets = [ticket({})], manage = true): ProjectBoard {
  return { ...fixtureBoard(tickets), estimateUnit: unit, canManageWorkflow: manage }
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([])
  api.listProjectTrash.mockResolvedValue([])
  api.listWorkspaces.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: false, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [], milestones: [] })
  api.getTicketDependencies.mockResolvedValue({ ticketId: 't1', blockedBy: [], blocks: [], related: [], canEdit: true, canWaive: false })
  api.getTicketComments.mockResolvedValue([])
  api.getTicketActivity.mockResolvedValue({ items: [], nextCursor: null })
  api.getTicketRevisions.mockResolvedValue({ items: [], nextBefore: null })
  api.getProjectCalendar.mockResolvedValue(noCalendar)
  api.getTicketReferences.mockResolvedValue({ items: [], nextCursor: null })
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  api.getProjectPermissions.mockResolvedValue({ members: [], capabilities: [], teams: [], viaGroups: [], recentChanges: [] })
})
afterEach(cleanup)

/** The Ticket as the server holds it now: what an update returns is what the next read returns. */
let live = ticket({})

function answer(updated: ReturnType<typeof ticket>) {
  live = updated
  return Promise.resolve(updated)
}

/** The edit button waits for the calendar: a time typed before its zone is known would be read in the wrong zone. */
async function editButton(panel: HTMLElement, name: string) {
  const button = (await within(panel).findByRole('button', { name })) as HTMLButtonElement
  await waitFor(() => expect(button.disabled).toBe(false))
  return button
}

async function openTicket(unit: ProjectBoard['estimateUnit'], found = ticket({})) {
  live = found
  api.getProjectBoard.mockResolvedValue(board(unit, [found]))
  api.getTicket.mockImplementation(() => Promise.resolve(live))
  mountWork(`${BOARD_PATH}?panel=ticket:${found.id}`)
  return await screen.findByRole('complementary', { name: 'Ticket detail' })
}

describe('Estimate in the Ticket panel', () => {
  it('saves a decimal as typed, in the Project unit, as one change', async () => {
    api.updateTicket.mockImplementation((_id: string, body: { estimate?: { value: string } | null }) => answer(ticket({ revisionNo: 2, estimate: body.estimate ? { value: body.estimate.value, unit: 'POINTS' } : null })))
    const panel = await openTicket('POINTS')
    const field = (await within(panel).findByLabelText('Estimate')) as HTMLInputElement
    expect(field.value).toBe('')
    fireEvent.change(field, { target: { value: '5.50' } })
    fireEvent.blur(field)
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledTimes(1))
    expect(api.updateTicket).toHaveBeenCalledWith('t1', { estimate: { value: '5.5' }, expectedRevision: 1 }, expect.any(String))
    await waitFor(() => expect((within(panel).getByLabelText('Estimate') as HTMLInputElement).value).toBe('5.5'))
  })

  it('clears an estimate and treats 0 as a value of its own', async () => {
    api.updateTicket.mockImplementation((_id: string, body: { estimate?: { value: string } | null }) => answer(ticket({ revisionNo: 3, estimate: body.estimate ? { value: body.estimate.value, unit: 'POINTS' } : null })))
    const panel = await openTicket('POINTS', ticket({ revisionNo: 2, estimate: { value: '8', unit: 'POINTS' } }))
    const field = (await within(panel).findByLabelText('Estimate')) as HTMLInputElement
    expect(field.value).toBe('8')
    fireEvent.change(field, { target: { value: '0' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledWith('t1', { estimate: { value: '0' }, expectedRevision: 2 }, expect.any(String)))
    fireEvent.click(await within(panel).findByRole('button', { name: 'Clear estimate' }))
    await waitFor(() => expect(api.updateTicket).toHaveBeenLastCalledWith('t1', { estimate: null, expectedRevision: 3 }, expect.any(String)))
  })

  it('does not send what the server would refuse', async () => {
    const panel = await openTicket('POINTS')
    const field = (await within(panel).findByLabelText('Estimate')) as HTMLInputElement
    fireEvent.change(field, { target: { value: '5.123' } })
    fireEvent.blur(field)
    expect((await within(panel).findByText(/at most two decimals/)).textContent).toContain('at most two decimals')
    expect(api.updateTicket).not.toHaveBeenCalled()
  })

  it('says why there is no input: a Project without a unit, and a Ticket that is not a Task', async () => {
    const panel = await openTicket('NONE')
    expect(await within(panel).findByText(/no estimate unit/)).toBeTruthy()
    expect(within(panel).queryByLabelText('Estimate')).toBeNull()
    cleanup()
    const epic = await openTicket('POINTS', ticket({ id: 'e1', key: 'PFI-009', hierarchyKind: 'EPIC' }))
    expect(await within(epic).findByText('Only a Task can be estimated.')).toBeTruthy()
    expect(within(epic).queryByLabelText('Estimate')).toBeNull()
  })

  it('shows a value written in another unit and says the next one uses the Project unit', async () => {
    const panel = await openTicket('HOURS', ticket({ estimate: { value: '5', unit: 'POINTS' } }))
    // Shown as it was written, never in the Project's unit, with the reason and what to do.
    expect(await within(panel).findByText('5 pt')).toBeTruthy()
    expect(within(panel).queryByText('5 h')).toBeNull()
    expect(within(panel).getByText(/not added to report totals/)).toBeTruthy()
    expect(within(panel).queryByLabelText('Estimate')).toBeNull()
    fireEvent.click(within(panel).getByRole('button', { name: 'Enter again in Hours' }))
    expect((within(panel).getByLabelText('Estimate') as HTMLInputElement).value).toBe('')
  })

  it('reports a refused value instead of a general failure', async () => {
    const refused = Object.assign(new Error('unit'), { code: 'estimate_unit_mismatch', status: 422 })
    api.updateTicket.mockRejectedValue(refused)
    const panel = await openTicket('POINTS')
    const field = (await within(panel).findByLabelText('Estimate')) as HTMLInputElement
    fireEvent.change(field, { target: { value: '3' } })
    fireEvent.blur(field)
    expect((await within(panel).findByText(/cannot be saved/)).textContent).toContain('Project unit')
  })

  it('is part of Create, only for a Task in a Project that estimates', async () => {
    api.getProjectBoard.mockResolvedValue(board('HOURS', []))
    api.createTicket.mockResolvedValue(ticket({ id: 'new' }))
    mountWork(BOARD_PATH)
    fireEvent.click(await screen.findByText('Add ticket'))
    const create = await screen.findByRole('complementary', { name: 'Create ticket' })
    fireEvent.change(within(create).getByLabelText('Title *'), { target: { value: 'Estimated at once' } })
    fireEvent.click(within(create).getByText('More properties'))
    const field = within(create).getByLabelText('Estimate') as HTMLInputElement
    fireEvent.change(field, { target: { value: '2,5' } })
    expect((within(create).getByRole('button', { name: 'Create' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(field, { target: { value: '2.50' } })
    fireEvent.click(within(create).getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createTicket).toHaveBeenCalledTimes(1))
    expect(api.createTicket.mock.calls[0][1]).toMatchObject({ estimate: { value: '2.5' }, hierarchyKind: 'TASK' })
  })

  it('has no field in Create for a Project without a unit', async () => {
    api.getProjectBoard.mockResolvedValue(board('NONE', []))
    mountWork(BOARD_PATH)
    fireEvent.click(await screen.findByText('Add ticket'))
    const create = await screen.findByRole('complementary', { name: 'Create ticket' })
    fireEvent.click(within(create).getByText('More properties'))
    expect(within(create).queryByLabelText('Estimate')).toBeNull()
  })

  it('appears on the card once there is one', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', [ticket({ estimate: { value: '3', unit: 'POINTS' } })]))
    mountWork(BOARD_PATH)
    expect(await screen.findByText('3 pt')).toBeTruthy()
  })
})

describe('Schedule in the Ticket panel', () => {
  it('is typed on the wall clock of the calendar zone and sent as an instant', async () => {
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.updateTicket.mockImplementation((_id: string, body: { schedule?: Record<string, unknown> | null }) => answer(ticket({ revisionNo: 2, schedule: body.schedule ? { plannedStartAt: null, durationMinutes: null, isScheduleMilestone: false, plannedEndAt: '2026-11-02T09:00:00Z', ...body.schedule } : null })))
    const panel = await openTicket('POINTS')
    fireEvent.click(await editButton(panel, 'Add a work schedule'))
    fireEvent.change(within(panel).getByLabelText('Planned start'), { target: { value: '2026-11-02T09:00' } })
    fireEvent.change(within(panel).getByLabelText('Work time'), { target: { value: '8' } })
    expect(within(panel).queryByLabelText('Remaining work')).toBeNull()
    expect(within(panel).getByText(/In Asia\/Seoul/)).toBeTruthy()
    fireEvent.click(within(panel).getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledTimes(1))
    expect(api.updateTicket).toHaveBeenCalledWith('t1', {
      schedule: { plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 480, isScheduleMilestone: false },
      expectedRevision: 1,
    }, expect.any(String))
    // The derived end the server returns is shown, not typed.
    expect(await within(panel).findByText('Planned end')).toBeTruthy()
    expect(within(panel).getByText(/Nov 2, 2026, 6:00 PM/)).toBeTruthy()
  })

  it('opens a schedule milestone for editing with the work time empty and in hours, not as "0 minutes"', async () => {
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    const panel = await openTicket('POINTS', ticket({ schedule: { plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 0, isScheduleMilestone: true, plannedEndAt: '2026-11-02T00:00:00Z' } }))
    fireEvent.click(await editButton(panel, 'Edit work schedule'))
    fireEvent.click(within(panel).getByLabelText('Schedule milestone (work time 0)'))
    expect((within(panel).getByLabelText('Work time') as HTMLInputElement).value).toBe('')
    expect((within(panel).getByLabelText('Unit of Work time') as HTMLSelectElement).value).toBe('h')
  })

  it('refuses a zero duration unless it is a schedule milestone, and sends a milestone as duration 0', async () => {
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.updateTicket.mockResolvedValue(ticket({ revisionNo: 2 }))
    const panel = await openTicket('POINTS')
    fireEvent.click(await editButton(panel, 'Add a work schedule'))
    fireEvent.change(within(panel).getByLabelText('Work time'), { target: { value: '0' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'Save' }))
    expect((await within(panel).findByText(/Only a schedule milestone may be 0/)).textContent).toContain('whole number above 0')
    expect(api.updateTicket).not.toHaveBeenCalled()
    fireEvent.click(within(panel).getByLabelText('Schedule milestone (work time 0)'))
    fireEvent.change(within(panel).getByLabelText('Planned start'), { target: { value: '2026-11-02T09:00' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledTimes(1))
    expect(api.updateTicket.mock.calls[0][1]).toMatchObject({ schedule: { plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 0, isScheduleMilestone: true } })
  })

  it('clears the schedule and saves nothing when the form is empty', async () => {
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.updateTicket.mockResolvedValue(ticket({ revisionNo: 3 }))
    const panel = await openTicket('POINTS', ticket({ revisionNo: 2, schedule: { plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 480, isScheduleMilestone: false, plannedEndAt: '2026-11-02T09:00:00Z' } }))
    expect(await within(panel).findByText('Work time')).toBeTruthy()
    expect(within(panel).getByText('8h')).toBeTruthy()
    fireEvent.click(await editButton(panel, 'Edit work schedule'))
    expect((within(panel).getByLabelText('Planned start') as HTMLInputElement).value).toBe('2026-11-02T09:00')
    fireEvent.click(within(panel).getByRole('button', { name: 'Clear work schedule' }))
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalledWith('t1', { schedule: null, expectedRevision: 2 }, expect.any(String)))
  })

  it('says the end cannot be calculated without a calendar, and where to make one', async () => {
    const panel = await openTicket('POINTS', ticket({ schedule: { plannedStartAt: '2026-11-02T00:00:00Z', durationMinutes: 480, isScheduleMilestone: false, plannedEndAt: null } }))
    expect(await within(panel).findByText('Cannot be calculated: the Project has no work calendar.')).toBeTruthy()
    expect((await within(panel).findByRole('link', { name: 'Create a work calendar' })).getAttribute('href')).toBe('/w/w/projects/p/settings')
  })

  it('cannot be edited without edit access', async () => {
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.getProjectBoard.mockResolvedValue({ ...board('POINTS'), canEdit: false })
    api.getTicket.mockResolvedValue(ticket({}))
    mountWork(`${BOARD_PATH}?panel=ticket:t1`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    expect(await within(panel).findByText('No work schedule.')).toBeTruthy()
    expect(within(panel).queryByRole('button', { name: 'Add a work schedule' })).toBeNull()
  })
})

describe('History and conflicts show the planning values', () => {
  it('reads an estimate and a schedule change like any other field', async () => {
    const found = ticket({ revisionNo: 3 })
    api.getTicketRevisions.mockResolvedValue({
      items: [{ revisionNo: 3, actorName: 'John Park', actorUserId: 'u1', recordedAt: '2026-10-20T00:00:00Z', restoreSourceRevisionNo: null, changedFields: ['estimate', 'schedule'] }],
      nextBefore: null,
    })
    api.getTicketRevision.mockResolvedValue({
      revisionNo: 3, currentRevisionNo: 3, actorUserId: 'u1', actorName: 'John Park', recordedAt: '2026-10-20T00:00:00Z', restoreSourceRevisionNo: null,
      changesFromPrevious: [
        { field: 'estimate', from: null, to: ['5', 'POINTS'] },
        { field: 'schedule', from: null, to: ['2026-11-02T00:00:00Z', '480', '', ''] },
      ],
      changesToCurrent: [],
      restorable: [],
    })
    const panel = await openTicket('POINTS', found)
    fireEvent.click(await within(panel).findByRole('tab', { name: 'History' }))
    fireEvent.click(await within(panel).findByText('View values'))
    expect(await within(panel).findByText('5 pt')).toBeTruthy()
    // The start is shown on the reader's clock, so only its shape is fixed here.
    expect(within(panel).getByText(/^Planned start .+ · Work time 8h$/)).toBeTruthy()
  })
})

describe('Project settings: estimate unit and work calendar', () => {
  it('changes the unit and tells why a change is refused while a Sprint is active', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', []))
    api.setProjectEstimateUnit.mockResolvedValueOnce({ id: 'p', estimateUnit: 'HOURS' }).mockRejectedValueOnce(Object.assign(new Error('locked'), { code: 'estimate_unit_locked', status: 409 }))
    mountSettings()
    const select = (await screen.findByRole('combobox', { name: 'Estimate unit' })) as HTMLSelectElement
    expect(select.value).toBe('POINTS')
    fireEvent.change(select, { target: { value: 'HOURS' } })
    await waitFor(() => expect(api.setProjectEstimateUnit).toHaveBeenCalledWith('p', 'HOURS'))
    expect(await screen.findByText('The unit was saved.')).toBeTruthy()
    fireEvent.change(select, { target: { value: 'NONE' } })
    expect((await screen.findByRole('alert')).textContent).toContain('A Sprint is active')
  })

  it('shows a proposal when there is no calendar and stores nothing until it is created', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', []))
    api.putProjectCalendar.mockResolvedValue(seoulCalendar.calendar)
    mountSettings()
    expect(await screen.findByText(/no work calendar yet/)).toBeTruthy()
    expect((screen.getByLabelText('Time zone') as HTMLInputElement).value).toBe('Asia/Seoul')
    expect((screen.getByLabelText('Mon From') as HTMLInputElement).value).toBe('09:00')
    expect(api.putProjectCalendar).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Create calendar' }))
    await waitFor(() => expect(api.putProjectCalendar).toHaveBeenCalledTimes(1))
    expect(api.putProjectCalendar).toHaveBeenCalledWith('p', { expectedVersion: 0, timeZone: 'Asia/Seoul', definition: weekdays })
  })

  it('edits hours and holidays of an existing calendar against the version that was read', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', []))
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.putProjectCalendar.mockResolvedValue({ ...seoulCalendar.calendar, version: 4 })
    mountSettings()
    const to = (await screen.findByLabelText('Fri To')) as HTMLInputElement
    fireEvent.change(to, { target: { value: '12:00' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add a Sat interval' }))
    fireEvent.change(screen.getByLabelText('Holiday date'), { target: { value: '2026-12-25' } })
    fireEvent.change(screen.getByLabelText('Holiday name'), { target: { value: 'Christmas' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add a holiday' }))
    expect(screen.getByText('Christmas', { exact: false })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Save calendar' }))
    await waitFor(() => expect(api.putProjectCalendar).toHaveBeenCalledTimes(1))
    const [, body] = api.putProjectCalendar.mock.calls[0]
    expect(body.expectedVersion).toBe(3)
    expect(body.definition.week.fri).toEqual([{ from: '09:00', to: '12:00' }])
    expect(body.definition.week.sat).toEqual([{ from: '09:00', to: '18:00' }])
    expect(body.definition.week.sun).toBeUndefined()
    expect(body.definition.holidays).toEqual([{ date: '2026-12-25', name: 'Christmas' }])
  })

  it('keeps what was typed and says so when someone saved first', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', []))
    api.getProjectCalendar.mockResolvedValue(seoulCalendar)
    api.putProjectCalendar.mockRejectedValue(Object.assign(new Error('conflict'), { code: 'calendar_version_conflict', status: 409 }))
    mountSettings()
    fireEvent.change(await screen.findByLabelText('Time zone'), { target: { value: 'Asia/Tokyo' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save calendar' }))
    expect((await screen.findByRole('alert')).textContent).toContain('Someone changed the calendar first')
  })

  it('is read only for someone who cannot manage the Project, and the page is for admins', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', [], false))
    mountSettings()
    expect(await screen.findByText(/Project admin/)).toBeTruthy()
    expect(api.getProjectCalendar).not.toHaveBeenCalled()
  })
})

describe('Sprint planning basis', () => {
  it('sends the time zone and the unit only when they were chosen, and shows them on the Sprint', async () => {
    api.getProjectBoard.mockResolvedValue(board('POINTS', []))
    api.getProjectPlanning.mockResolvedValue({
      canManage: true,
      sprints: [{ id: 'sp1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'PLANNED', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 1, startedAt: null, endedAt: null, timeZone: 'Asia/Seoul', estimateUnit: 'POINTS' }],
      milestones: [],
    })
    api.createSprint.mockResolvedValue({})
    renderWithProviders(
      <PlanningPage workspaceId="w" projectId="p" />,
      { route: '/w/w/projects/p/planning', path: '/w/:workspaceId/projects/:projectId/planning' },
    )
    expect(await screen.findByText('Asia/Seoul · Points')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Create a Sprint' }))
    fireEvent.change(await screen.findByLabelText('Sprint name'), { target: { value: 'Sprint 2' } })
    fireEvent.change(screen.getByLabelText('Starts'), { target: { value: '2026-11-16' } })
    fireEvent.change(screen.getByLabelText('Ends'), { target: { value: '2026-11-27' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createSprint).toHaveBeenCalledWith('p', { name: 'Sprint 2', startsOn: '2026-11-16', endsOn: '2026-11-27' }, expect.any(String)))
    fireEvent.click(await screen.findByRole('button', { name: 'Create a Sprint' }))
    fireEvent.change(await screen.findByLabelText('Sprint name'), { target: { value: 'Sprint 3' } })
    fireEvent.change(screen.getByLabelText('Starts'), { target: { value: '2026-11-30' } })
    fireEvent.change(screen.getByLabelText('Ends'), { target: { value: '2026-12-11' } })
    fireEvent.change(screen.getByLabelText('Time zone'), { target: { value: 'Europe/London' } })
    fireEvent.change(screen.getByLabelText('Estimate unit'), { target: { value: 'HOURS' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createSprint).toHaveBeenLastCalledWith('p', { name: 'Sprint 3', startsOn: '2026-11-30', endsOn: '2026-12-11', timeZone: 'Europe/London', estimateUnit: 'HOURS' }, expect.any(String)))
  })
})
