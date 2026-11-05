import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { Route, Routes, useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  getInbox: vi.fn(), inboxCommand: vi.fn(), inboxReadAll: vi.fn(),
  getSubscription: vi.fn(), watch: vi.fn(), unwatch: vi.fn(),
  search: vi.fn(), getSpace: vi.fn(), getProjectWorkflow: vi.fn(),
  getProjectBoard: vi.fn(), getProjectPermissions: vi.fn(), searchDirectory: vi.fn(), setProjectGrant: vi.fn(), removeProjectGrant: vi.fn(), linkProjectTeam: vi.fn(), unlinkProjectTeam: vi.fn(), setProjectTeamRole: vi.fn(), getProjectTeamMembers: vi.fn(), setDesignation: vi.fn(), getProjectPins: vi.fn(),
  getNotificationPreferences: vi.fn(), saveNotificationPreferences: vi.fn(), getMutedChannels: vi.fn(), muteChannel: vi.fn(),
  getFavorites: vi.fn(), getFavoriteFolders: vi.fn(), createFavoriteFolder: vi.fn(), deleteFavoriteFolder: vi.fn(), moveFavorite: vi.fn(), renameFavoriteFolder: vi.fn(),
  orderFavorites: vi.fn(), removeFavorite: vi.fn(), getLater: vi.fn(), setReminder: vi.fn(), cancelReminder: vi.fn(), updateLater: vi.fn(), removeLater: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { InboxPage, inboxHref } from './InboxPage'
import { WatchButton } from './WatchButton'
import { SearchPage } from '../search/SearchPage'
import { resultHref } from '../search/resultHref'
import { ProjectSettings } from './ProjectSettings'
import { NotificationSettings } from './NotificationSettings'
import { SavedPage } from '../personal/SavedPage'
import { reminderPresets } from './reminderTimes'
import { renderWithProviders } from '../../test/renderWithProviders'

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}

function mount(path: string, route: string, element: React.ReactNode) {
  return renderWithProviders(<Routes>
      <Route path={route} element={element} />
      <Route path="*" element={<Where />} />
      </Routes>, { route: path })
}

const item = (over: object = {}) => ({
  id: 'i1', kind: 'ticket.assigned', state: 'UNREAD', snoozedUntil: null, createdAt: '2026-10-22T07:00:00Z', version: 1,
  actor: { id: 'u2', name: 'Mina' }, subject: { kind: 'TICKET', id: 't1', title: 'PEN-1 Fix login' }, params: {},
  link: { type: 'ticket', ticketId: 't1', projectId: 'p1' }, ...over,
})

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getInbox.mockResolvedValue({ items: [], nextCursor: null, unreadCount: 0, unreadCapped: false })
  api.getFavorites.mockResolvedValue([])
  api.getFavoriteFolders.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
})
afterEach(cleanup)

