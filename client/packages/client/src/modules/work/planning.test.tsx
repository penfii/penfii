import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ProjectView, WorkflowState } from '@penfii/contracts'
import { fixtureBoard, fixtureStates, fixtureTicket as ticket } from './testFixtures'
import { defaultDefinition, emptyFilter } from './viewModel'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getSprintResults: vi.fn(), getSprintClosePreview: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getProjectWorkflow: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(),
  getTicketRevisions: vi.fn(), getTicketRevision: vi.fn(), changeTicketStatus: vi.fn(), updateTicket: vi.fn(),
  moveTicket: vi.fn(), getProjectViews: vi.fn(), createProjectView: vi.fn(), updateProjectView: vi.fn(), deleteProjectView: vi.fn(),
  getProjectPlanning: vi.fn(), getTicketDependencies: vi.fn(), addTicketDependency: vi.fn(), removeDependency: vi.fn(), waiveDependency: vi.fn(), restoreDependency: vi.fn(),
  assignSprint: vi.fn(), setTicketMilestones: vi.fn(), updateWorkflowState: vi.fn(), createSprint: vi.fn(), sprintCommand: vi.fn(),
  createMilestone: vi.fn(), updateMilestone: vi.fn(), createWorkflowState: vi.fn(), deleteWorkflowState: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { PlanningPage } from './PlanningPage'
import { Work } from './Work'
import { DetailPanelHost } from './testHost'
import { renderWithProviders } from '../../test/renderWithProviders'

const BOARD_PATH = '/w/w/projects/p/work'
const backend = { id: 'l-be', name: 'backend', color: null }

function mount(path: string) {
  return renderWithProviders(
    <><Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} /><DetailPanelHost /></>,
    { route: path, path: '/w/:workspaceId/projects/:projectId/work' },
  )
}

function savedView(partial: Partial<ProjectView> = {}): ProjectView {
  return {
    id: 'v1', projectId: 'p', targetKind: 'PROJECT', teamId: null, name: 'Backend', layout: 'BOARD', scope: 'PERSONAL', ownerId: 'u1',
    definition: { ...defaultDefinition(), filter: { ...emptyFilter(), labelIds: ['l-be'] } },
    version: 3, canEdit: true, createdAt: '2026-10-19T00:00:00Z', updatedAt: '2026-10-19T00:00:00Z', ...partial,
  }
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([backend, { id: 'l-fe', name: 'frontend', color: null }])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getTicketComments.mockResolvedValue([])
  api.getTicketActivity.mockResolvedValue({ items: [], nextCursor: null })
  api.getTicketRevisions.mockResolvedValue({ items: [], nextBefore: null })
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: false, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [], blocks: [], related: [], canEdit: true, canWaive: false })
})
afterEach(cleanup)

