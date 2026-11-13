import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'

const api = vi.hoisted(() => ({
  getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getProjectWorkflow: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getTicketComments: vi.fn(), getTicketActivity: vi.fn(), getTicketRevisions: vi.fn(), getTicketAttachments: vi.fn(),
  getSubscription: vi.fn(), watch: vi.fn(), unwatch: vi.fn(), addLater: vi.fn(), removeLater: vi.fn(), addFavorite: vi.fn(), removeFavorite: vi.fn(),
  getProjectViews: vi.fn(), getProjectPlanning: vi.fn(), getTicketDependencies: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { DetailPanelHost } from './testHost'
import { NoticeHost, dismissNotice } from '../../design/notices'
import { renderWithProviders } from '../../test/renderWithProviders'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'

const PATH = '/w/w/projects/p/work'
const tickets = [
  ticket({ id: 'a', key: 'PFI-001', title: 'API login', statusId: 's-todo' }),
  ticket({ id: 'b', key: 'PFI-002', title: 'Login page', statusId: 's-doing' }),
]

function mount() {
  return renderWithProviders(<><DetailPanelHost /><NoticeHost /></>, { route: `${PATH}?panel=ticket:a`, path: '/w/:workspaceId/projects/:projectId/work' })
}

/** A panel of the given width: the observer reports it once and the head folds its actions or not. */
function panelWidth(width: number) {
  vi.stubGlobal('ResizeObserver', class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) { this.callback([{ target, contentRect: { width } } as ResizeObserverEntry], this as unknown as ResizeObserver) }
    unobserve() {}
    disconnect() {}
  })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: true, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getTicketDependencies.mockResolvedValue({ ticketId: 'a', blockedBy: [], blocks: [], related: [], canEdit: true, canWaive: false })
  api.getTicketComments.mockResolvedValue([])
  api.getTicketAttachments.mockResolvedValue([])
  api.getTicketActivity.mockResolvedValue({ items: [], nextCursor: null })
  api.getTicketRevisions.mockResolvedValue({ items: [], nextBefore: null })
  api.getSubscription.mockResolvedValue({ targetKind: 'TICKET', targetId: 'a', mode: 'NONE', watching: false, version: 0 })
  api.getProjectBoard.mockResolvedValue(fixtureBoard(tickets))
  api.getTicket.mockResolvedValue(tickets[0])
  setViewport(DESKTOP)
})
afterEach(() => { act(() => dismissNotice()); cleanup(); vi.unstubAllGlobals() })

describe('ticket panel header', () => {
  it('keeps Previous / Next as their own buttons, apart from Watch and Save', async () => {
    mount()
    const group = await screen.findByRole('group', { name: 'Neighbouring tickets' })
    const prev = within(group).getByRole('button', { name: 'Previous —' }) as HTMLButtonElement
    const next = within(group).getByRole('button', { name: 'Next PFI-002' })
    expect(prev.disabled).toBe(true)
    expect(next.getAttribute('title')).toBe('Next PFI-002')
    // Watch and Save are not inside the navigation group, so one cannot sit on top of the other.
    expect(within(group).queryByRole('button', { name: /watch|later|favorite/i })).toBeNull()
    expect(screen.getByRole('button', { name: 'Get ordinary updates for this item' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'More actions' })).toBeNull()
  })

  it('folds Favorite, Later and Pin into a More menu in a narrow panel and keeps Watch and Previous / Next', async () => {
    panelWidth(480)
    api.addLater.mockResolvedValue({})
    mount()
    const more = await screen.findByRole('button', { name: 'More actions' })
    expect(screen.getByRole('group', { name: 'Neighbouring tickets' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Get ordinary updates for this item' })).toBeTruthy()
    fireEvent.click(more)
    const menu = await screen.findByRole('menu', { name: 'More actions' })
    expect(within(menu).getAllByRole('menuitemcheckbox').map((item) => item.textContent)).toEqual([
      expect.stringContaining('Add to Favorites'),
      expect.stringContaining('Save for later'),
      expect.stringContaining('Pin for the project'),
    ])
    fireEvent.click(within(menu).getByRole('menuitemcheckbox', { name: /Save for later/ }))
    await waitFor(() => expect(api.addLater).toHaveBeenCalledWith('w', 'a'))
    expect(screen.queryByRole('menu')).toBeNull()
    expect((await screen.findByText('Saved for later'))).toBeTruthy()
  })

  it('closes the menu on Escape without closing the ticket, and returns focus to the button', async () => {
    panelWidth(480)
    mount()
    const more = await screen.findByRole('button', { name: 'More actions' })
    fireEvent.click(more)
    const menu = await screen.findByRole('menu')
    fireEvent.keyDown(within(menu).getAllByRole('menuitemcheckbox')[0], { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(document.activeElement).toBe(more)
    expect(screen.getByRole('complementary', { name: /ticket/i })).toBeTruthy()
  })

  it('does not show Previous / Next on a phone', async () => {
    setViewport(PHONE)
    mount()
    await screen.findByRole('button', { name: 'Save' })
    expect(screen.queryByRole('group', { name: 'Neighbouring tickets' })).toBeNull()
    expect(screen.queryByRole('button', { name: /^(Previous|Next) / })).toBeNull()
  })

  it('announces Stop watching with Undo, and Undo watches again', async () => {
    api.getSubscription.mockResolvedValue({ targetKind: 'TICKET', targetId: 'a', mode: 'AUTO', watching: true, version: 1 })
    api.unwatch.mockResolvedValue({ watching: false })
    api.watch.mockResolvedValue({ watching: true })
    mount()
    const button = await screen.findByRole('button', { name: 'Stop watching' })
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(button)
    await waitFor(() => expect(api.unwatch).toHaveBeenCalledWith('TICKET', 'a'))
    const notice = await screen.findByText(/^Stopped watching/)
    expect(notice.closest('[role="status"]')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
    await waitFor(() => expect(api.watch).toHaveBeenCalledWith('TICKET', 'a'))
    expect(await screen.findByText('Now watching')).toBeTruthy()
  })

  it('keeps a failed Watch change on screen until dismissed', async () => {
    api.watch.mockRejectedValue(new Error('no'))
    mount()
    const button = await screen.findByRole('button', { name: 'Get ordinary updates for this item' })
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(button)
    expect(await screen.findByText('Could not change watching')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByText('Could not change watching')).toBeNull()
  })
})