describe('Inbox', () => {
  it('lists what happened, opens an item, and marks it read when it is opened', async () => {
    api.getInbox.mockResolvedValue({ items: [item(), item({ id: 'i2', kind: 'mention.user', state: 'READ', version: 3, subject: { kind: 'MESSAGE', id: 'm1', title: 'please look' }, link: { type: 'message', messageId: 'm1', conversationId: 'c1', threadRootId: null } })], nextCursor: null, unreadCount: 1, unreadCapped: false })
    api.inboxCommand.mockResolvedValue({ id: 'i1', state: 'READ', version: 2 })
    mount('/w/w/inbox', '/w/:workspaceId/inbox', <InboxPage />)
    expect(await screen.findByText('Mina assigned a Task to you')).toBeTruthy()
    expect(screen.getByText('PEN-1 Fix login')).toBeTruthy()
    fireEvent.click(screen.getByText('Mina assigned a Task to you'))
    await waitFor(() => expect(api.inboxCommand).toHaveBeenCalledWith('i1', 'read', { expectedVersion: 1, until: undefined }))
    await waitFor(() => expect(screen.getByTestId('where').textContent).toBe('/w/w/projects/p1/work?panel=ticket:t1'))
  })

  it('does not mark an already read item again', async () => {
    api.getInbox.mockResolvedValue({ items: [item({ state: 'READ' })], nextCursor: null, unreadCount: 0, unreadCapped: false })
    mount('/w/w/inbox', '/w/:workspaceId/inbox', <InboxPage />)
    fireEvent.click(await screen.findByText('Mina assigned a Task to you'))
    await waitFor(() => expect(screen.getByTestId('where').textContent).toContain('/projects/p1/work'))
    expect(api.inboxCommand).not.toHaveBeenCalled()
  })

  it('snoozes, archives and marks everything read', async () => {
    api.getInbox.mockResolvedValue({ items: [item()], nextCursor: null, unreadCount: 1, unreadCapped: false })
    api.inboxCommand.mockResolvedValue({ id: 'i1', state: 'UNREAD', version: 2 })
    api.inboxReadAll.mockResolvedValue({ changed: 1 })
    mount('/w/w/inbox', '/w/:workspaceId/inbox', <InboxPage />)
    await screen.findByText('Mina assigned a Task to you')
    fireEvent.click(screen.getByRole('button', { name: 'Archive' }))
    await waitFor(() => expect(api.inboxCommand).toHaveBeenCalledWith('i1', 'archive', { expectedVersion: 1, until: undefined }))
    fireEvent.click(screen.getByRole('button', { name: 'Snooze: In 1 hour' }))
    await waitFor(() => expect(api.inboxCommand).toHaveBeenCalledWith('i1', 'snooze', expect.objectContaining({ expectedVersion: 1, until: expect.any(String) })))
    fireEvent.click(screen.getByRole('button', { name: 'Mark all read' }))
    await waitFor(() => expect(api.inboxReadAll).toHaveBeenCalledWith('w', undefined))
  })

  it('says the Invitations filter is empty because no producer exists', async () => {
    mount('/w/w/inbox?box=invitations', '/w/:workspaceId/inbox', <InboxPage />)
    expect(await screen.findByText('This server does not create invitation notices yet.')).toBeTruthy()
    await waitFor(() => expect(api.getInbox).toHaveBeenCalledWith('w', expect.objectContaining({ filter: 'invitations' })))
  })

  it('says how late a reminder arrived', async () => {
    api.getInbox.mockResolvedValue({ items: [item({ kind: 'reminder.due', params: { lateSeconds: 600 } })], nextCursor: null, unreadCount: 1, unreadCapped: false })
    mount('/w/w/inbox?box=reminders', '/w/:workspaceId/inbox', <InboxPage />)
    expect(await screen.findByText(/Arrived 10 min late/)).toBeTruthy()
  })

  it('builds links without leaking anything that was not given', () => {
    expect(inboxHref('w', { type: 'message', messageId: 'm', conversationId: 'c', threadRootId: 'r' })).toBe('/w/w/c/c/t/r')
    expect(inboxHref('w', { type: 'message', messageId: 'm', conversationId: null, threadRootId: null })).toBeNull()
    expect(inboxHref('w', { type: 'project', projectId: 'p' })).toBe('/w/w/projects/p')
  })
})

