import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureBoard, fixtureTicket } from './testFixtures'
import { textToDescription } from './ticketModel'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getProjectWorkflow: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(),
  getTicketRevisions: vi.fn(), getTicketRevision: vi.fn(), createTicket: vi.fn(), updateTicket: vi.fn(),
  changeTicketStatus: vi.fn(), createTicketComment: vi.fn(), restoreTicket: vi.fn(), createProjectLabel: vi.fn(),
  moveTicket: vi.fn(), getProjectViews: vi.fn(), getProjectPlanning: vi.fn(), getTicketDependencies: vi.fn(), assignSprint: vi.fn(),
  setTicketMilestones: vi.fn(), addTicketDependency: vi.fn(), removeDependency: vi.fn(), updateWorkflowState: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { Work } from './Work'
import { DetailPanelHost } from './testHost'
import { renderWithProviders } from '../../test/renderWithProviders'

const ticket = fixtureTicket
const boardOf = fixtureBoard

function mount(path: string) {
  return renderWithProviders(<><Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} /><DetailPanelHost /></>, { route: path, path: '/w/:workspaceId/projects/:projectId/work' })
}

const BOARD_PATH = '/w/w/projects/p/work'

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([{ id: 'l1', name: 'backend', color: null }])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: false, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getTicketDependencies.mockResolvedValue({ ticketId: 't1', blockedBy: [], blocks: [], related: [], canEdit: true, canWaive: false })
  api.getTicketComments.mockResolvedValue([])
  api.getTicketActivity.mockResolvedValue({ items: [], nextCursor: null })
  api.getTicketRevisions.mockResolvedValue({ items: [], nextBefore: null })
})
afterEach(cleanup)

