import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getFavorites: vi.fn(), addFavorite: vi.fn(), removeFavorite: vi.fn(), orderFavorites: vi.fn(),
  getLater: vi.fn(), addLater: vi.fn(), updateLater: vi.fn(), removeLater: vi.fn(), getProjectPins: vi.fn(), addProjectPin: vi.fn(), removePin: vi.fn(),
  getDrafts: vi.fn(), deleteDraft: vi.fn(), getDraft: vi.fn(), saveDraft: vi.fn(),
  getFavoriteFolders: vi.fn(), createFavoriteFolder: vi.fn(), deleteFavoriteFolder: vi.fn(), moveFavorite: vi.fn(), renameFavoriteFolder: vi.fn(), setReminder: vi.fn(), cancelReminder: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { DraftsPage } from './DraftsPage'
import { FavoriteButton, LaterButton } from './SavedButtons'
import { PinsPanel } from './PinsPanel'
import { SavedPageRoute } from './SavedRoute'
import { renderWithProviders } from '../../test/renderWithProviders'

function mount(path: string, element: React.ReactNode, route: string) {
  return renderWithProviders(element, { route: path, path: route })
}

const ref = (kind: string, targetId: string, title: string | null, extra: object = {}) => ({ kind, targetId, available: title !== null, title, subtitle: null, projectId: 'p', teamId: null, viewTarget: null, layout: null, position: 1024, createdAt: '', ...extra })

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getFavoriteFolders.mockResolvedValue([])
})
afterEach(cleanup)

