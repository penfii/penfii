import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ScopeDataset, ScopedTicket } from '@penfii/contracts'
import { fixtureBoard, fixtureStates, fixtureTicket as ticket } from './testFixtures'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getSpace: vi.fn(), getProjectLabels: vi.fn(), getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(),
  getProjectViews: vi.fn(), getWideViews: vi.fn(), createWideView: vi.fn(), createProjectView: vi.fn(), updateProjectView: vi.fn(),
  getProjectPlanning: vi.fn(), getScopeDataset: vi.fn(), changeTicketStatus: vi.fn(), bulkTickets: vi.fn(), getProjectWorkflow: vi.fn(),
  moveTicket: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(), getTicketRevisions: vi.fn(),
  getTicketDependencies: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), updateWorkflowState: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { Work } from './Work'
import { renderWithProviders } from '../../test/renderWithProviders'

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}

function mount(path: string, element: React.ReactNode, route = '/w/:workspaceId/projects/:projectId/work') {
  return renderWithProviders(<>{element}<Where /></>, { route: path, path: route })
}

const project = (id: string, name: string, canEdit = true) => ({ id, name, canEdit })
function scoped(partial: Partial<ScopedTicket>): ScopedTicket {
  return { ...ticket({}), projectName: 'Alpha', channelName: null, stateName: 'Todo', stateCategory: 'NOT_STARTED', ...partial }
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [{ id: 'c1', name: 'general', projectId: 'p', conversationId: 'c1', unreadCount: 0, visibility: 'PUBLIC', teamId: null }], dms: [] })
  api.getProjectLabels.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getWideViews.mockResolvedValue([])
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: false, pins: [] })
})
afterEach(cleanup)

const projectWork = <Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} />

describe('Work in a Project: one dataset, three layouts', () => {
  const tickets = [
    ticket({ id: 'a', key: 'PFI-001', title: 'API login', statusId: 's-todo', priority: 'HIGH', assigneeId: 'u1', assigneeName: 'John Park', channelId: 'c1' }),
    ticket({ id: 'b', key: 'PFI-002', title: 'Login page', statusId: 's-doing' }),
  ]

  it('opens as a Board and switches to List and Table without losing a Ticket', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    mount('/w/w/projects/p/work', projectWork)
    expect(await screen.findByText('API login')).toBeTruthy()
    const layouts = screen.getByRole('group', { name: 'Layout' })
    expect(within(layouts).getAllByRole('button').map((button) => button.textContent)).toEqual(['Board', 'List', 'Table'])
    expect(document.querySelector('.board')).not.toBeNull()

    fireEvent.click(within(layouts).getByRole('button', { name: 'List' }))
    expect(await screen.findByLabelText('Open PFI-001')).toBeTruthy()
    expect(document.querySelector('.board')).toBeNull()
    expect(screen.getByTestId('where').textContent).toContain('layout=list')
    expect(screen.getByLabelText('Open PFI-002')).toBeTruthy()
    // The list names a channel the Ticket lives in; the Board and the List read one dataset.
    expect(screen.getByText('# general')).toBeTruthy()

    fireEvent.click(within(layouts).getByRole('button', { name: 'Table' }))
    const table = await screen.findByRole('table')
    expect(within(table).getAllByRole('columnheader').map((header) => header.textContent?.trim())).toEqual(expect.arrayContaining(['Key', 'Title', 'State', 'Assignee', 'Priority']))
    expect(within(table).getByText('API login')).toBeTruthy()
    expect(within(table).getByText('Login page')).toBeTruthy()
    expect(api.getProjectBoard).toHaveBeenCalledTimes(1)
  })

  it('keeps the layout in the address so a refresh returns to it', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    mount('/w/w/projects/p/work?layout=table', projectWork)
    expect(await screen.findByRole('table')).toBeTruthy()
    expect(screen.getByText('You have unsaved changes.')).toBeTruthy()
  })

  it('saves the layout with the View', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.createProjectView.mockResolvedValue({ id: 'v9', projectId: 'p', targetKind: 'PROJECT', teamId: null, name: 'My table', layout: 'TABLE', scope: 'PERSONAL', ownerId: 'u1', definition: {}, version: 1, canEdit: true, createdAt: '', updatedAt: '' })
    mount('/w/w/projects/p/work?layout=table', projectWork)
    await screen.findByRole('table')
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'My table' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createProjectView).toHaveBeenCalledTimes(1))
    expect(api.createProjectView).toHaveBeenCalledWith('p', expect.objectContaining({ name: 'My table', layout: 'TABLE' }), expect.any(String))
  })

  it('groups a List by State category with each Ticket still showing its own State', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    mount('/w/w/projects/p/work?layout=list&d=1&group=CATEGORY', projectWork)
    await screen.findByLabelText('Open PFI-001')
    const headings = screen.getAllByRole('heading', { level: 3 }).map((heading) => heading.textContent)
    expect(headings.join(' ')).toContain('Not started')
    expect(headings.join(' ')).toContain('In progress')
    expect(screen.getAllByText('In Progress').length).toBeGreaterThan(0)
  })

  it('changes a State from the row menu and explains a block by a canceled predecessor', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.changeTicketStatus.mockRejectedValueOnce(Object.assign(new Error('blocked'), { code: 'dependency_blocked', details: { openBlockers: 1, canceledBlockers: 1 } }))
    mount('/w/w/projects/p/work?layout=list', projectWork)
    const row = (await screen.findByLabelText('Open PFI-001')).closest('li') as HTMLElement
    fireEvent.click(within(row).getByRole('button', { name: 'Todo' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Done' }))
    await waitFor(() => expect(api.changeTicketStatus).toHaveBeenCalledWith('a', { expectedRevision: 1, statusId: 's-done' }, expect.any(String)))
    expect((await screen.findByRole('alert')).textContent).toContain('canceled predecessor')
  })

  it('moves a row up in the shared order from the List', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-001', title: 'One' }), ticket({ id: 'b', key: 'PFI-002', title: 'Two' })]))
    api.moveTicket.mockResolvedValue({})
    mount('/w/w/projects/p/work?layout=list&d=1&group=NONE&subtasks=0', projectWork)
    await screen.findByLabelText('Open PFI-002')
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-002 up' }))
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledWith('b', { beforeId: 'a' }, expect.any(String)))
  })

  it('hides the reorder buttons when the View is not in the shared order', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    mount('/w/w/projects/p/work?layout=list&d=1&sort=TITLE:ASC', projectWork)
    await screen.findByLabelText('Open PFI-001')
    expect(screen.queryByRole('button', { name: /Move PFI-001/ })).toBeNull()
  })
})