describe('Board', () => {
  it('shows compact cards with child progress and hides subtasks on request', async () => {
    const task = ticket({ id: 'task', title: 'Parent task', childSummary: { total: 2, done: 1, canceled: 0 }, assigneeName: 'John Park', priority: 'HIGH' })
    const sub = ticket({ id: 'sub', key: 'PFI-003', title: 'Child work', hierarchyKind: 'SUBTASK', parentId: 'task', parent: { id: 'task', key: 'PFI-001', title: 'Parent task', hierarchyKind: 'TASK' } })
    api.getProjectBoard.mockResolvedValue(boardOf([task, sub]))
    mount(BOARD_PATH)
    expect(await screen.findByText('Parent task')).toBeTruthy()
    expect(screen.getByText('1/2 subtasks')).toBeTruthy()
    expect(screen.getByText('High')).toBeTruthy()
    expect(screen.getByText(/↑ PFI-001/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Display' }))
    fireEvent.click(screen.getByLabelText('Show subtasks as tickets'))
    expect(screen.queryByText('Child work')).toBeNull()
  })

  it('opens the single Create Ticket panel from the board and shows the new ticket with its key', async () => {
    const created = ticket({ id: 'new', key: 'PFI-001', title: 'Write release notes', priority: 'HIGH', assigneeId: 'u1', assigneeName: 'John Park', dueOn: '2026-12-24', labels: [{ id: 'l1', name: 'backend', color: null }] })
    let board = boardOf([])
    api.getProjectBoard.mockImplementation(() => Promise.resolve(board))
    api.createTicket.mockImplementation(() => {
      board = boardOf([created])
      return Promise.resolve(created)
    })
    mount(BOARD_PATH)
    fireEvent.click(await screen.findByText('Add ticket'))
    const panel = await screen.findByRole('complementary', { name: 'Create ticket' })
    // Nothing is created by opening the panel, and a title is required.
    expect(api.createTicket).not.toHaveBeenCalled()
    const create = await within(panel).findByRole('button', { name: 'Create' })
    expect((create as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(within(panel).getByLabelText('Title *'), { target: { value: 'Write release notes' } })
    fireEvent.change(within(panel).getByLabelText('Description'), { target: { value: 'first line\nsecond line' } })
    fireEvent.change(within(panel).getByLabelText('Assignee'), { target: { value: 'u1' } })
    fireEvent.change(within(panel).getByLabelText('Priority'), { target: { value: 'HIGH' } })
    fireEvent.click(within(panel).getByText('More properties'))
    fireEvent.change(within(panel).getByLabelText('Due date'), { target: { value: '2026-12-24' } })
    fireEvent.click(within(panel).getByRole('button', { name: 'backend' }))
    // A new ticket starts in the initial state; the field shows it but cannot change it.
    expect((within(panel).getByLabelText('State') as HTMLSelectElement).disabled).toBe(true)
    fireEvent.click(create)
    await waitFor(() => expect(api.createTicket).toHaveBeenCalledTimes(1))
    expect(api.createTicket).toHaveBeenCalledWith('p', {
      title: 'Write release notes',
      description: textToDescription('first line\nsecond line'),
      hierarchyKind: 'TASK', parentId: null, channelId: null, statusId: 's-todo', assigneeId: 'u1', priority: 'HIGH',
      dueOn: '2026-12-24', labelIds: ['l1'], typeId: null,
    }, expect.any(String))
    expect(await screen.findByText('PFI-001')).toBeTruthy()
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
  })

  it('keeps everything that was entered when creating fails', async () => {
    api.getProjectBoard.mockResolvedValue(boardOf([]))
    api.createTicket.mockRejectedValue(new Error('boom'))
    mount(BOARD_PATH)
    fireEvent.click(await screen.findByText('Add ticket'))
    const title = (await screen.findByLabelText('Title *')) as HTMLInputElement
    fireEvent.change(title, { target: { value: 'Keep me' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    expect((await screen.findByRole('alert')).textContent).toContain('Could not create')
    expect(title.value).toBe('Keep me')
  })

  it('requires a parent for a subtask and takes the channel from it', async () => {
    const parentTask = ticket({ id: 'task', key: 'PFI-002', title: 'Parent task', channelId: 'c1' })
    api.getProjectBoard.mockResolvedValue(boardOf([parentTask]))
    api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [{ id: 'c1', name: 'backend', projectId: 'p' }], dms: [] })
    mount(BOARD_PATH)
    await screen.findByText('Parent task')
    cleanup()
    mount(`${BOARD_PATH}?panel=ticket-new&newProject=p&newKind=SUBTASK`)
    const title = await screen.findByLabelText('Title *')
    fireEvent.change(title, { target: { value: 'Collect logs' } })
    expect((screen.getByRole('button', { name: 'Create' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Channel'), { target: { value: 'c1' } })
    fireEvent.change(screen.getByLabelText('Parent'), { target: { value: 'task' } })
    expect((screen.getByLabelText('Channel') as HTMLSelectElement).disabled).toBe(true)
    expect((screen.getByRole('button', { name: 'Create' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('opens Create from a parent ticket with the parent and subtask kind already set', async () => {
    const parentTask = ticket({ id: 'task', key: 'PFI-002', title: 'Parent task', channelId: 'c1', childSummary: { total: 0, done: 0, canceled: 0 } })
    api.getProjectBoard.mockResolvedValue(boardOf([parentTask]))
    api.getTicket.mockResolvedValue(parentTask)
    mount(`${BOARD_PATH}?panel=ticket:task`)
    fireEvent.click(await screen.findByRole('button', { name: '+ Add subtask' }))
    const panel = await screen.findByRole('complementary', { name: 'Create ticket' })
    expect((within(panel).getByRole('radio', { name: 'Subtask' }) as HTMLInputElement).checked).toBe(true)
    expect((within(panel).getByLabelText('Parent') as HTMLSelectElement).value).toBe('task')
    expect((within(panel).getByLabelText('Channel') as HTMLSelectElement).disabled).toBe(true)
    // The panel says whose subtask this is without scrolling to the Parent field.
    expect(within(panel).getByRole('note').textContent).toBe('Creating a subtask of PFI-002 Parent task.')
  })

  it('is read only without edit access', async () => {
    api.getProjectBoard.mockResolvedValue(boardOf([ticket({})], false))
    mount(BOARD_PATH)
    expect(await screen.findByText('Fix login')).toBeTruthy()
    expect(screen.queryByText('Add ticket')).toBeNull()
    expect(screen.getByRole('note').textContent).toContain('Read only')
  })

  it('shows an error with retry when the board cannot load', async () => {
    api.getProjectBoard.mockRejectedValue(new Error('down'))
    mount(BOARD_PATH)
    expect(await screen.findByText('Could not load the board.')).toBeTruthy()
  })
})

describe('Ticket panel', () => {
  const task = ticket({
    id: 'task', title: 'Fix login', childSummary: { total: 2, done: 1, canceled: 0 }, revisionNo: 3,
    parent: { id: 'epic', key: 'PFI-010', title: 'Stability', hierarchyKind: 'EPIC' }, parentId: 'epic',
  })
  const epic = ticket({ id: 'epic', key: 'PFI-010', title: 'Stability', hierarchyKind: 'EPIC' })

  beforeEach(() => {
    api.getProjectBoard.mockResolvedValue(boardOf([task, epic]))
    api.getTicket.mockImplementation((id: string) => Promise.resolve(id === 'task' ? task : epic))
    api.getTicketChildren.mockResolvedValue([ticket({ id: 'sub', key: 'PFI-003', title: 'Collect logs', hierarchyKind: 'SUBTASK', parentId: 'task', statusId: 's-done' })])
  })

  it('reads in the order Description, Hierarchy and subtasks, Dependencies, Sprint and Milestone, then Comments', async () => {
    mount(`${BOARD_PATH}?panel=ticket:task`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    await within(panel).findByText('Collect logs')
    const headings = [...panel.querySelectorAll('.ticket-section > h3, .ticket-section [role="tablist"]')].map((node) => (node.tagName === 'H3' ? (node.textContent ?? '') : 'tabs'))
    const at = (label: string) => headings.findIndex((text) => text.toLowerCase().includes(label))
    expect(at('description')).toBeGreaterThan(-1)
    expect(at('description')).toBeLessThan(at('hierarchy'))
    expect(at('hierarchy')).toBeLessThan(at('dependenc'))
    // Sprint and Milestone sit in the properties column beside the main reading order.
    expect(at('dependenc')).toBeLessThan(headings.indexOf('tabs'))
  })

  it('opens beside the board with parent, subtasks and progress', async () => {
    mount(`${BOARD_PATH}?panel=ticket:task`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    expect(await within(panel).findByRole('button', { name: 'Open PFI-010' })).toBeTruthy()
    expect(await within(panel).findByText('Collect logs')).toBeTruthy()
    expect(within(panel).getByText('1/2 done')).toBeTruthy()
    expect(screen.getByText('Fix login', { selector: '.ticket-card-title' })).toBeTruthy()
  })

  it('saves a title edit against the revision that was read', async () => {
    api.updateTicket.mockResolvedValue({ ...task, title: 'Fix login now', revisionNo: 4 })
    mount(`${BOARD_PATH}?panel=ticket:task`)
    const input = await screen.findByLabelText('Title')
    fireEvent.change(input, { target: { value: 'Fix login now' } })
    fireEvent.blur(input)
    await waitFor(() => expect(api.updateTicket).toHaveBeenCalled())
    expect(api.updateTicket).toHaveBeenCalledWith('task', { title: 'Fix login now', expectedRevision: 3 }, expect.any(String))
  })

  it('keeps my change next to the latest value when someone saved first', async () => {
    const conflict = Object.assign(new Error('conflict'), { code: 'version_conflict', status: 409 })
    api.updateTicket.mockRejectedValueOnce(conflict)
    api.getTicket.mockImplementation((id: string) => Promise.resolve(id === 'task' ? { ...task, title: 'Fix login (theirs)', revisionNo: 4 } : epic))
    mount(`${BOARD_PATH}?panel=ticket:task`)
    const input = (await screen.findByLabelText('Title')) as HTMLInputElement
    fireEvent.change(input, { target: { value: 'Fix login (mine)' } })
    fireEvent.blur(input)
    const banner = await screen.findByRole('alert', { name: 'Someone else changed this ticket first' })
    expect(within(banner).getByText('Fix login (mine)')).toBeTruthy()
    expect(within(banner).getByText('Fix login (theirs)')).toBeTruthy()
    expect(input.value).toBe('Fix login (mine)')
    api.updateTicket.mockResolvedValueOnce({ ...task, title: 'Fix login (mine)', revisionNo: 5 })
    fireEvent.click(within(banner).getByText('Re-apply on latest'))
    await waitFor(() => expect(api.updateTicket).toHaveBeenLastCalledWith('task', { title: 'Fix login (mine)', expectedRevision: 4 }, expect.any(String)))
  })

  it('lists activity as sentences and history as versions, opened on demand', async () => {
    api.getTicketActivity.mockResolvedValue({
      items: [{ id: 'a1', ticketId: 'task', revisionNo: 3, activityKind: 'ticket.updated', actorUserId: 'u1', actorName: 'Kyu', data: {}, occurredAt: '2026-10-19T01:00:00Z', changes: [{ field: 'priority', from: 'MEDIUM', to: 'HIGH' }] }],
      nextCursor: null,
    })
    api.getTicketRevisions.mockResolvedValue({ items: [{ revisionNo: 2, actorUserId: 'u1', actorName: 'Kyu', recordedAt: '2026-10-19T01:00:00Z', changedFields: ['priority'], restoreSourceRevisionNo: null }], nextBefore: null })
    api.getTicketRevision.mockResolvedValue({
      revisionNo: 2, currentRevisionNo: 3, actorUserId: 'u1', actorName: 'Kyu', recordedAt: '2026-10-19T01:00:00Z', restoreSourceRevisionNo: null,
      changesFromPrevious: [{ field: 'priority', from: 'MEDIUM', to: 'HIGH' }],
      changesToCurrent: [{ field: 'title', from: 'Old title', to: 'Fix login' }], restorable: [{ field: 'title', applicable: true, reason: null }],
    })
    api.restoreTicket.mockResolvedValue({ ...task, title: 'Old title', revisionNo: 6 })
    mount(`${BOARD_PATH}?panel=ticket:task`)
    fireEvent.click(await screen.findByRole('tab', { name: 'Activity' }))
    expect(await screen.findByText('Kyu changed the priority')).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: 'History' }))
    fireEvent.click(await screen.findByText('View values'))
    expect(await screen.findByText('Different from now')).toBeTruthy()
    expect(api.getTicketRevision).toHaveBeenCalledWith('task', 2)
    fireEvent.click(screen.getByLabelText('Title', { selector: 'input[type="checkbox"]' }))
    fireEvent.click(screen.getByText('Restore selected'))
    await waitFor(() => expect(api.restoreTicket).toHaveBeenCalledWith('task', { expectedRevision: 3, sourceRevision: 2, fields: ['title'] }, expect.any(String)))
  })

  it('shows the not-found state for a ticket the viewer cannot see', async () => {
    api.getTicket.mockRejectedValue(Object.assign(new Error('nf'), { name: 'ApiError', status: 404, code: 'not_found' }))
    api.getProjectBoard.mockResolvedValue(boardOf([]))
    mount(`${BOARD_PATH}?panel=ticket:ghost`)
    expect(await screen.findByText(/does not exist or you cannot see it|Could not load the ticket/)).toBeTruthy()
  })

  it('is read only for viewers', async () => {
    api.getProjectBoard.mockResolvedValue(boardOf([task, epic], false))
    mount(`${BOARD_PATH}?panel=ticket:task`)
    const panel = await screen.findByRole('complementary', { name: 'Ticket detail' })
    expect((await within(panel).findByRole('note')).textContent).toContain('Read only')
    expect(within(panel).queryByLabelText('Title')).toBeNull()
    expect((within(panel).getByLabelText('Priority') as HTMLSelectElement).disabled).toBe(true)
  })
})
