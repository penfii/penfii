import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'

// The same set of calls the Work and Ticket panel tests make; the phone path reads the same data.
const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getProjectWorkflow: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(), getTicketRevisions: vi.fn(),
  changeTicketStatus: vi.fn(), addLater: vi.fn(), removeLater: vi.fn(), addFavorite: vi.fn(), removeFavorite: vi.fn(),
  bulkTickets: vi.fn(), moveTicket: vi.fn(), getProjectViews: vi.fn(), getProjectPlanning: vi.fn(), getTicketDependencies: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { Work } from './Work'
import { DetailPanelHost } from './testHost'
import { renderWithProviders } from '../../test/renderWithProviders'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'

function mount(path: string) {
  return renderWithProviders(<><Work target={{ kind: 'PROJECT', workspaceId: 'w', projectId: 'p' }} /><DetailPanelHost /></>, { route: path, path: '/w/:workspaceId/projects/:projectId/work' })
}

const PATH = '/w/w/projects/p/work'
const tickets = [
  ticket({ id: 'a', key: 'PFI-001', title: 'API login', statusId: 's-todo', priority: 'HIGH', assigneeId: 'u1', assigneeName: 'John Park' }),
  ticket({ id: 'b', key: 'PFI-002', title: 'Login page', statusId: 's-doing' }),
]

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: true, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [], blocks: [], related: [], canEdit: true, canWaive: false })
  api.getTicketComments.mockResolvedValue([])
  api.getTicketActivity.mockResolvedValue({ items: [], nextCursor: null })
  api.getTicketRevisions.mockResolvedValue({ items: [], nextBefore: null })
  api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
  api.getTicket.mockResolvedValue(tickets[0])
})
afterEach(cleanup)