describe('Views on the Board', () => {
  const tickets = [
    ticket({ id: 'a', key: 'PFI-001', title: 'API login', labels: [backend] }),
    ticket({ id: 'b', key: 'PFI-002', title: 'Login page', labels: [{ id: 'l-fe', name: 'frontend', color: null }] }),
  ]

  it('shows all work by default and a saved View\'s filter when it is selected', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.getProjectViews.mockResolvedValue([savedView()])
    mount(`${BOARD_PATH}?v=v1`)
    expect(await screen.findByText('API login')).toBeTruthy()
    expect(screen.queryByText('Login page')).toBeNull()
    expect(screen.getByText('Showing 1 of 2 with filters')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'All work' }))
    expect(await screen.findByText('Login page')).toBeTruthy()
  })

  it('narrows by the search box as a draft that can be saved over the View, with its version', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.getProjectViews.mockResolvedValue([savedView({ definition: defaultDefinition() })])
    api.updateProjectView.mockImplementation((_id: string, body: { definition: unknown }) => Promise.resolve(savedView({ version: 4, definition: body.definition as ProjectView['definition'] })))
    mount(`${BOARD_PATH}?v=v1`)
    await screen.findByText('API login')
    fireEvent.change(screen.getByLabelText('Search title or key'), { target: { value: 'page' } })
    expect(screen.queryByText('API login')).toBeNull()
    expect(screen.getByText('You have unsaved changes.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Save changes' }))
    await waitFor(() => expect(api.updateProjectView).toHaveBeenCalledTimes(1))
    expect(api.updateProjectView).toHaveBeenCalledWith('v1', { expectedVersion: 3, layout: 'BOARD', definition: expect.objectContaining({ filter: expect.objectContaining({ text: 'page' }) }) })
  })

  it('does not let a read-only member save over a shared View', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.getProjectViews.mockResolvedValue([savedView({ scope: 'PROJECT', canEdit: false, definition: defaultDefinition() })])
    mount(`${BOARD_PATH}?v=v1`)
    await screen.findByText('API login')
    fireEvent.change(screen.getByLabelText('Search title or key'), { target: { value: 'api' } })
    expect(screen.queryByRole('button', { name: 'Save changes' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Save as new view' })).toBeTruthy()
  })

  it('creates a View from the current draft and selects it', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.createProjectView.mockResolvedValue(savedView({ id: 'v2', name: 'Login work' }))
    mount(`${BOARD_PATH}?d=1&q=login`)
    await screen.findByText('API login')
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Login work' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createProjectView).toHaveBeenCalledTimes(1))
    expect(api.createProjectView).toHaveBeenCalledWith('p', { name: 'Login work', scope: 'PERSONAL', layout: 'BOARD', definition: expect.objectContaining({ filter: expect.objectContaining({ text: 'login' }) }) }, expect.any(String))
  })

  it('explains a taken name instead of failing silently', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.createProjectView.mockRejectedValue(Object.assign(new Error('taken'), { code: 'view_name_taken' }))
    mount(`${BOARD_PATH}?d=1&q=login`)
    await screen.findByText('API login')
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Backend' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    expect(await screen.findByText('A view with this name already exists.')).toBeTruthy()
  })

  it('groups by assignee and says cards cannot be dragged there', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'API login', assigneeId: 'u1', assigneeName: 'John Park' }), ticket({ id: 'b', title: 'Login page' })]))
    mount(`${BOARD_PATH}?d=1&group=ASSIGNEE`)
    await screen.findByText('API login')
    const columns = screen.getAllByRole('region')
    expect(columns.map((column) => column.getAttribute('aria-label'))).toEqual(['John Park', 'Unassigned'])
    expect(screen.getByText(/Cards cannot be dragged in this grouping/)).toBeTruthy()
    expect(document.querySelectorAll('.ticket-card[draggable="true"]').length).toBe(0)
  })
})