describe('Favorite and Later buttons', () => {
  it('toggles a Favorite without touching anything else', async () => {
    api.addFavorite.mockResolvedValue(ref('TICKET', 't1', 'PFI-001 Fix'))
    mount('/w/w', <FavoriteButton workspaceId="w" kind="TICKET" targetId="t1" />, '/w/:workspaceId')
    const star = (await screen.findByRole('button', { name: 'Add to Favorites' })) as HTMLButtonElement
    await waitFor(() => expect(star.disabled).toBe(false))
    fireEvent.click(star)
    await waitFor(() => expect(api.addFavorite).toHaveBeenCalledWith('w', 'TICKET', 't1'))
  })

  it('shows a saved Favorite as pressed and removes it', async () => {
    api.getFavorites.mockResolvedValue([ref('TICKET', 't1', 'PFI-001 Fix')])
    api.removeFavorite.mockResolvedValue(undefined)
    mount('/w/w', <FavoriteButton workspaceId="w" kind="TICKET" targetId="t1" />, '/w/:workspaceId')
    const star = await screen.findByRole('button', { name: 'Remove from Favorites' })
    expect(star.getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(star)
    await waitFor(() => expect(api.removeFavorite).toHaveBeenCalledWith('w', 'TICKET', 't1'))
  })

  it('saves a Ticket for later and takes it out again', async () => {
    api.addLater.mockResolvedValue({})
    mount('/w/w', <LaterButton workspaceId="w" ticketId="t1" />, '/w/:workspaceId')
    const clock = (await screen.findByRole('button', { name: 'Save for later' })) as HTMLButtonElement
    await waitFor(() => expect(clock.disabled).toBe(false))
    fireEvent.click(clock)
    await waitFor(() => expect(api.addLater).toHaveBeenCalledWith('w', 't1'))
  })
})

describe('Saved page', () => {
  it('lists Favorites with links and reorders them', async () => {
    api.getFavorites.mockResolvedValue([ref('PROJECT', 'p', 'Penfii App'), ref('TICKET', 't1', 'PFI-001 Fix', { subtitle: 'Penfii App' })])
    api.orderFavorites.mockResolvedValue([])
    mount('/w/w/saved', <SavedPageRoute />, '/w/:workspaceId/saved')
    const link = await screen.findByRole('link', { name: /PFI-001 Fix/ })
    expect(link.getAttribute('href')).toBe('/w/w/projects/p/work?panel=ticket:t1')
    fireEvent.click(screen.getByRole('button', { name: 'Move PFI-001 Fix up' }))
    await waitFor(() => expect(api.orderFavorites).toHaveBeenCalledWith('w', [{ kind: 'TICKET', targetId: 't1' }, { kind: 'PROJECT', targetId: 'p' }]))
  })

  it('finishes a Later card without anything about the Ticket changing, and hides what can no longer be opened', async () => {
    api.getLater.mockResolvedValue([
      { ...ref('TICKET', 't1', 'PFI-001 Fix'), id: 'l1', note: 'after release', state: 'ACTIVE', version: 1, createdAt: '', updatedAt: '' },
      { ...ref('TICKET', 't2', null), id: 'l2', note: 'my note', state: 'ACTIVE', version: 1, createdAt: '', updatedAt: '' },
    ])
    api.updateLater.mockResolvedValue({})
    mount('/w/w/saved', <SavedPageRoute />, '/w/:workspaceId/saved')
    fireEvent.click(await screen.findByRole('tab', { name: 'Later' }))
    expect(await screen.findByText('after release')).toBeTruthy()
    expect(screen.getByText('The linked item cannot be opened')).toBeTruthy()
    expect(screen.queryByText(/t2/)).toBeNull()
    fireEvent.click(screen.getAllByRole('button', { name: 'Mark handled' })[0] as HTMLElement)
    await waitFor(() => expect(api.updateLater).toHaveBeenCalledWith('l1', { expectedVersion: 1, state: 'DONE' }))
  })
})

describe('Pins', () => {
  it('lets only an admin pin a link and shows the pins to everyone', async () => {
    api.getProjectPins.mockResolvedValue({ canManage: false, pins: [{ id: 'x', kind: 'URL', title: 'Runbook', url: 'https://example.test/r', ticket: null, position: 1, pinnedBy: 'u', createdAt: '' }] })
    mount('/w/w/projects/p', <PinsPanel projectId="p" />, '/w/:workspaceId/projects/:projectId')
    expect(await screen.findByRole('link', { name: 'Runbook' })).toBeTruthy()
    expect(screen.queryByLabelText('Link address')).toBeNull()
    cleanup()
    api.getProjectPins.mockResolvedValue({ canManage: true, pins: [] })
    api.addProjectPin.mockResolvedValue({ canManage: true, pins: [] })
    mount('/w/w/projects/p', <PinsPanel projectId="p" />, '/w/:workspaceId/projects/:projectId')
    fireEvent.change(await screen.findByLabelText('Link address'), { target: { value: 'https://example.test/x' } })
    fireEvent.click(screen.getByRole('button', { name: 'Pin link' }))
    await waitFor(() => expect(api.addProjectPin).toHaveBeenCalledWith('p', { url: 'https://example.test/x', title: null }))
  })
})

describe('Drafts page', () => {
  it('lists unsent messages with a way back and deletes one', async () => {
    api.getDrafts.mockResolvedValue([
      { id: 'd1', contextKind: 'CHANNEL_COMPOSE', contextId: 'c1', conversationId: 'c1', threadRootId: null, title: 'general', projectId: null, preview: 'half written', updatedAt: '2026-10-22T00:00:00Z' },
      { id: 'd2', contextKind: 'THREAD_COMPOSE', contextId: 'm1', conversationId: 'c2', threadRootId: 'm1', title: 'build', projectId: 'p', preview: 'in a thread', updatedAt: '2026-10-22T00:00:00Z' },
    ])
    api.deleteDraft.mockResolvedValue(undefined)
    mount('/w/w/drafts', <DraftsPage />, '/w/:workspaceId/drafts')
    const first = await screen.findByRole('link', { name: /half written/ })
    expect(first.getAttribute('href')).toBe('/w/w/c/c1')
    expect(within(screen.getByRole('link', { name: /in a thread/ })).getByText('Thread')).toBeTruthy()
    expect(screen.getByRole('link', { name: /in a thread/ }).getAttribute('href')).toBe('/w/w/projects/p/chat/c2/t/m1')
    fireEvent.click(screen.getAllByRole('button', { name: 'Delete draft' })[0] as HTMLElement)
    await waitFor(() => expect(api.deleteDraft).toHaveBeenCalledWith('d1'))
  })
})