describe('Work on a phone', () => {
  beforeEach(() => setViewport(PHONE))

  it('shows a Table view as a List: there is no Table on a phone', async () => {
    mount(`${PATH}?layout=table`)
    expect(await screen.findByText('API login')).toBeTruthy()
    expect(document.querySelector('table')).toBeNull()
    expect(document.querySelector('.work-list')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'List' }).getAttribute('aria-pressed')).toBe('true')
    // The Table button stays in the markup for a saved Table view; the phone stylesheet hides it.
    expect(document.querySelector('[data-layout="TABLE"]')).toBeTruthy()
  })

  it('chooses the fields a List row shows in the Display sheet', async () => {
    mount(`${PATH}?layout=list`)
    expect(await screen.findByText('High')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Display' }))
    const sheet = await screen.findByRole('dialog', { name: 'Display' })
    expect(sheet.classList.contains('m-sheet')).toBe(true)
    expect(within(sheet).getByText('Fields shown')).toBeTruthy()
    fireEvent.click(within(sheet).getByLabelText('Priority'))
    await waitFor(() => expect(screen.queryByText('High')).toBeNull())
    // The key and title are not optional.
    expect((within(sheet).getByLabelText('Key') as HTMLInputElement).disabled).toBe(true)
  })

  it('offers sort and group in one sheet, and the filter in another', async () => {
    mount(`${PATH}?layout=list`)
    await screen.findByText('API login')
    fireEvent.click(screen.getByRole('button', { name: 'Sort and group' }))
    const sortSheet = await screen.findByRole('dialog', { name: 'Sort and group' })
    expect(within(sortSheet).getByLabelText('Group')).toBeTruthy()
    expect(within(sortSheet).getByLabelText('Sort')).toBeTruthy()
    fireEvent.click(within(sortSheet).getByText('Close'))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Sort and group' })).toBeNull())
    fireEvent.click(screen.getByRole('button', { name: 'Filter' }))
    expect((await screen.findByRole('dialog', { name: 'Filter' })).classList.contains('m-sheet')).toBe(true)
  })

  it('changes the State of a Ticket from a sheet in its own screen', async () => {
    api.changeTicketStatus.mockResolvedValue({ ...tickets[0], statusId: 's-doing' })
    mount(`${PATH}?panel=ticket:a`)
    fireEvent.click(await screen.findByRole('button', { name: 'Change state' }))
    const sheet = await screen.findByRole('dialog', { name: 'Change state' })
    fireEvent.click(within(sheet).getByRole('button', { name: /In Progress/ }))
    await waitFor(() => expect(api.changeTicketStatus).toHaveBeenCalled())
    expect(api.changeTicketStatus.mock.calls[0][1]).toMatchObject({ statusId: 's-doing' })
    expect(screen.queryByRole('dialog', { name: 'Change state' })).toBeNull()
  })

  it('saves a Ticket to Favorites or Later from one sheet', async () => {
    api.addLater.mockResolvedValue({})
    api.addFavorite.mockResolvedValue({})
    mount(`${PATH}?panel=ticket:a`)
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }))
    const sheet = await screen.findByRole('dialog', { name: 'Save' })
    fireEvent.click(await within(sheet).findByRole('button', { name: /Save for later/ }))
    await waitFor(() => expect(api.addLater).toHaveBeenCalledWith('w', 'a'))
    expect(api.addFavorite).not.toHaveBeenCalled()
    fireEvent.click(within(sheet).getByRole('button', { name: /Add to Favorites/ }))
    await waitFor(() => expect(api.addFavorite).toHaveBeenCalledWith('w', 'TICKET', 'a'))
    // A Project admin may also pin from here.
    expect(within(sheet).getByRole('button', { name: /Pin/ })).toBeTruthy()
  })

  it('does not save twice while a save is still running', async () => {
    let finish: () => void = () => undefined
    api.addLater.mockReturnValue(new Promise<void>((resolve) => { finish = resolve }))
    mount(`${PATH}?panel=ticket:a`)
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }))
    const sheet = await screen.findByRole('dialog', { name: 'Save' })
    const row = await within(sheet).findByRole('button', { name: /Save for later/ })
    fireEvent.click(row)
    await waitFor(() => expect((row as HTMLButtonElement).disabled).toBe(true))
    fireEvent.click(row)
    expect(api.addLater).toHaveBeenCalledTimes(1)
    finish()
  })

  describe('select mode', () => {
    async function enter() {
      mount(`${PATH}?layout=list`)
      await screen.findByText('API login')
      fireEvent.click(screen.getByRole('button', { name: 'Select' }))
    }

    it('chooses rows instead of opening them, and shows the count', async () => {
      await enter()
      expect(screen.getByText('Choose tickets')).toBeTruthy()
      fireEvent.click(screen.getByRole('button', { name: 'Open PFI-001' }))
      expect(screen.getByText('1 selected')).toBeTruthy()
      expect((screen.getByRole('checkbox', { name: 'Select PFI-001' }) as HTMLInputElement).checked).toBe(true)
      expect(document.querySelector('[role="dialog"]')).toBeNull()
      fireEvent.click(screen.getByRole('button', { name: 'Select all' }))
      expect(screen.getByText('2 selected')).toBeTruthy()
      fireEvent.click(screen.getByRole('button', { name: 'Select all' }))
      expect(screen.getByText('Choose tickets')).toBeTruthy()
      expect((screen.getByRole('button', { name: 'Change' }) as HTMLButtonElement).disabled).toBe(true)
    })

    it('applies nothing until a value is tapped, then changes every chosen Ticket in one request', async () => {
      api.bulkTickets.mockResolvedValue({ total: 2, succeeded: 2, failed: 0, items: [] })
      await enter()
      fireEvent.click(screen.getByRole('button', { name: 'Select all' }))
      fireEvent.click(screen.getByRole('button', { name: 'Change' }))
      const sheet = await screen.findByRole('dialog', { name: 'Bulk change' })
      fireEvent.click(within(sheet).getByRole('button', { name: 'Priority' }))
      const apply = within(sheet).getByRole('button', { name: 'Apply to 2' }) as HTMLButtonElement
      expect(apply.disabled).toBe(true)
      fireEvent.click(within(sheet).getByRole('button', { name: 'No priority' }))
      fireEvent.click(apply)
      await waitFor(() => expect(api.bulkTickets).toHaveBeenCalledWith('w', ['a', 'b'], { type: 'PRIORITY', priority: null }, expect.any(String)))
      expect(await screen.findByText('2 of 2 succeeded, 0 failed')).toBeTruthy()
      expect(screen.queryByRole('dialog', { name: 'Bulk change' })).toBeNull()
    })

    it('keeps the sheet open with an error when the request fails, and the selection after a partial failure', async () => {
      api.bulkTickets.mockRejectedValueOnce(new Error('offline'))
      await enter()
      fireEvent.click(screen.getByRole('button', { name: 'Open PFI-001' }))
      fireEvent.click(screen.getByRole('button', { name: 'Change' }))
      const sheet = await screen.findByRole('dialog', { name: 'Bulk change' })
      fireEvent.click(within(sheet).getByRole('button', { name: 'Priority' }))
      fireEvent.click(within(sheet).getByRole('button', { name: 'No priority' }))
      fireEvent.click(within(sheet).getByRole('button', { name: 'Apply to 1' }))
      expect(await within(sheet).findByRole('alert')).toBeTruthy()
      api.bulkTickets.mockResolvedValueOnce({ total: 1, succeeded: 0, failed: 1, items: [{ ticketId: 'a', key: 'PFI-001', ok: false, code: 'forbidden', message: null }] })
      fireEvent.click(within(sheet).getByRole('button', { name: 'Apply to 1' }))
      expect(await screen.findByText('0 of 1 succeeded, 1 failed')).toBeTruthy()
      expect(screen.getByText('1 selected')).toBeTruthy()
    })

    it('leaves select mode with Done and clears the choice', async () => {
      await enter()
      fireEvent.click(screen.getByRole('button', { name: 'Open PFI-001' }))
      fireEvent.click(screen.getByRole('button', { name: 'Done' }))
      expect(screen.queryByRole('checkbox')).toBeNull()
      expect(screen.queryByRole('region', { name: 'Bulk change' })).toBeNull()
    })
  })

  it('moves a Board card to another State by name, since a card cannot be dragged', async () => {
    api.moveTicket.mockResolvedValue({ ...tickets[0], statusId: 's-doing' })
    mount(`${PATH}?layout=board`)
    await screen.findByText('API login')
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-001' }))
    const sheet = await screen.findByRole('dialog', { name: 'Move PFI-001' })
    fireEvent.click(within(sheet).getByRole('button', { name: /In Progress/ }))
    await waitFor(() => expect(api.moveTicket).toHaveBeenCalled())
    expect(api.moveTicket.mock.calls[0][0]).toBe('a')
    expect(api.moveTicket.mock.calls[0][1]).toMatchObject({ statusId: 's-doing' })
  })

  it('keeps the sort and group controls in the sheet only, not inline', async () => {
    mount(`${PATH}?layout=list`)
    await screen.findByText('API login')
    expect(document.querySelector('.view-inline-order')).toBeNull()
  })
})