describe('ordering, blocking and WIP on the Board', () => {
  it('moves a child with Alt+ArrowDown next to its sibling, in the shared order', async () => {
    const parent = { id: 'p1', key: 'PFI-010', title: 'Parent', hierarchyKind: 'TASK' as const }
    api.getProjectBoard.mockResolvedValue(fixtureBoard([
      ticket({ id: 'p1', key: 'PFI-010', title: 'Parent', childSummary: { total: 2, done: 0, canceled: 0 } }),
      ticket({ id: 'a', key: 'PFI-001', title: 'First', hierarchyKind: 'SUBTASK', parentId: 'p1', parent }),
      ticket({ id: 'b', key: 'PFI-002', title: 'Second', hierarchyKind: 'SUBTASK', parentId: 'p1', parent }),
    ]))
    api.moveTicket.mockResolvedValue(ticket({ id: 'a' }))
    mount(BOARD_PATH)
    const open = (await screen.findByText('First')).closest('article')?.querySelector('button')
    if (!open) throw new Error('card button missing')
    fireEvent.keyDown(open, { key: 'ArrowDown', altKey: true })
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledTimes(1))
    expect(api.moveTicket).toHaveBeenCalledWith('a', { afterId: 'b' }, expect.any(String))
  })

  it('does not change the shared order when a parent or a lone card is nudged', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-001', title: 'First' }), ticket({ id: 'b', key: 'PFI-002', title: 'Second' })]))
    mount(BOARD_PATH)
    const open = (await screen.findByText('First')).closest('article')?.querySelector('button')
    if (!open) throw new Error('card button missing')
    fireEvent.keyDown(open, { key: 'ArrowDown', altKey: true })
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(api.moveTicket).not.toHaveBeenCalled()
  })

  it('keeps the card where it was and explains when a predecessor blocks completing it', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-001', title: 'Waiting', dependencies: { openBlockers: 1, blockedBy: 1, blocks: 0 } })]))
    api.moveTicket.mockRejectedValue(Object.assign(new Error('blocked'), { code: 'dependency_blocked' }))
    mount(BOARD_PATH)
    const open = (await screen.findByText('Waiting')).closest('article')?.querySelector('button')
    if (!open) throw new Error('card button missing')
    expect(screen.getByText('Open predecessors 1')).toBeTruthy()
    fireEvent.keyDown(open, { key: 'ArrowRight', altKey: true })
    expect((await screen.findByRole('alert')).textContent).toContain('block completing')
    expect(api.moveTicket).toHaveBeenCalledWith('a', { expectedRevision: 1, statusId: 's-doing' }, expect.any(String))
  })

  it('calls a finished card whose predecessor reopened a recheck, and an in-progress one still waiting', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'Finished', statusId: 's-done', dependencies: { openBlockers: 2, blockedBy: 2, blocks: 0 } }), ticket({ id: 'b', title: 'Underway', statusId: 's-doing', dependencies: { openBlockers: 1, blockedBy: 1, blocks: 0 } })]))
    mount(BOARD_PATH)
    await screen.findByText('Underway')
    expect(screen.getByText('Recheck 2')).toBeTruthy()
    expect(screen.getByText('Open predecessors 1')).toBeTruthy()
  })

  it('warns over the WIP limit using the whole column even when a filter hides cards', async () => {
    const limited: WorkflowState[] = fixtureStates.map((state) => (state.id === 's-todo' ? { ...state, wipLimit: 2 } : state))
    api.getProjectBoard.mockResolvedValue(fixtureBoard([
      ticket({ id: 'a', title: 'One', labels: [backend] }), ticket({ id: 'b', title: 'Two' }), ticket({ id: 'c', title: 'Three' }),
    ], true, limited))
    mount(`${BOARD_PATH}?d=1&label=l-be`)
    await screen.findByText('One')
    expect(screen.queryByText('Two')).toBeNull()
    const todo = screen.getByRole('region', { name: 'Todo' })
    expect(within(todo).getByText(/Over WIP · WIP 3\/2/)).toBeTruthy()
  })

  it('shows no WIP badge for a column without a limit', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    mount(BOARD_PATH)
    await screen.findByText('One')
    expect(screen.queryByText(/WIP/)).toBeNull()
  })

  it('lets an admin set the limit and sends the workflow version', async () => {
    const board = { ...fixtureBoard([ticket({ id: 'a', title: 'One' })]), canManageWorkflow: true, workflowVersion: 7 }
    api.getProjectBoard.mockResolvedValue(board)
    api.updateWorkflowState.mockResolvedValue({ version: 8, state: fixtureStates[0] })
    mount(BOARD_PATH)
    await screen.findByText('One')
    fireEvent.click(within(screen.getByRole('region', { name: 'Todo' })).getByRole('button', { name: 'Settings for column Todo' }))
    fireEvent.change(screen.getByLabelText('WIP limit'), { target: { value: '3' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.updateWorkflowState).toHaveBeenCalledWith('p', 's-todo', { expectedWorkflowVersion: 7, wipLimit: 3 }))
  })
})