describe('Watch button', () => {
  it('follows and stops following, and the label says what it will do', async () => {
    api.getSubscription.mockResolvedValue({ targetKind: 'TICKET', targetId: 't1', mode: 'NONE', watching: false, version: 0 })
    api.watch.mockResolvedValue({ watching: true })
    mount('/x', '/x', <WatchButton kind="TICKET" targetId="t1" />)
    const button = await screen.findByRole('button', { name: 'Get ordinary updates for this item' })
    await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(button)
    await waitFor(() => expect(api.watch).toHaveBeenCalledWith('TICKET', 't1'))
  })

  it('shows a followed item as pressed and unwatches it', async () => {
    api.getSubscription.mockResolvedValue({ targetKind: 'THREAD', targetId: 'r', mode: 'AUTO', watching: true, version: 1 })
    api.unwatch.mockResolvedValue({ watching: false })
    mount('/x', '/x', <WatchButton kind="THREAD" targetId="r" />)
    const button = await screen.findByRole('button', { name: 'Stop watching' })
    expect(button.getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(button)
    await waitFor(() => expect(api.unwatch).toHaveBeenCalledWith('THREAD', 'r'))
  })
})

const response = (results: object[] = [], extra: object = {}) => ({ query: '', normalized: {}, results, nextCursor: null, status: { ready: true, unavailableKinds: [], stale: false }, ...extra })

describe('Search', () => {
  it('asks for nothing until something is typed', async () => {
    mount('/w/w/search', '/w/:workspaceId/search', <SearchPage />)
    await screen.findAllByText(/You only see what you can open/)
    expect(api.search).not.toHaveBeenCalled()
  })

  it('shows highlighted words and where each result leads', async () => {
    api.search.mockResolvedValue(response([
      { kind: 'ticket', id: 't1', title: 'PEN-1 Fix login', subtitle: 'PEN-1 · Alpha', snippet: [{ text: 'login', hit: true }, { text: ' broken', hit: false }], createdAt: null, fields: { projectId: 'p1', key: 'PEN-1' } },
      { kind: 'message', id: 'm1', title: 'Mina', subtitle: 'general', snippet: [], createdAt: null, fields: { conversationId: 'c1', threadRootId: null } },
    ]))
    mount('/w/w/search?q=login', '/w/:workspaceId/search', <SearchPage />)
    const mark = await screen.findByText('login', { selector: 'mark' })
    expect(mark).toBeTruthy()
    expect(api.search).toHaveBeenCalledWith('w', expect.objectContaining({ q: 'login', kind: 'all' }))
    expect(screen.getByText('PEN-1 Fix login').closest('a')?.getAttribute('href')).toBe('/w/w/projects/p1/work?panel=ticket:t1')
    expect(screen.getByText('Mina').closest('a')?.getAttribute('href')).toBe('/w/w/c/c1')
  })

  it('turns a chip into the same text a person could type', async () => {
    api.search.mockResolvedValue(response())
    mount('/w/w/search?q=deploy', '/w/:workspaceId/search', <SearchPage />)
    await screen.findByText('No results.')
    fireEvent.click(screen.getByRole('button', { name: 'Assigned to me' }))
    await waitFor(() => expect(api.search).toHaveBeenCalledWith('w', expect.objectContaining({ q: 'deploy assignee:me' })))
  })

  it('explains an invalid status and points to the State chip', async () => {
    api.search.mockRejectedValue(new ApiError(400, 'invalid_status', 'x', { token: 'status:review' }))
    mount('/w/w/search?q=status:review', '/w/:workspaceId/search', <SearchPage />)
    expect(await screen.findByText(/status:review.*not-started, in-progress, done or canceled/)).toBeTruthy()
  })

  it('says search is unavailable without blaming the query, and offers a retry', async () => {
    api.search.mockRejectedValue(new ApiError(503, 'search_unavailable', 'x', { retryable: true }))
    mount('/w/w/search?q=anything', '/w/:workspaceId/search', <SearchPage />)
    expect(await screen.findByText(/Search is not available right now/)).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy()
  })

  it('lets a person settle an ambiguous name by choosing one', async () => {
    api.search.mockRejectedValueOnce(new ApiError(400, 'ambiguous_filter', 'x', { token: 'from:kim', candidates: [{ id: 'u1', name: 'Kim A' }, { id: 'u2', name: 'Kim B' }] }))
    api.search.mockResolvedValue(response())
    mount('/w/w/search?q=from:kim', '/w/:workspaceId/search', <SearchPage />)
    fireEvent.click(await screen.findByRole('button', { name: 'Kim B' }))
    await waitFor(() => expect(api.search).toHaveBeenLastCalledWith('w', expect.objectContaining({ q: 'from:u2' })))
  })

  it('tells which kinds are still being indexed', async () => {
    api.search.mockResolvedValue(response([], { status: { ready: false, unavailableKinds: ['ticket'], stale: false } }))
    mount('/w/w/search?q=x', '/w/:workspaceId/search', <SearchPage />)
    expect(await screen.findByText(/Ticket is still being indexed/)).toBeTruthy()
  })

  it('leads each kind of result to its place', () => {
    const base = { title: 't', subtitle: null, snippet: [], createdAt: null }
    expect(resultHref('w', { ...base, kind: 'space', id: 'c1', fields: { spaceKind: 'CHANNEL', projectId: 'p' } })).toBe('/w/w/projects/p/chat/c1')
    expect(resultHref('w', { ...base, kind: 'space', id: 'tm', fields: { spaceKind: 'TEAM' } })).toBe('/w/w/teams/tm')
    expect(resultHref('w', { ...base, kind: 'view', id: 'v', fields: { targetKind: 'WORKSPACE' } })).toBe('/w/w/my-work?v=v')
    expect(resultHref('w', { ...base, kind: 'file', id: 'f', fields: { conversationId: 'c', threadRootId: 'r' } })).toBe('/w/w/c/c/t/r')
  })
})

describe('Project permissions', () => {
  const permissions = {
    members: [
      { userId: 'a', displayName: 'Admin', role: 'ADMIN', source: 'DIRECT', canWaiveDependencies: false, canManagePins: false, designatable: false },
      { userId: 'e', displayName: 'Edie', role: 'EDITOR', source: 'DIRECT', canWaiveDependencies: false, canManagePins: true, designatable: true },
      { userId: 'v', displayName: 'Vic', role: 'VIEWER', source: 'DIRECT', canWaiveDependencies: false, canManagePins: false, designatable: false },
    ],
    viaGroups: [], teams: [{ teamId: 't', name: 'Builders', role: 'EDITOR', memberCount: 3 }, { teamId: 'q', name: 'Linked', role: null, memberCount: 2 }],
    capabilities: ['dependency_waiver', 'pin_management'],
    recentChanges: [{ at: '2026-10-22T07:00:00Z', actorName: 'Admin', userName: 'Edie', userId: 'e', before: { canWaiveDependencies: false, canManagePins: false }, after: { canWaiveDependencies: false, canManagePins: true } }],
  }

  it('lets an admin designate only a direct editor and shows the trail', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: true })
    api.getProjectPermissions.mockResolvedValue(permissions)
    api.setDesignation.mockResolvedValue({})
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    const waive = await screen.findByRole('checkbox', { name: 'Edie: Waive dependencies' })
    expect(screen.queryByRole('checkbox', { name: /Vic/ })).toBeNull()
    expect(screen.queryByRole('checkbox', { name: /Admin/ })).toBeNull()
    expect((screen.getByRole('checkbox', { name: 'Edie: Manage Pins' }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(waive)
    await waitFor(() => expect(api.setDesignation).toHaveBeenCalledWith('p', 'e', { canWaiveDependencies: true, canManagePins: true }))
    expect(screen.getByText(/Admin changed Edie’s abilities: none → Manage Pins/)).toBeTruthy()
    expect(screen.getByText(/Builders · 3 people/)).toBeTruthy()
    expect((screen.getByLabelText('Builders: Team role') as HTMLSelectElement).value).toBe('EDITOR')
    expect((screen.getByLabelText('Linked: Team role') as HTMLSelectElement).value).toBe('')
  })

  it('shows the choice at once, locks that row while it saves, then reloads from the server', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: true })
    api.getProjectPermissions.mockResolvedValue(permissions)
    api.getProjectPins.mockResolvedValue({ pins: [], canManage: true })
    let finish: (value: unknown) => void = () => undefined
    api.setDesignation.mockReturnValue(new Promise((resolve) => { finish = resolve }))
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    const waive = (await screen.findByRole('checkbox', { name: 'Edie: Waive dependencies' })) as HTMLInputElement
    fireEvent.click(waive)
    await waitFor(() => expect(waive.disabled).toBe(true))
    expect(waive.checked).toBe(true)
    const loads = api.getProjectPermissions.mock.calls.length
    finish({})
    await waitFor(() => expect(waive.disabled).toBe(false))
    expect(api.getProjectPermissions.mock.calls.length).toBeGreaterThan(loads)
    // The server still says no, so the reloaded row shows what it stores.
    expect(waive.checked).toBe(false)
  })

  it('manages direct roles, Team links and the role a Team gives, one separate action each', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: true })
    api.getProjectPermissions.mockResolvedValue(permissions)
    api.getSpace.mockResolvedValue({ teams: [{ id: 't', name: 'Builders' }, { id: 'q', name: 'Linked' }, { id: 'z', name: 'Free' }], projects: [], channels: [], dms: [] })
    api.searchDirectory.mockResolvedValue({ results: [{ type: 'person', person: { id: 'n', displayName: 'Nina', username: 'nina', email: 'n@x', organizationId: 'o', organizationName: 'O', affiliation: [] } }] })
    for (const call of [api.setProjectGrant, api.removeProjectGrant, api.linkProjectTeam, api.unlinkProjectTeam, api.setProjectTeamRole]) call.mockResolvedValue(undefined)
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    fireEvent.change(await screen.findByLabelText('Vic: Role'), { target: { value: 'EDITOR' } })
    await waitFor(() => expect(api.setProjectGrant).toHaveBeenCalledWith('p', 'v', 'EDITOR'))
    fireEvent.click(screen.getByRole('button', { name: 'Remove Vic' }))
    await waitFor(() => expect(api.removeProjectGrant).toHaveBeenCalledWith('p', 'v'))
    // A new person is given the chosen role.
    fireEvent.change(screen.getByLabelText('Role to give'), { target: { value: 'EDITOR' } })
    fireEvent.click(await screen.findByRole('button', { name: /Nina/ }))
    await waitFor(() => expect(api.setProjectGrant).toHaveBeenCalledWith('p', 'n', 'EDITOR'))
    // Linking a Team gives nothing; a role is a second step, and ending it keeps the link.
    fireEvent.change(screen.getByLabelText('Team to link'), { target: { value: 'z' } })
    fireEvent.click(screen.getByRole('button', { name: 'Link team' }))
    await waitFor(() => expect(api.linkProjectTeam).toHaveBeenCalledWith('p', 'z'))
    expect(api.setProjectTeamRole).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Linked: Team role'), { target: { value: 'VIEWER' } })
    await waitFor(() => expect(api.setProjectTeamRole).toHaveBeenCalledWith('p', 'q', 'VIEWER'))
    fireEvent.change(screen.getByLabelText('Builders: Team role'), { target: { value: '' } })
    await waitFor(() => expect(api.setProjectTeamRole).toHaveBeenCalledWith('p', 't', null))
    fireEvent.click(screen.getByRole('button', { name: 'Unlink Builders' }))
    await waitFor(() => expect(api.unlinkProjectTeam).toHaveBeenCalledWith('p', 't'))
  })

  it('invites a whole team as team access, or picked members as direct roles', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: true })
    api.getProjectPermissions.mockResolvedValue(permissions)
    api.getSpace.mockResolvedValue({ teams: [{ id: 't', name: 'Builders' }, { id: 'q', name: 'Linked' }, { id: 'z', name: 'Free' }], projects: [], channels: [], dms: [] })
    api.searchDirectory.mockResolvedValue({ results: [] })
    api.getProjectTeamMembers.mockResolvedValue([{ userId: 'm1', displayName: 'Mina' }, { userId: 'v', displayName: 'Vic' }])
    for (const call of [api.setProjectGrant, api.linkProjectTeam, api.setProjectTeamRole]) call.mockResolvedValue(undefined)
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    // The whole team: any team, linked first when needed, then a team role. No direct grant is made.
    fireEvent.change(await screen.findByLabelText('Team'), { target: { value: 'z' } })
    fireEvent.change(screen.getByLabelText('Invite role'), { target: { value: 'EDITOR' } })
    fireEvent.click(screen.getByRole('button', { name: 'Invite the whole team' }))
    await waitFor(() => expect(api.setProjectTeamRole).toHaveBeenCalledWith('p', 'z', 'EDITOR'))
    expect(api.linkProjectTeam).toHaveBeenCalledWith('p', 'z')
    expect(api.setProjectGrant).not.toHaveBeenCalled()
    // Picked members: only linked teams are offered, those who already have a direct role are not, and each pick is a direct grant.
    fireEvent.click(screen.getByLabelText('Pick people from a team'))
    const options = Array.from((screen.getByLabelText('Team') as HTMLSelectElement).options).map((option) => option.textContent)
    expect(options).toEqual(['—', 'Builders', 'Linked'])
    fireEvent.change(screen.getByLabelText('Team'), { target: { value: 't' } })
    expect(await screen.findByLabelText('Mina')).toBeTruthy()
    expect(screen.queryByLabelText('Vic')).toBeNull()
    fireEvent.click(screen.getByLabelText('Mina'))
    fireEvent.click(screen.getByRole('button', { name: 'Add the people picked' }))
    await waitFor(() => expect(api.setProjectGrant).toHaveBeenCalledWith('p', 'm1', 'EDITOR'))
    expect(api.setProjectTeamRole).toHaveBeenCalledTimes(1)
  })

  it('says so when a designation fails', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: true })
    api.getProjectPermissions.mockResolvedValue(permissions)
    api.getProjectPins.mockResolvedValue({ pins: [], canManage: true })
    api.setDesignation.mockRejectedValue(new ApiError(500, 'internal', 'boom'))
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Edie: Waive dependencies' }))
    expect(await screen.findByText('Could not change permissions')).toBeTruthy()
  })

  it('shows nothing of the settings to someone who is not an admin', async () => {
    api.getProjectBoard.mockResolvedValue({ canManageWorkflow: false })
    mount('/w/w/projects/p/settings', '/w/:workspaceId/projects/:projectId/settings', <ProjectSettings workspaceId="w" projectId="p" />)
    expect(await screen.findByText('Only a Project admin can see these settings.')).toBeTruthy()
    expect(api.getProjectPermissions).not.toHaveBeenCalled()
  })
})