describe('Work on a desktop-sized window', () => {
  beforeEach(() => setViewport(DESKTOP))

  it('still shows the Table, with columns in Display, and no sheets', async () => {
    mount(`${PATH}?layout=table`)
    expect(await screen.findByText('API login')).toBeTruthy()
    expect(document.querySelector('table')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Display' }))
    expect(await screen.findByText('Table columns')).toBeTruthy()
    expect(document.querySelector('.m-sheet')).toBeNull()
    // Group, sort and direction are inline on a wide screen; the phone's single trigger is not rendered at all.
    expect(document.querySelector('.view-inline-order .view-select')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Sort and group' })).toBeNull()
    // Select mode is a phone List's; the desktop Table has its own checkboxes and bar.
    expect(screen.queryByRole('button', { name: 'Select' })).toBeNull()
  })

  it('does not offer the phone Move button on a Board card', async () => {
    mount(`${PATH}?layout=board`)
    await screen.findByText('API login')
    expect(screen.queryByRole('button', { name: 'Move PFI-001' })).toBeNull()
  })

  it('keeps the two Ticket save buttons and the native status menu', async () => {
    mount(`${PATH}?panel=ticket:a`)
    expect(await screen.findByRole('button', { name: 'Add to Favorites' })).toBeTruthy()
    expect(screen.getByRole('combobox', { name: 'Change state' })).toBeTruthy()
  })
})