describe('dependencies in the ticket panel', () => {
  const waiting = ticket({ id: 'a', key: 'PFI-001', title: 'Waiting' })
  const other = ticket({ id: 'b', key: 'PFI-002', title: 'Other' })
  const edge = { id: 'e1', ticket: { id: 'b', key: 'PFI-002', title: 'Other', hierarchyKind: 'TASK' as const }, stateName: 'Todo', stateCategory: 'NOT_STARTED' as const, satisfied: false, createdAt: '2026-10-19T00:00:00Z', kind: 'BLOCKS' as const, waived: false, waiveReason: null, predecessorCanceled: false, needsReview: false }

  beforeEach(() => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([waiting, other]))
    api.getTicket.mockResolvedValue(waiting)
  })

  it('lists what must finish first and removes one', async () => {
    api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [edge], blocks: [], related: [], canEdit: true, canWaive: false })
    api.removeDependency.mockResolvedValue(undefined)
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    expect(await within(panel).findByText('Must finish first')).toBeTruthy()
    expect(within(panel).getByText(/Waiting · Todo/)).toBeTruthy()
    fireEvent.click(within(panel).getByRole('button', { name: 'Remove predecessor PFI-002' }))
    await waitFor(() => expect(api.removeDependency).toHaveBeenCalledWith('e1'))
  })

  it('explains a canceled predecessor and lets an admin waive it with a reason, then restore it', async () => {
    const canceled = { ...edge, stateName: 'Canceled', stateCategory: 'CANCELED' as const, predecessorCanceled: true }
    api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [canceled], blocks: [], related: [], canEdit: true, canWaive: true })
    api.waiveDependency.mockResolvedValue({})
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    expect(await within(panel).findByText(/PFI-002 was canceled/)).toBeTruthy()
    expect(within(panel).getByText(/Predecessor canceled · Canceled/)).toBeTruthy()
    fireEvent.click(within(panel).getByRole('button', { name: 'Waive' }))
    const submit = within(panel).getAllByRole('button', { name: 'Waive' }).find((button) => button.getAttribute('type') === 'submit') as HTMLElement
    expect((submit as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(within(panel).getByLabelText(/Why waive predecessor/), { target: { value: 'Replaced by PFI-009' } })
    fireEvent.click(submit)
    await waitFor(() => expect(api.waiveDependency).toHaveBeenCalledWith('e1', 'Replaced by PFI-009', expect.any(String)))
  })

  it('does not offer a waiver to someone who cannot waive, but still lets the successor editor remove a canceled relation', async () => {
    const canceled = { ...edge, stateName: 'Canceled', stateCategory: 'CANCELED' as const, predecessorCanceled: true }
    api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [canceled], blocks: [], related: [], canEdit: true, canWaive: false })
    api.removeDependency.mockResolvedValue(undefined)
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    await within(panel).findByText(/Predecessor canceled/)
    expect(within(panel).queryByRole('button', { name: 'Waive' })).toBeNull()
    fireEvent.click(within(panel).getByRole('button', { name: 'Remove predecessor PFI-002' }))
    await waitFor(() => expect(api.removeDependency).toHaveBeenCalledWith('e1'))
  })

  it('links a related ticket without any block', async () => {
    api.addTicketDependency.mockResolvedValue({})
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    fireEvent.change(await within(panel).findByLabelText('Choose a related ticket'), { target: { value: 'b' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'Add related' }))
    await waitFor(() => expect(api.addTicketDependency).toHaveBeenCalledWith('a', 'b', expect.any(String), 'RELATED'))
  })

  it('offers only tickets that can be linked and adds the chosen predecessor', async () => {
    api.addTicketDependency.mockResolvedValue({ ticketId: 'a', blockedBy: [edge], blocks: [], related: [], canEdit: true, canWaive: false })
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    const pick = (await within(panel).findByLabelText('Choose a predecessor')) as HTMLSelectElement
    expect(Array.from(pick.options).map((option) => option.textContent)).toEqual(['Choose a predecessor', 'PFI-002 · Other'])
    fireEvent.change(pick, { target: { value: 'b' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'Add' }))
    await waitFor(() => expect(api.addTicketDependency).toHaveBeenCalledWith('a', 'b', expect.any(String)))
  })

  it('says why a loop is refused', async () => {
    api.addTicketDependency.mockRejectedValue(Object.assign(new Error('cycle'), { code: 'dependency_cycle' }))
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    fireEvent.change(await within(panel).findByLabelText('Choose a predecessor'), { target: { value: 'b' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'Add' }))
    expect(await within(panel).findByText('That would make tickets wait for each other.')).toBeTruthy()
  })

  it('shows a Sprint choice only a Task can use', async () => {
    api.getProjectPlanning.mockResolvedValue({
      canManage: false,
      sprints: [{ id: 's1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'ACTIVE', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 2, startedAt: null, endedAt: null }],
      milestones: [],
    })
    api.assignSprint.mockResolvedValue({ ...waiting, sprint: { id: 's1', name: 'Sprint 1', state: 'ACTIVE', inherited: false } })
    mount(`${BOARD_PATH}?panel=ticket:a`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    const select = (await within(panel).findByLabelText('Sprint')) as HTMLSelectElement
    await waitFor(() => expect(Array.from(select.options).map((option) => option.textContent)).toContain('Sprint 1'))
    fireEvent.change(select, { target: { value: 's1' } })
    await waitFor(() => expect(api.assignSprint).toHaveBeenCalledWith('a', 's1', expect.any(String)))
  })
})

describe('parent groups on the Board', () => {
  const parent = { id: 'a', key: 'PFI-010', title: 'Task A', hierarchyKind: 'TASK' as const }
  const sub = (id: string, key: string, over = {}) => ticket({ id, key, title: id.toUpperCase(), hierarchyKind: 'SUBTASK', parentId: 'a', parent, ...over })

  it('shows a parent from another State as a header that is neither a card nor counted', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([
      ticket({ id: 'a', key: 'PFI-010', title: 'Task A', statusId: 's-done' }),
      sub('a1', 'PFI-011'),
      sub('a2', 'PFI-012', { statusId: 's-doing' }),
    ]))
    mount(BOARD_PATH)
    await screen.findByText('A1')
    const doing = screen.getByRole('region', { name: 'In Progress' })
    const header = doing.querySelector('[data-context-id="a"]')
    expect(header?.textContent).toContain('PFI-010')
    expect(header?.textContent).toContain('Task A')
    expect(header?.textContent).toContain('Done')
    expect(doing.querySelectorAll('[data-ticket-id]')).toHaveLength(1)
    expect(doing.querySelector('[data-ticket-id="a"]')).toBeNull()
    expect(within(doing).getByLabelText('1')).toBeTruthy()
    const done = screen.getByRole('region', { name: 'Done' })
    expect(done.querySelector('[data-context-id]')).toBeNull()
    expect(done.querySelector('[data-ticket-id="a"]')).toBeTruthy()
    // The header opens the real parent.
    api.getTicket.mockResolvedValue(ticket({ id: 'a', key: 'PFI-010', title: 'Task A', statusId: 's-done' }))
    fireEvent.click(header as Element)
    await waitFor(() => expect(api.getTicket).toHaveBeenCalledWith('a'))
  })

  it('does not count a context header toward WIP', async () => {
    const limited: WorkflowState[] = fixtureStates.map((state) => (state.id === 's-doing' ? { ...state, wipLimit: 1 } : state))
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-010', title: 'Task A', statusId: 's-done' }), sub('a2', 'PFI-012', { statusId: 's-doing' })], true, limited))
    mount(BOARD_PATH)
    await screen.findByText('A2')
    const doing = screen.getByRole('region', { name: 'In Progress' })
    expect(within(doing).getByText('WIP 1/1')).toBeTruthy()
    expect(within(doing).queryByText(/Over WIP/)).toBeNull()
  })

  it('moves a child to another State without changing the shared order of its parent group', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-010', title: 'Task A' }), sub('a1', 'PFI-011')]))
    api.moveTicket.mockResolvedValue(ticket({ id: 'a1' }))
    mount(BOARD_PATH)
    const open = (await screen.findByText('A1')).closest('article')?.querySelector('button')
    if (!open) throw new Error('card button missing')
    fireEvent.keyDown(open, { key: 'ArrowRight', altKey: true })
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledTimes(1))
    expect(api.moveTicket).toHaveBeenCalledWith('a1', { expectedRevision: 1, statusId: 's-doing' }, expect.any(String))
  })

  it('is flat and says order is fixed once the sort is not the shared order', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-010', title: 'Task A', statusId: 's-done' }), sub('a2', 'PFI-012', { statusId: 's-doing' })]))
    mount(`${BOARD_PATH}?d=1&sort=TITLE:ASC`)
    await screen.findByText('A2')
    expect(document.querySelector('[data-context-id]')).toBeNull()
    expect(screen.getByText(/Order cannot be changed by dragging|not hand-sorted|drag/i)).toBeTruthy()
  })
})