describe('Notification settings', () => {
  it('saves the level with the version it was read at', async () => {
    api.getNotificationPreferences.mockResolvedValue({ level: 'ALL', dnd: { enabled: false, schedule: null, snoozeUntil: null }, external: {}, version: 4, dndActive: false })
    api.getMutedChannels.mockResolvedValue({ channelIds: [] })
    api.saveNotificationPreferences.mockResolvedValue({})
    mount('/x', '/x', <NotificationSettings />)
    fireEvent.click(await screen.findByLabelText('Only what is addressed to me'))
    fireEvent.click(screen.getByRole('button', { name: 'Save notification settings' }))
    await waitFor(() => expect(api.saveNotificationPreferences).toHaveBeenCalledWith(expect.objectContaining({ level: 'DIRECT_ONLY', expectedVersion: 4 })))
  })

  it('unmutes a channel', async () => {
    api.getNotificationPreferences.mockResolvedValue({ level: 'ALL', dnd: { enabled: false, schedule: null, snoozeUntil: null }, external: {}, version: 1, dndActive: false })
    api.getMutedChannels.mockResolvedValue({ channelIds: ['c1'] })
    api.muteChannel.mockResolvedValue(undefined)
    mount('/x', '/x', <NotificationSettings />)
    fireEvent.click(await screen.findByRole('button', { name: 'Unmute' }))
    await waitFor(() => expect(api.muteChannel).toHaveBeenCalledWith('c1', false))
  })
})