describe('Work in a Team or the Workspace: List and Table, never a Board', () => {
  const dataset: ScopeDataset = {
    projects: [project('pa', 'Alpha'), project('pb', 'Beta', false)],
    truncated: false,
    tickets: [
      scoped({ id: 'a', key: 'A-1', title: 'Alpha work', projectId: 'pa', projectName: 'Alpha', statusId: 'sa', stateName: 'Todo', stateCategory: 'NOT_STARTED' }),
      scoped({ id: 'b', key: 'B-1', title: 'Beta work', projectId: 'pb', projectName: 'Beta', statusId: 'sb', stateName: 'Review', stateCategory: 'IN_PROGRESS', channelName: 'beta-chat', channelId: 'cb' }),
      scoped({ id: 'c', key: 'B-2', title: 'Beta shipped', projectId: 'pb', projectName: 'Beta', statusId: 'sb2', stateName: 'Shipped', stateCategory: 'DONE' }),
    ],
  }
  const teamWork = <Work target={{ kind: 'TEAM', workspaceId: 'w', teamId: 't1' }} />
  const teamRoute = '/w/:workspaceId/teams/:teamId'

  it('offers only List and Table, groups by category, and shows Project and own State on every row', async () => {
    api.getScopeDataset.mockResolvedValue(dataset)
    mount('/w/w/teams/t1', teamWork, teamRoute)
    expect(await screen.findByLabelText('Open A-1')).toBeTruthy()
    const layouts = screen.getByRole('group', { name: 'Layout' })
    expect(within(layouts).getAllByRole('button').map((button) => button.textContent)).toEqual(['List', 'Table'])
    expect(screen.queryByRole('button', { name: 'Board' })).toBeNull()
    expect(document.querySelector('.board')).toBeNull()
    const headings = screen.getAllByRole('heading', { level: 3 }).map((heading) => heading.textContent ?? '')
    expect(headings.some((text) => text.startsWith('Not started'))).toBe(true)
    expect(headings.some((text) => text.startsWith('In progress'))).toBe(true)
    expect(screen.getByText('Review')).toBeTruthy()
    expect(screen.getByText('Shipped')).toBeTruthy()
    expect(screen.getAllByText(/Beta/).length).toBeGreaterThan(0)
    expect(api.getScopeDataset).toHaveBeenCalledWith('w', { kind: 'TEAM', teamId: 't1' }, 'ALL', [])
    // No drag-reorder in a scope that mixes Projects.
    expect(screen.queryByRole('button', { name: /Move A-1/ })).toBeNull()
  })

  it('ignores a requested Board layout in the address', async () => {
    api.getScopeDataset.mockResolvedValue(dataset)
    mount('/w/w/teams/t1?layout=board', teamWork, teamRoute)
    expect(await screen.findByLabelText('Open A-1')).toBeTruthy()
    expect(document.querySelector('.board')).toBeNull()
  })

  it('shows a Table with the Project column and lets a read-only Project keep its State fixed', async () => {
    api.getScopeDataset.mockResolvedValue(dataset)
    mount('/w/w/teams/t1?layout=table', teamWork, teamRoute)
    const table = await screen.findByRole('table')
    expect(within(table).getByRole('columnheader', { name: 'Project' })).toBeTruthy()
    // Alpha can be edited: its State is a button. Beta is read-only: its State is plain text.
    expect(within(table).getByRole('button', { name: 'Todo' })).toBeTruthy()
    expect(within(table).queryByRole('button', { name: 'Review' })).toBeNull()
  })

  it('asks the server for the chosen Projects, so a scope over the cap can be narrowed', async () => {
    api.getScopeDataset.mockResolvedValue(dataset)
    mount('/w/w/teams/t1?d=1&group=CATEGORY&project=pb', teamWork, teamRoute)
    await screen.findByLabelText('Open B-1')
    expect(api.getScopeDataset).toHaveBeenCalledWith('w', { kind: 'TEAM', teamId: 't1' }, 'ALL', ['pb'])
  })

  it('says when the scope is only partly shown', async () => {
    api.getScopeDataset.mockResolvedValue({ ...dataset, truncated: true })
    mount('/w/w/teams/t1', teamWork, teamRoute)
    expect(await screen.findByText(/1000 most recently updated/)).toBeTruthy()
  })

  it('saves a Team View against the Team', async () => {
    api.getScopeDataset.mockResolvedValue(dataset)
    api.createWideView.mockResolvedValue({ id: 'v1', projectId: null, targetKind: 'TEAM', teamId: 't1', name: 'Mine', layout: 'LIST', scope: 'PERSONAL', ownerId: 'u1', definition: {}, version: 1, canEdit: true, createdAt: '', updatedAt: '' })
    mount('/w/w/teams/t1?d=1&group=PROJECT', teamWork, teamRoute)
    await screen.findByLabelText('Open A-1')
    fireEvent.click(screen.getByRole('button', { name: 'Save view' }))
    fireEvent.change(screen.getByLabelText('View name'), { target: { value: 'Mine' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createWideView).toHaveBeenCalledTimes(1))
    expect(api.createWideView).toHaveBeenCalledWith('w', expect.objectContaining({ targetKind: 'TEAM', teamId: 't1', layout: 'LIST' }), expect.any(String))
    // A Team View is personal: there is no "share with the Project" choice.
    expect(screen.queryByLabelText('Visibility')).toBeNull()
  })

  it('serves My Work from the Workspace scope with a lens', async () => {
    api.getScopeDataset.mockResolvedValue({ projects: [project('pa', 'Alpha')], truncated: false, tickets: [dataset.tickets[0] as ScopedTicket] })
    mount('/w/w/my-work', <Work target={{ kind: 'WORKSPACE', workspaceId: 'w' }} lens="ASSIGNED" />, '/w/:workspaceId/my-work')
    expect(await screen.findByLabelText('Open A-1')).toBeTruthy()
    expect(api.getScopeDataset).toHaveBeenCalledWith('w', { kind: 'WORKSPACE' }, 'ASSIGNED', [])
  })
})

describe('bulk changes from a Table', () => {
  it('reports every Ticket and does not turn a partial failure into success', async () => {
    const tickets = [ticket({ id: 'a', key: 'PFI-001', title: 'One' }), ticket({ id: 'b', key: 'PFI-002', title: 'Two' }), ticket({ id: 'c', key: 'PFI-003', title: 'Three' })]
    api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
    api.bulkTickets.mockResolvedValue({
      total: 3, succeeded: 2, failed: 1,
      items: [
        { ticketId: 'a', key: 'PFI-001', ok: true, code: null, message: null },
        { ticketId: 'b', key: null, ok: false, code: 'dependency_blocked', message: 'blocked' },
        { ticketId: 'c', key: 'PFI-003', ok: true, code: null, message: null },
      ],
    })
    api.getProjectWorkflow.mockResolvedValue({ projectId: 'p', version: 1, states: fixtureStates })
    mount('/w/w/projects/p/work?layout=table', projectWork)
    const table = await screen.findByRole('table')
    expect(screen.queryByRole('region', { name: 'Bulk change' })).toBeNull()
    fireEvent.click(within(table).getByLabelText('Select all'))
    const bar = await screen.findByRole('region', { name: 'Bulk change' })
    expect(within(bar).getByText('3 selected')).toBeTruthy()
    fireEvent.change(within(bar).getByLabelText('Change'), { target: { value: 'STATUS' } })
    await within(bar).findByRole('option', { name: 'Done' })
    fireEvent.change(within(bar).getByLabelText('Value'), { target: { value: 's-done' } })
    fireEvent.click(within(bar).getByRole('button', { name: 'Apply' }))
    await waitFor(() => expect(api.bulkTickets).toHaveBeenCalledWith('w', ['a', 'b', 'c'], { type: 'STATUS', statusId: 's-done' }, expect.any(String)))
    const result = await within(bar).findByRole('status')
    expect(result.textContent).toContain('2 of 3 succeeded, 1 failed')
    expect(result.textContent).toContain('PFI-002')
    expect(result.textContent).toContain('Unfinished predecessors block completing')
    // The failed Ticket stays selected so it can be dealt with.
    await waitFor(() => expect((within(table).getByLabelText('Select PFI-002') as HTMLInputElement).checked).toBe(true))
    expect((within(table).getByLabelText('Select PFI-001') as HTMLInputElement).checked).toBe(false)
  })

  it('offers State, label, Sprint and Milestone only for Tickets of one Project', async () => {
    const dataset: ScopeDataset = {
      projects: [project('pa', 'Alpha'), project('pb', 'Beta')], truncated: false,
      tickets: [scoped({ id: 'a', key: 'A-1', title: 'Alpha work', projectId: 'pa', projectName: 'Alpha' }), scoped({ id: 'b', key: 'B-1', title: 'Beta work', projectId: 'pb', projectName: 'Beta' })],
    }
    api.getScopeDataset.mockResolvedValue(dataset)
    mount('/w/w/teams/t1?layout=table', <Work target={{ kind: 'TEAM', workspaceId: 'w', teamId: 't1' }} />, '/w/:workspaceId/teams/:teamId')
    const table = await screen.findByRole('table')
    fireEvent.click(within(table).getByLabelText('Select all'))
    const bar = await screen.findByRole('region', { name: 'Bulk change' })
    fireEvent.change(within(bar).getByLabelText('Change'), { target: { value: 'STATUS' } })
    expect(within(bar).getByText(/one project/i)).toBeTruthy()
    expect((within(bar).getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(within(bar).getByLabelText('Change'), { target: { value: 'PRIORITY' } })
    expect(within(bar).queryByText(/one project/i)).toBeNull()
    expect((within(bar).getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(false)
  })
})

describe('a read-only Table', () => {
  it('offers no selection, because nothing could be changed', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-001', title: 'One' })], false))
    mount('/w/w/projects/p/work?layout=table', projectWork)
    const table = await screen.findByRole('table')
    expect(within(table).queryByRole('checkbox')).toBeNull()
    expect(screen.queryByRole('region', { name: 'Bulk change' })).toBeNull()
  })
})

describe('a fully successful bulk change', () => {
  it('stays reported after the selection is gone', async () => {
    api.getProjectBoard.mockResolvedValue(fixtureBoard([ticket({ id: 'a', key: 'PFI-001', title: 'One' })]))
    api.bulkTickets.mockResolvedValue({ total: 1, succeeded: 1, failed: 0, items: [{ ticketId: 'a', key: 'PFI-001', ok: true, code: null, message: null }] })
    mount('/w/w/projects/p/work?layout=table', projectWork)
    const table = await screen.findByRole('table')
    fireEvent.click(within(table).getByLabelText('Select PFI-001'))
    const bar = await screen.findByRole('region', { name: 'Bulk change' })
    fireEvent.change(within(bar).getByLabelText('Value'), { target: { value: 'HIGH' } })
    fireEvent.click(within(bar).getByRole('button', { name: 'Apply' }))
    expect((await within(bar).findByRole('status')).textContent).toContain('1 of 1 succeeded, 0 failed')
    expect((within(table).getByLabelText('Select PFI-001') as HTMLInputElement).checked).toBe(false)
    expect(screen.getByRole('region', { name: 'Bulk change' })).toBeTruthy()
  })
})

describe('reordering in a hierarchical List', () => {
  it('moves a child among its siblings and only puts a parent on top', async () => {
    const parent = ticket({ id: 'p1', key: 'PFI-001', title: 'Parent', childSummary: { total: 2, done: 0, canceled: 0 } })
    const kid = (id: string, key: string) => ticket({ id, key, title: id, hierarchyKind: 'SUBTASK', parentId: 'p1', parent: { id: 'p1', key: 'PFI-001', title: 'Parent', hierarchyKind: 'TASK' } })
    api.getProjectBoard.mockResolvedValue(fixtureBoard([parent, kid('k1', 'PFI-002'), kid('k2', 'PFI-003')]))
    api.moveTicket.mockResolvedValue({})
    mount('/w/w/projects/p/work?layout=list&d=1&group=NONE', projectWork)
    await screen.findByLabelText('Open PFI-003')
    expect(screen.queryByRole('button', { name: 'Move PFI-001 up' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-003 up' }))
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalledWith('k2', { beforeId: 'k1' }, expect.any(String)))
  })
})
