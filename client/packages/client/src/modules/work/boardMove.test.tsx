import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'
import { refreshTicketChanges } from './refresh'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getSpace: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), moveTicket: vi.fn(), getProjectViews: vi.fn(), getProjectPlanning: vi.fn(),
  assignSprint: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { Work } from './Work'
import { renderWithProviders } from '../../test/renderWithProviders'
import { DESKTOP, setViewport } from '../../test/viewport'

const tickets = [ticket({ id: 'a', key: 'PFI-001', title: 'API login', statusId: 's-todo' })]

function mount() {
  return renderWithProviders(<Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} />, {
    route: '/w/w/projects/p/work?layout=board',
    path: '/w/:workspaceId/projects/:projectId/work',
  })
}
const column = (name: string) => screen.getByRole('region', { name })
const inColumn = (name: string) => within(column(name)).queryByText('API login') !== null

/** Moves the card with the keyboard: Alt+→ is "next State". */
async function moveRight() {
  fireEvent.keyDown(await screen.findByRole('button', { name: 'Open PFI-001' }), { key: 'ArrowRight', altKey: true })
}

beforeEach(() => {
  setViewport(DESKTOP)
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: true, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getProjectBoard.mockImplementation(() => Promise.resolve(fixtureBoard(tickets)))
})
afterEach(cleanup)

describe('moving a card on the Board', () => {
  it('shows the card in its new column at once and keeps it there when a read that started earlier comes back', async () => {
    let answer: (value: unknown) => void = () => undefined
    api.moveTicket.mockImplementation(() => new Promise((resolve) => { answer = resolve }))
    const { client } = mount()
    await screen.findByText('API login')
    expect(inColumn('Todo')).toBe(true)
    await moveRight()
    await waitFor(() => expect(inColumn('In Progress')).toBe(true))
    // A read of the Board is asked for while the server has not answered; it answers with the old place.
    await act(async () => {
      await refreshTicketChanges(client, 'p', [{ ticketId: 'other', kind: 'ticket.updated' }])
    })
    expect(inColumn('In Progress')).toBe(true)
    expect(api.getProjectBoard).toHaveBeenCalledTimes(1)
    // The server answers: the card stays, and the Board is read once, now.
    api.getProjectBoard.mockImplementation(() => Promise.resolve(fixtureBoard([{ ...tickets[0]!, statusId: 's-doing', revisionNo: 2 }])))
    await act(async () => answer({ ...tickets[0], statusId: 's-doing', revisionNo: 2 }))
    await waitFor(() => expect(api.getProjectBoard).toHaveBeenCalledTimes(2))
    expect(inColumn('In Progress')).toBe(true)
  })

  it('puts the card back and says why when the server refuses the move', async () => {
    api.moveTicket.mockRejectedValue(new ApiError(403, 'forbidden', 'no'))
    mount()
    await screen.findByText('API login')
    await moveRight()
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalled())
    await waitFor(() => expect(inColumn('Todo')).toBe(true))
    expect(inColumn('In Progress')).toBe(false)
  })

  it('puts the card back when the browser is offline, without needing the Board to be read again', async () => {
    api.moveTicket.mockRejectedValue(new ApiError(0, 'offline', 'You are offline'))
    mount()
    await screen.findByText('API login')
    // Reads are paused while offline: the Board never comes back.
    api.getProjectBoard.mockImplementation(() => new Promise(() => undefined))
    await moveRight()
    await waitFor(() => expect(inColumn('Todo')).toBe(true))
    expect(inColumn('In Progress')).toBe(false)
  })

  it('shows the server\'s own picture once the move has worked', async () => {
    api.moveTicket.mockResolvedValue({ ...tickets[0], statusId: 's-doing', revisionNo: 2 })
    api.getProjectBoard.mockImplementationOnce(() => Promise.resolve(fixtureBoard(tickets)))
    mount()
    await screen.findByText('API login')
    api.getProjectBoard.mockImplementation(() => Promise.resolve(fixtureBoard([{ ...tickets[0]!, statusId: 's-doing', revisionNo: 2 }])))
    await moveRight()
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalled())
    await waitFor(() => expect(api.getProjectBoard.mock.calls.length).toBeGreaterThan(1))
    expect(inColumn('In Progress')).toBe(true)
  })
})