describe('Board settings: Workflow', () => {
  const board = (manage: boolean) => ({ ...fixtureBoard([ticket({ id: 'a', title: 'One' })]), canManageWorkflow: manage, workflowVersion: 4 })
  const openSettings = async () => {
    await screen.findByText('One')
    fireEvent.click(screen.getByRole('button', { name: /Board settings/ }))
  }

  it('lets anyone read the States in Workflow order but gives no controls to a non-admin', async () => {
    api.getProjectBoard.mockResolvedValue(board(false))
    mount(BOARD_PATH)
    await openSettings()
    const list = screen.getByRole('list', { name: 'Workflow' })
    expect(within(list).getAllByRole('listitem').map((item) => item.textContent)).toEqual([expect.stringContaining('Todo'), expect.stringContaining('In Progress'), expect.stringContaining('Done')])
    expect(screen.queryByRole('button', { name: 'Add' })).toBeNull()
    expect(screen.queryByRole('button', { name: /^Delete/ })).toBeNull()
    expect(screen.getByRole('note').textContent).toContain('Project admins')
  })

  it('adds a State at the end with the workflow version', async () => {
    api.getProjectBoard.mockResolvedValue(board(true))
    api.createWorkflowState.mockResolvedValue({ version: 5, state: fixtureStates[1] })
    mount(BOARD_PATH)
    await openSettings()
    fireEvent.change(screen.getByLabelText('State name'), { target: { value: 'Review' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    await waitFor(() => expect(api.createWorkflowState).toHaveBeenCalledWith('p', { expectedWorkflowVersion: 4, name: 'Review', category: 'IN_PROGRESS', position: 3 }))
  })

  it('renames a State and explains why a State with tickets cannot be deleted', async () => {
    api.getProjectBoard.mockResolvedValue(board(true))
    api.updateWorkflowState.mockResolvedValue({ version: 5, state: fixtureStates[1] })
    api.deleteWorkflowState.mockRejectedValue(Object.assign(new Error('in use'), { code: 'state_in_use', details: { tickets: 3 } }))
    mount(BOARD_PATH)
    await openSettings()
    fireEvent.change(screen.getByLabelText('Name of In Progress'), { target: { value: 'Doing' } })
    fireEvent.click(screen.getAllByRole('button', { name: 'Save' })[0])
    await waitFor(() => expect(api.updateWorkflowState).toHaveBeenCalledWith('p', 's-doing', { expectedWorkflowVersion: 4, name: 'Doing' }))
    fireEvent.click(screen.getByRole('button', { name: 'Delete Done' }))
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Delete' })).getByRole('button', { name: 'Delete' }))
    expect((await screen.findByRole('alert')).textContent).toContain('3 ticket')
    expect(api.deleteWorkflowState).toHaveBeenCalledWith('p', 's-done', 4)
  })
})

describe('Planning page', () => {
  function mountPlanning() {
    return renderWithProviders(
      <><PlanningPage workspaceId="w" projectId="p" /><DetailPanelHost /></>,
      { route: '/w/w/projects/p/planning', path: '/w/:workspaceId/projects/:projectId/planning' },
    )
  }

  it('guides an admin to create the first Sprint and Milestone', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [], milestones: [] })
    api.createSprint.mockResolvedValue({})
    mountPlanning()
    expect((await screen.findByText(/No Sprints yet/)).textContent).toContain('Create a Sprint')
    fireEvent.click(screen.getByRole('button', { name: 'Create a Sprint' }))
    fireEvent.change(await screen.findByLabelText('Sprint name'), { target: { value: 'Sprint 1' } })
    fireEvent.change(screen.getByLabelText('Starts'), { target: { value: '2026-11-02' } })
    fireEvent.change(screen.getByLabelText('Ends'), { target: { value: '2026-11-13' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createSprint).toHaveBeenCalledWith('p', { name: 'Sprint 1', startsOn: '2026-11-02', endsOn: '2026-11-13' }, expect.any(String)))
    fireEvent.click(screen.getByRole('tab', { name: /Milestone/ }))
    expect(screen.getByText(/No Milestones yet/).textContent).toContain('Create a Milestone')
  })

  it('fills a new Sprint after the last one with a two-week length, and asks before starting one', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    const planned = { id: 's9', projectId: 'p', name: 'Sprint 9', goal: null, state: 'PLANNED', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 1, startedAt: null, endedAt: null }
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [planned], milestones: [] })
    api.sprintCommand.mockResolvedValue({ ...planned, state: 'ACTIVE' })
    mountPlanning()
    fireEvent.click(await screen.findByRole('button', { name: 'Create a Sprint' }))
    const dates = (await screen.findAllByDisplayValue(/^2026-11-/)).map((input) => (input as HTMLInputElement).value)
    expect(dates).toEqual(['2026-11-14', '2026-11-27'])
    fireEvent.click(within(screen.getByRole('dialog', { name: 'Create a Sprint' })).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Create a Sprint' })).toBeNull())
    fireEvent.click(await screen.findByRole('button', { name: 'Start' }))
    expect(api.sprintCommand).not.toHaveBeenCalled()
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Start Sprint 9?' })).getByRole('button', { name: 'Start' }))
    await waitFor(() => expect(api.sprintCommand).toHaveBeenCalledWith('s9', 'start', 1, expect.any(String)))
  })

  it('says why a Sprint cannot start while another is active', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    const base = { projectId: 'p', goal: null, startsOn: '2026-11-02', endsOn: '2026-11-13', version: 1, startedAt: null, endedAt: null }
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [{ ...base, id: 'a', name: 'Sprint A', state: 'ACTIVE' }, { ...base, id: 'b', name: 'Sprint B', state: 'PLANNED' }], milestones: [] })
    mountPlanning()
    expect((await screen.findByRole('button', { name: 'Start' })).hasAttribute('disabled')).toBe(true)
    expect(screen.getByText(/Sprint A is already active/)).toBeTruthy()
  })

  it('tells a member that an admin creates Sprints', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    mountPlanning()
    expect((await screen.findByText(/No Sprints yet/)).textContent).toContain('A project admin can create them')
    expect(screen.queryByLabelText('Sprint name')).toBeNull()
  })

  it('closes a Sprint with one decision: the preview defaults to the next Sprint and sends it', async () => {
    const active = { id: 's1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'ACTIVE', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 2, startedAt: null, endedAt: null }
    const next = { id: 's2', projectId: 'p', name: 'Sprint 2', goal: null, state: 'PLANNED', startsOn: '2026-11-16', endsOn: '2026-11-27', version: 1, startedAt: null, endedAt: null }
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One' })]))
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [active, next], milestones: [] })
    api.getSprintResults.mockResolvedValue({ sprint: active, baselineCount: 1, baselineRecordedAt: null, results: [] })
    api.getSprintClosePreview.mockResolvedValue({
      sprintId: 's1', sprintVersion: 2,
      done: [{ id: 'd', key: 'PFI-009', title: 'Done thing', hierarchyKind: 'TASK' }], canceled: [],
      unfinished: [{ id: 'u', key: 'PFI-010', title: 'Left over', hierarchyKind: 'TASK' }],
      targets: [{ id: 's2', name: 'Sprint 2', state: 'PLANNED', startsOn: '2026-11-16' }], defaultTargetId: 's2',
    })
    api.sprintCommand.mockResolvedValue({ ...active, state: 'COMPLETED' })
    mountPlanning()
    fireEvent.click(await screen.findByRole('button', { name: 'Complete' }))
    expect(await screen.findByText('1 done · 0 canceled · 1 unfinished')).toBeTruthy()
    expect((screen.getByRole('radio', { name: /Sprint 2/ }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Close Sprint' }))
    await waitFor(() => expect(api.sprintCommand).toHaveBeenCalledWith('s1', 'complete', 2, expect.any(String), 's2'))
  })

  it('sends the backlog when the person picks it', async () => {
    const active = { id: 's1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'ACTIVE', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 2, startedAt: null, endedAt: null }
    api.getProjectBoard.mockResolvedValue(fixtureBoard([]))
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [active], milestones: [] })
    api.getSprintResults.mockResolvedValue({ sprint: active, baselineCount: 0, baselineRecordedAt: null, results: [] })
    api.getSprintClosePreview.mockResolvedValue({ sprintId: 's1', sprintVersion: 2, done: [], canceled: [], unfinished: [], targets: [], defaultTargetId: null })
    api.sprintCommand.mockResolvedValue({ ...active, state: 'COMPLETED' })
    mountPlanning()
    fireEvent.click(await screen.findByRole('button', { name: 'Complete' }))
    expect((await screen.findByRole('radio', { name: 'Backlog' }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Close Sprint' }))
    await waitFor(() => expect(api.sprintCommand).toHaveBeenCalledWith('s1', 'complete', 2, expect.any(String), null))
  })

  it('lists a closed Sprint from its recorded results, not from a filter', async () => {
    const closed = { id: 's1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'COMPLETED', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 3, startedAt: null, endedAt: '2026-11-13T00:00:00Z' }
    api.getProjectBoard.mockResolvedValue(fixtureBoard([]))
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [closed], milestones: [] })
    const row = (key: string, title: string, outcome: string, extra: object = {}) => ({ ticket: { id: key, key, title, hierarchyKind: 'TASK' }, outcome, carriedTo: null, carriedFrom: null, needsNextAssignment: false, inBaseline: true, assignedAt: '2026-11-02T00:00:00Z', endedAt: '2026-11-13T00:00:00Z', ...extra })
    api.getSprintResults.mockResolvedValue({
      sprint: closed, baselineCount: 3, baselineRecordedAt: null,
      results: [row('PFI-1', 'Shipped', 'DONE'), row('PFI-2', 'Moved on', 'NOT_DONE', { carriedTo: { id: 's2', name: 'Sprint 2', state: 'PLANNED', startsOn: '2026-11-16' } }), row('PFI-3', 'Hidden after', 'NOT_DONE', { needsNextAssignment: true, inBaseline: false })],
    })
    mountPlanning()
    fireEvent.click(await screen.findByRole('button', { name: 'Show tickets' }))
    expect(await screen.findByText('Shipped')).toBeTruthy()
    expect(screen.getByText('Not done → Sprint 2')).toBeTruthy()
    expect(screen.getByText('needs next assignment')).toBeTruthy()
    expect(screen.getByText('added after start')).toBeTruthy()
    expect(screen.getByText(/Done 1/)).toBeTruthy()
  })
})

describe('cards outside a State grouping', () => {
  it('name their State when the View groups by assignee, and not when it groups by State', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', title: 'One', statusId: 's-doing' })]))
    mount(`${BOARD_PATH}?d=1&group=ASSIGNEE`)
    const card = (await screen.findByText('One')).closest('article') as HTMLElement
    expect(within(card).getByText('In Progress')).toBeTruthy()
    cleanup()
    mount(BOARD_PATH)
    const plain = (await screen.findByText('One')).closest('article') as HTMLElement
    expect(within(plain).queryByText('In Progress')).toBeNull()
  })
})

