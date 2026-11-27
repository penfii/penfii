import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// FEA-199: one Favorite state and change path for every star, menu row and Saved row, and the places
// that list Favorites (chat, docs, views, search).
const api = vi.hoisted(() => ({
  getFavorites: vi.fn(), addFavorite: vi.fn(), removeFavorite: vi.fn(), orderFavorites: vi.fn(), moveFavorite: vi.fn(),
  getFavoriteFolders: vi.fn(), getSpace: vi.fn(), getLater: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))
vi.mock('../../shell/create', () => ({ useCreate: () => () => undefined }))

import type { FavoriteItem, SavedRef, SearchResult } from '@penfii/contracts'
import { QueryClient } from '@tanstack/react-query'
import { FavoriteButton } from './SavedButtons'
import { SavedPageRoute } from './SavedRoute'
import { savedLink, savedTitle } from './SavedPage'
import { NoticeHost, dismissNotice } from '../../design/notices'
import { ContextSidebar } from '../../shell/ContextSidebar'
import { favoriteViewsFirst } from '../work/ViewBar'
import { favoriteTarget } from '../search/resultHref'
import { renderWithProviders } from '../../test/renderWithProviders'

const fav = (kind: FavoriteItem['kind'], targetId: string, title: string | null, extra: Partial<FavoriteItem> = {}): FavoriteItem => ({
  kind, targetId, available: true, title, subtitle: null, projectId: null, teamId: null, viewTarget: null, layout: null, position: 1024, createdAt: '', folderId: null, ...extra,
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

beforeEach(() => {
  localStorage.clear()
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getFavorites.mockResolvedValue([])
  api.getFavoriteFolders.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
})
afterEach(() => {
  dismissNotice()
  cleanup()
})

describe('A Favorite star (FEA-200)', () => {
  it('is outlined, then filled, and two stars of one target share one change in progress', async () => {
    const saving = deferred<FavoriteItem>()
    api.addFavorite.mockReturnValue(saving.promise)
    renderWithProviders(<><FavoriteButton workspaceId="w" kind="TICKET" targetId="t1" label="PFI-1" /><FavoriteButton workspaceId="w" kind="TICKET" targetId="t1" /><NoticeHost /></>, { route: '/' })
    const [first, second] = await screen.findAllByRole('button', { name: /Add .*to Favorites/ }) as HTMLButtonElement[]
    await waitFor(() => expect(first!.disabled).toBe(false))
    expect(first!.getAttribute('aria-pressed')).toBe('false')
    expect(first!.querySelector('path')!.getAttribute('fill')).toBe('none')
    fireEvent.click(first!)
    // The other star of the same target waits too: no second request.
    await waitFor(() => expect(second!.disabled).toBe(true))
    fireEvent.click(second!)
    expect(api.addFavorite).toHaveBeenCalledTimes(1)
    api.getFavorites.mockResolvedValue([fav('TICKET', 't1', 'PFI-1 Fix')])
    await act(async () => saving.resolve(fav('TICKET', 't1', 'PFI-1 Fix')))
    const on = await screen.findByRole('button', { name: 'Remove PFI-1 from Favorites' })
    expect(on.getAttribute('aria-pressed')).toBe('true')
    expect(on.querySelector('path')!.getAttribute('fill')).toBe('currentColor')
    expect(screen.getByRole('button', { name: 'Remove from Favorites' }).getAttribute('aria-pressed')).toBe('true')
    expect(await screen.findByText('Added to Favorites')).toBeTruthy()
  })

  it('says a failed change failed and stays as it was', async () => {
    api.addFavorite.mockRejectedValue(new Error('offline'))
    renderWithProviders(<><FavoriteButton workspaceId="w" kind="DOC" targetId="d1" /><NoticeHost /></>, { route: '/' })
    const star = await screen.findByRole('button', { name: 'Add to Favorites' }) as HTMLButtonElement
    await waitFor(() => expect(star.disabled).toBe(false))
    fireEvent.click(star)
    expect(await screen.findByText('Could not change the saved state')).toBeTruthy()
    expect(screen.queryByText('Added to Favorites')).toBeNull()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Add to Favorites' }).getAttribute('aria-pressed')).toBe('false'))
  })

  it('does not guess when the list could not be read, and checks again on a press', async () => {
    api.getFavorites.mockRejectedValueOnce(new Error('down'))
    renderWithProviders(<FavoriteButton workspaceId="w" kind="PROJECT" targetId="p1" />, { route: '/' })
    const star = await screen.findByRole('button', { name: 'Could not check Favorites. Check again' })
    expect(star.hasAttribute('aria-pressed')).toBe(false)
    api.getFavorites.mockResolvedValue([fav('PROJECT', 'p1', 'Alpha')])
    fireEvent.click(star)
    expect(await screen.findByRole('button', { name: 'Remove from Favorites' })).toBeTruthy()
    expect(api.addFavorite).not.toHaveBeenCalled()
    expect(api.removeFavorite).not.toHaveBeenCalled()
  })

  it('keeps the last known state when a later read fails', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    api.getFavorites.mockResolvedValue([fav('TEAM', 'tm', 'Ops')])
    renderWithProviders(<FavoriteButton workspaceId="w" kind="TEAM" targetId="tm" />, { route: '/', client })
    expect(await screen.findByRole('button', { name: 'Remove from Favorites' })).toBeTruthy()
    api.getFavorites.mockRejectedValue(new Error('down'))
    await act(async () => { await client.refetchQueries({ queryKey: ['favorites', 'w'] }) })
    expect(screen.getByRole('button', { name: 'Remove from Favorites' }).getAttribute('aria-pressed')).toBe('true')
  })

  it('undoes a removal back into its folder and in front of what came after it, keeping later changes', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    api.getFavorites.mockResolvedValue([
      fav('PROJECT', 'p1', 'Alpha', { position: 1024 }),
      fav('CHANNEL', 'c1', 'general', { folderId: 'f1', position: 2048 }),
      fav('DOC', 'd1', 'Handbook', { position: 3072 }),
      fav('TEAM', 'tm', 'Ops', { position: 4096 }),
    ])
    api.orderFavorites.mockResolvedValue([])
    api.removeFavorite.mockResolvedValue(undefined)
    renderWithProviders(<><FavoriteButton workspaceId="w" kind="CHANNEL" targetId="c1" /><NoticeHost /></>, { route: '/', client })
    const star = await screen.findByRole('button', { name: 'Remove from Favorites' }) as HTMLButtonElement
    // After the removal, someone puts the Team before the Doc.
    api.getFavorites.mockResolvedValue([fav('PROJECT', 'p1', 'Alpha'), fav('TEAM', 'tm', 'Ops'), fav('DOC', 'd1', 'Handbook')])
    fireEvent.click(star)
    await waitFor(() => expect(api.removeFavorite).toHaveBeenCalledWith('w', 'CHANNEL', 'c1'))
    await waitFor(() => expect(client.getQueryData<FavoriteItem[]>(['favorites', 'w'])?.map((item) => item.targetId)).toEqual(['p1', 'tm', 'd1']))
    api.addFavorite.mockResolvedValue(fav('CHANNEL', 'c1', 'general', { position: 9000 }))
    api.moveFavorite.mockResolvedValue(undefined)
    api.getFavorites.mockResolvedValue([fav('CHANNEL', 'c1', 'general', { folderId: 'f1', position: 9000 })])
    fireEvent.click(await screen.findByRole('button', { name: 'Undo' }))
    await waitFor(() => expect(api.moveFavorite).toHaveBeenCalledWith('w', 'CHANNEL', 'c1', 'f1'))
    // In front of the two that came after it, which keep their new order; the Project before it is not named.
    await waitFor(() => expect(api.orderFavorites).toHaveBeenCalledWith('w', [{ kind: 'CHANNEL', targetId: 'c1' }, { kind: 'TEAM', targetId: 'tm' }, { kind: 'DOC', targetId: 'd1' }], 'LISTED'))
    expect(await screen.findByRole('button', { name: 'Remove from Favorites' })).toBeTruthy()
  })

  it('is its own control in a row that opens: a press does not reach the row', async () => {
    const opened = vi.fn()
    renderWithProviders(<div role="presentation" onClick={opened}><FavoriteButton workspaceId="w" kind="TICKET" targetId="t9" /></div>, { route: '/' })
    api.addFavorite.mockResolvedValue(fav('TICKET', 't9', 'x'))
    const star = await screen.findByRole('button', { name: 'Add to Favorites' }) as HTMLButtonElement
    await waitFor(() => expect(star.disabled).toBe(false))
    fireEvent.keyDown(star, { key: 'Enter' })
    fireEvent.click(star)
    await waitFor(() => expect(api.addFavorite).toHaveBeenCalled())
    expect(opened).not.toHaveBeenCalled()
  })
})

describe('Saved (FEA-207, FEA-208)', () => {
  it('links every kind explicitly and never guesses for an unknown one', () => {
    const ref = (kind: string, extra: Partial<SavedRef> = {}) => ({ ...fav('PROJECT', 'x', 't'), kind, ...extra }) as SavedRef
    expect(savedLink('w', ref('DM'))).toBe('/w/w/c/x')
    expect(savedLink('w', ref('DOC'))).toBe('/w/w/docs?doc=x')
    expect(savedLink('w', ref('CHANNEL', { projectId: 'p' }))).toBe('/w/w/projects/p/chat/x')
    expect(savedLink('w', ref('VIEW', { viewTarget: 'TEAM', teamId: 'tm' }))).toBe('/w/w/teams/tm?v=x')
    expect(savedLink('w', ref('TEAM'))).toBe('/w/w/teams/x')
    expect(savedLink('w', ref('SOMETHING'))).toBeNull()
    expect(savedTitle(ref('DM', { title: '' }), 'Direct message')).toBe('Direct message')
  })

  it('reorders within a folder only, removes with Undo, and shows a failure with the saved order', async () => {
    api.getFavoriteFolders.mockResolvedValue([{ id: 'f1', name: 'Daily', position: 1024 }])
    api.getFavorites.mockResolvedValue([
      fav('PROJECT', 'p1', 'Alpha', { position: 1024 }),
      fav('DM', 'dm1', 'Ann', { position: 2048, folderId: 'f1' }),
      fav('DOC', 'd1', 'Handbook', { position: 3072, folderId: 'f1' }),
    ])
    api.orderFavorites.mockRejectedValueOnce(new Error('offline'))
    renderWithProviders(<><SavedPageRoute /><NoticeHost /></>, { route: '/w/w/saved', path: '/w/:workspaceId/saved' })
    const folder = await screen.findByRole('region', { name: 'Daily' })
    expect(within(folder).getByRole('link', { name: /Ann/ }).getAttribute('href')).toBe('/w/w/c/dm1')
    expect(within(folder).getByRole('link', { name: /Handbook/ }).getAttribute('href')).toBe('/w/w/docs?doc=d1')
    fireEvent.click(within(folder).getByRole('button', { name: 'Move Handbook up' }))
    await waitFor(() => expect(api.orderFavorites).toHaveBeenCalledWith('w', [{ kind: 'DOC', targetId: 'd1' }, { kind: 'DM', targetId: 'dm1' }], 'LISTED'))
    expect(await screen.findByText('Could not change the order. Showing the saved order again.')).toBeTruthy()
    api.removeFavorite.mockResolvedValue(undefined)
    fireEvent.click(screen.getByRole('button', { name: 'Remove Alpha from Favorites' }))
    await waitFor(() => expect(api.removeFavorite).toHaveBeenCalledWith('w', 'PROJECT', 'p1'))
    expect(await screen.findByRole('button', { name: 'Undo' })).toBeTruthy()
  })

  it('says when Favorites cannot be loaded and tries again', async () => {
    api.getFavorites.mockRejectedValueOnce(new Error('down'))
    renderWithProviders(<SavedPageRoute />, { route: '/w/w/saved', path: '/w/:workspaceId/saved' })
    expect(await screen.findByText('Could not load Favorites.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(await screen.findByText('No Favorites')).toBeTruthy()
  })
})

describe('Chat Favorites (FEA-201, FEA-202)', () => {
  it('lists starred channels and DMs in Saved order, opens them in Chat, and keeps them in their own groups', async () => {
    api.getSpace.mockResolvedValue({
      teams: [], projects: [{ id: 'p1', name: 'Alpha' }],
      channels: [
        { id: 'c1', conversationId: 'c1', name: 'general', unreadCount: 3, projectId: null, visibility: 'PUBLIC' },
        { id: 'c2', conversationId: 'c2', name: 'plan', unreadCount: 0, projectId: 'p1', visibility: 'PRIVATE' },
      ],
      dms: [{ id: 'dm1', conversationId: 'dm1', peers: [{ id: 'u2', displayName: 'Ann' }], unreadCount: 0, lastMessageSeq: 4 }],
    })
    api.getFavorites.mockResolvedValue([fav('DM', 'dm1', 'Ann'), fav('CHANNEL', 'c2', 'plan', { projectId: 'p1' }), fav('PROJECT', 'p1', 'Alpha')])
    renderWithProviders(<ContextSidebar workspaceId="w" section="chat" />, { route: '/w/w/chat', path: '/w/:workspaceId/chat' })
    const list = await waitFor(() => {
      const found = document.querySelector('.favorite-conversations')
      if (!found) throw new Error('no favorites yet')
      return found as HTMLElement
    })
    const links = within(list).getAllByRole('link')
    expect(links.map((link) => link.getAttribute('href'))).toEqual(['/w/w/c/dm1', '/w/w/c/c2'])
    // The channels stay in their own group as well.
    expect(screen.getAllByRole('link', { name: /plan/ })).toHaveLength(2)
    // The row menu offers removing it.
    fireEvent.click(within(list).getAllByRole('button', { name: 'Conversation menu' })[0]!)
    const remove = await screen.findByRole('menuitemcheckbox', { name: /Remove from Favorites/ })
    // Removing takes the row away: focus goes to the group's heading, not to nowhere.
    api.removeFavorite.mockResolvedValue(undefined)
    api.getFavorites.mockResolvedValue([fav('CHANNEL', 'c2', 'plan', { projectId: 'p1' }), fav('PROJECT', 'p1', 'Alpha')])
    fireEvent.click(remove)
    await waitFor(() => expect(within(list).getAllByRole('link')).toHaveLength(1))
    await waitFor(() => expect(document.activeElement?.textContent).toContain('Favorites'))
    expect(document.activeElement?.className).toContain('tree-fold')
  })

  it('drops a shortcut to a conversation that left the side list and asks the server again', async () => {
    api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [{ id: 'c1', conversationId: 'c1', name: 'general', unreadCount: 0, projectId: null, visibility: 'PUBLIC' }], dms: [] })
    api.getFavorites.mockResolvedValue([fav('CHANNEL', 'gone', 'secret plans'), fav('CHANNEL', 'c1', 'general')])
    renderWithProviders(<ContextSidebar workspaceId="w" section="chat" />, { route: '/w/w/chat', path: '/w/:workspaceId/chat' })
    const list = await waitFor(() => {
      const found = document.querySelector('.favorite-conversations')
      if (!found) throw new Error('no favorites yet')
      return found as HTMLElement
    })
    await waitFor(() => expect(within(list).getAllByRole('link').map((link) => link.getAttribute('href'))).toEqual(['/w/w/c/c1']))
    expect(screen.queryByText(/secret plans/)).toBeNull()
    await waitFor(() => expect(api.getFavorites.mock.calls.length).toBeGreaterThanOrEqual(2))
  })

  it('tells an empty Favorites from one that is still loading', async () => {
    renderWithProviders(<ContextSidebar workspaceId="w" section="chat" />, { route: '/w/w/chat', path: '/w/:workspaceId/chat' })
    expect(await screen.findByText('No favorite conversations.')).toBeTruthy()
  })
})

describe('Views and search (FEA-205, FEA-207)', () => {
  it('puts the starred saved Views first, in Favorites order, and keeps the rest in theirs', () => {
    const view = (id: string) => ({ id, name: id }) as never
    const ordered = favoriteViewsFirst([view('a'), view('b'), view('c'), view('d')], [fav('VIEW', 'c', 'c'), fav('PROJECT', 'a', 'a'), fav('VIEW', 'zz', 'zz'), fav('VIEW', 'b', 'b')])
    expect(ordered.map((item: { id: string }) => item.id)).toEqual(['c', 'b', 'a', 'd'])
  })

  it('offers a star only for results that are a kind Favorites knows, by their own id', () => {
    const result = (kind: SearchResult['kind'], fields: Record<string, unknown> = {}) => ({ kind, id: 'r1', title: 'x', subtitle: null, fields }) as unknown as SearchResult
    const inside = (id: string) => id === 'r1'
    expect(favoriteTarget(result('ticket'), inside)).toEqual({ kind: 'TICKET', targetId: 'r1' })
    expect(favoriteTarget(result('doc'), inside)).toEqual({ kind: 'DOC', targetId: 'r1' })
    expect(favoriteTarget(result('view'), inside)).toEqual({ kind: 'VIEW', targetId: 'r1' })
    expect(favoriteTarget(result('space', { spaceKind: 'CHANNEL' }), inside)).toEqual({ kind: 'CHANNEL', targetId: 'r1' })
    expect(favoriteTarget(result('space', { spaceKind: 'TEAM' }), inside)).toEqual({ kind: 'TEAM', targetId: 'r1' })
    expect(favoriteTarget(result('space', { spaceKind: 'PROJECT' }), inside)).toEqual({ kind: 'PROJECT', targetId: 'r1' })
    expect(favoriteTarget(result('space', { spaceKind: 'PROJECT' }), () => false)).toBeNull()
    expect(favoriteTarget(result('message'), inside)).toBeNull()
    expect(favoriteTarget(result('file'), inside)).toBeNull()
    expect(favoriteTarget(result('person'), inside)).toBeNull()
  })
})