describe('Favorite folders and Later reminders', () => {
  const fav = (targetId: string, title: string, folderId: string | null = null) => ({ kind: 'PROJECT', targetId, available: true, title, subtitle: null, projectId: targetId, teamId: null, viewTarget: null, layout: null, position: 1, createdAt: '', folderId })

  it('shows a folder with its Favorites, moves one in, and deletes a folder without losing the Favorites', async () => {
    api.getFavorites.mockResolvedValue([fav('p1', 'Alpha', 'f1'), fav('p2', 'Beta')])
    api.getFavoriteFolders.mockResolvedValue([{ id: 'f1', name: 'Daily', position: 1 }])
    api.moveFavorite.mockResolvedValue(undefined)
    api.deleteFavoriteFolder.mockResolvedValue({ movedToTopLevel: 1 })
    mount('/w/w/saved', '/w/:workspaceId/saved', <SavedPage workspaceId="w" />)
    const folder = await screen.findByRole('region', { name: 'Daily' })
    expect(within(folder).getByText('Alpha')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Move Beta to a folder'), { target: { value: 'f1' } })
    await waitFor(() => expect(api.moveFavorite).toHaveBeenCalledWith('w', 'PROJECT', 'p2', 'f1'))
    fireEvent.click(within(folder).getByRole('button', { name: 'Delete folder' }))
    await waitFor(() => expect(api.deleteFavoriteFolder).toHaveBeenCalledWith('f1'))
    expect(await screen.findByText('Moved 1 to the top level')).toBeTruthy()
  })

  it('creates a folder and reports a duplicate name', async () => {
    api.createFavoriteFolder.mockRejectedValue(new ApiError(409, 'folder_name_taken', 'x'))
    mount('/w/w/saved', '/w/:workspaceId/saved', <SavedPage workspaceId="w" />)
    fireEvent.change(await screen.findByLabelText('Folder name'), { target: { value: 'Daily' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create folder' }))
    await waitFor(() => expect(api.createFavoriteFolder).toHaveBeenCalledWith('w', 'Daily'))
    expect(await screen.findByText('A folder with this name already exists')).toBeTruthy()
  })

  it('sets a reminder from a preset on a Later card', async () => {
    api.getLater.mockResolvedValue([{ id: 'l1', kind: 'MESSAGE', targetId: 'm1', available: true, title: 'ping me', subtitle: 'Mina · #general', projectId: null, teamId: null, viewTarget: null, layout: null, conversationId: 'c1', note: null, state: 'ACTIVE', version: 1, createdAt: '', updatedAt: '', reminder: null }])
    api.setReminder.mockResolvedValue({})
    mount('/w/w/saved', '/w/:workspaceId/saved', <SavedPage workspaceId="w" />)
    fireEvent.click(await screen.findByRole('tab', { name: 'Later' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Remind me' }))
    fireEvent.click(await screen.findByRole('button', { name: 'In 10 minutes' }))
    await waitFor(() => expect(api.setReminder).toHaveBeenCalledWith('l1', expect.objectContaining({ dueAt: expect.any(String), timeZone: expect.any(String) })))
  })
})

describe('Reminder times', () => {
  it('offers only times that are still ahead, in the person’s own clock', () => {
    const noon = new Date(2026, 9, 21, 12, 0, 0)
    const ids = reminderPresets(noon).map((p) => p.id)
    expect(ids).toEqual(['in10', 'later', 'tomorrow', 'nextWeek'])
    const lateEvening = new Date(2026, 9, 21, 19, 0, 0)
    expect(reminderPresets(lateEvening).map((p) => p.id)).toEqual(['in10', 'tomorrow', 'nextWeek'])
    const monday = reminderPresets(noon).find((p) => p.id === 'nextWeek')!.at
    expect(monday.getDay()).toBe(1)
    expect(monday.getTime()).toBeGreaterThan(noon.getTime())
  })
})