describe('move to top', () => {
  const parent = { id: 'p1', key: 'PFI-010', title: 'Parent', hierarchyKind: 'TASK' as const }
  const group = [
    ticket({ id: 'p1', key: 'PFI-010', title: 'Parent', updatedAt: '2026-10-19T10:00:00Z' }),
    ticket({ id: 'a', key: 'PFI-001', title: 'First', hierarchyKind: 'SUBTASK', parentId: 'p1', parent }),
    ticket({ id: 'b', key: 'PFI-002', title: 'Second', hierarchyKind: 'SUBTASK', parentId: 'p1', parent }),
    ticket({ id: 'z', key: 'PFI-020', title: 'Lone card', updatedAt: '2026-10-19T12:00:00Z' }),
  ]

  it('puts a parent first in the shared order and moves its group on top at once', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(group))
    api.moveTicket.mockImplementation(() => {
      // The server now holds the stamp, so the next fetch agrees with the card's new place.
      api.getProjectBoard.mockResolvedValue(fixtureBoard(group.map((item) => (item.id === 'p1' ? { ...item, movedToTopAt: new Date().toISOString() } : item))))
      return Promise.resolve(ticket({ id: 'p1' }))
    })
    mount(BOARD_PATH)
    await screen.findByText('Lone card')
    const order = () => [...document.querySelectorAll('section.board-col[aria-label="Todo"] .ticket-card-title')].map((node) => node.textContent)
    expect(order()).toEqual(['Lone card', 'Parent', 'First', 'Second'])
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-010 to top' }))
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledWith('p1', { position: 'first' }, expect.any(String)))
    await waitFor(() => expect(order()).toEqual(['Parent', 'First', 'Second', 'Lone card']))
  })

  it('puts a child first among its siblings, and offers nothing for the one already first', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(group))
    api.moveTicket.mockResolvedValue(ticket({ id: 'b' }))
    mount(BOARD_PATH)
    await screen.findByText('Second')
    expect(screen.queryByRole('button', { name: 'Move PFI-001 to top' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-002 to top' }))
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledWith('b', { beforeId: 'a' }, expect.any(String)))
  })

  it('works from the keyboard with Alt+Home', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(group))
    api.moveTicket.mockResolvedValue(ticket({ id: 'p1' }))
    mount(BOARD_PATH)
    const open = (await screen.findByText('Parent', { selector: '.ticket-card-title' })).closest('article')?.querySelector('button')
    if (!open) throw new Error('card button missing')
    fireEvent.keyDown(open, { key: 'Home', altKey: true })
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledWith('p1', { position: 'first' }, expect.any(String)))
  })

  it('is not offered where the View orders by something else, nor to a read-only member', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(group))
    mount(`${BOARD_PATH}?d=1&sort=TITLE:ASC`)
    await screen.findByText('Lone card')
    expect(screen.queryByRole('button', { name: /to top/ })).toBeNull()
    cleanup()
    api.getProjectBoard.mockResolvedValue(fixtureBoard(group, false))
    mount(BOARD_PATH)
    await screen.findByText('Lone card')
    expect(screen.queryByRole('button', { name: /to top/ })).toBeNull()
  })
})
