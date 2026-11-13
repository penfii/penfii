import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  me: vi.fn(), listWorkspaces: vi.fn(), getSpace: vi.fn(),
  listMembers: vi.fn(), changeMemberRole: vi.fn(), removeMember: vi.fn(), setAccountDisabled: vi.fn(),
  listInvitations: vi.fn(), revokeInvitation: vi.fn(),
  listTeamMembers: vi.fn(), addTeamMember: vi.fn(), removeTeamMember: vi.fn(), searchDirectory: vi.fn(),
  listAudit: vi.fn(), serverInfo: vi.fn(), mcpPolicy: vi.fn(),
  getOffboardingImpact: vi.fn(), listProjectsWithoutAdmin: vi.fn(), recoverProjectAdmin: vi.fn(),
  listChannelsWithoutManager: vi.fn(), recoverChannelManager: vi.fn(),
  listProjectTrash: vi.fn(), restoreTicketFromTrash: vi.fn(), purgeTicket: vi.fn(), trashTicket: vi.fn(),
  reauthPassword: vi.fn(), reauthProviderStart: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))
vi.mock('../../shell/WorkspaceShell', () => ({
  OpenCreate: ({ label }: { label: string }) => <button type="button">{label}</button>,
}))

import { NoticeHost, dismissNotice } from '../../design/notices'
import type { AdminSection } from './adminSections'
import { WorkspaceAdmin } from './WorkspaceAdmin'
import { TrashSection } from './TrashSection'
import { TrashTicketButton } from './TrashTicketButton'
import { renderWithProviders } from '../../test/renderWithProviders'

const W = 'w1'

function member(over: object = {}) {
  return {
    userId: 'u2', displayName: 'Mina', email: 'mina@example.com', role: 'MEMBER', state: 'ACTIVE',
    joinedAt: '2026-10-20T00:00:00Z', deactivatedAt: null, accountDisabled: false, teams: [{ id: 't1', name: 'Core' }],
    ...over,
  }
}

const people = [
  member({ userId: 'u1', displayName: 'Olivia', email: 'olivia@example.com', role: 'OWNER', teams: [] }),
  member({ userId: 'u3', displayName: 'Adam', email: 'adam@example.com', role: 'ADMIN', teams: [] }),
  member(),
  member({ userId: 'u4', displayName: 'Gone', email: 'gone@example.com', state: 'INACTIVE', deactivatedAt: '2026-10-25T00:00:00Z', teams: [] }),
]

function mount(role: 'OWNER' | 'ADMIN' | 'MEMBER', meId = 'u1', element?: React.ReactNode, section: AdminSection = 'members') {
  api.listWorkspaces.mockResolvedValue([{ id: W, name: 'W', timeZone: 'UTC', role, createdAt: '2026-10-01T00:00:00Z' }])
  api.me.mockResolvedValue({ id: meId, email: 'me@example.com', displayName: 'Me' })
  return renderWithProviders(element ?? <><WorkspaceAdmin workspaceId={W} section={section} /><NoticeHost /></>, { route: '/' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  for (const fn of Object.values(api)) fn.mockReset()
  api.listMembers.mockResolvedValue(people)
  api.listInvitations.mockResolvedValue([])
  api.getSpace.mockResolvedValue({ teams: [{ id: 't1', name: 'Core' }], projects: [], channels: [] })
  api.listTeamMembers.mockResolvedValue([])
  api.listAudit.mockResolvedValue({ items: [], nextCursor: null })
  api.getOffboardingImpact.mockResolvedValue({ projects: [], channels: [] })
  api.listProjectsWithoutAdmin.mockResolvedValue([])
  api.listChannelsWithoutManager.mockResolvedValue([])
  api.recoverProjectAdmin.mockResolvedValue({ newAccess: true })
  api.removeMember.mockResolvedValue(undefined)
  api.changeMemberRole.mockResolvedValue(undefined)
  api.setAccountDisabled.mockResolvedValue(undefined)
})
afterEach(() => {
  act(() => dismissNotice())
  cleanup()
  vi.restoreAllMocks()
})

/** Answers the confirmation dialog the way a person does, by pressing one of its buttons. */
async function answer(label: string) {
  fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: label }))
}

/** Opens a person's actions menu and chooses one entry, as a person does. */
async function choose(name: string, entry: RegExp | string) {
  fireEvent.click(screen.getByRole('button', { name: new RegExp(`^Actions for ${name}`) }))
  fireEvent.click(await screen.findByRole('menuitem', { name: entry }))
}

function row(name: string): HTMLElement {
  const found = screen.getAllByText(name).map((node) => node.closest('tr')).find((tr) => tr !== null)
  return found as HTMLElement
}

describe('workspace administration', () => {
  it('tells a plain member there is no access and does not call the admin API', async () => {
    mount('MEMBER')
    expect(await screen.findByText('No access')).toBeTruthy()
    expect(screen.queryByText('Manage W')).toBeNull()
    expect(api.listMembers).not.toHaveBeenCalled()
    expect(api.listAudit).not.toHaveBeenCalled()
  })

  it('names the workspace being managed', async () => {
    mount('OWNER', 'u1', undefined, 'general')
    expect(await screen.findByRole('heading', { name: 'Manage W' })).toBeTruthy()
  })

  it('confirms every role change, with more words when ownership changes hands, and says the result stays', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Mina', 'Make Owner')
    expect(within(await screen.findByRole('dialog')).getByText(/Change the role of Mina from Member to Owner\?.*Owners hold every administrative right/)).toBeTruthy()
    await answer('Cancel')
    expect(api.changeMemberRole).not.toHaveBeenCalled()
    await choose('Mina', 'Make Owner')
    await answer('Confirm')
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenCalledWith(W, 'u2', 'OWNER'))
    // The result stays in the person's row; the notice offers Undo, which changes the role back after asking again.
    expect(await screen.findByText('Mina is now Owner')).toBeTruthy()
    expect(within(row('Mina')).getByText('Saved')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
    expect(within(await screen.findByRole('dialog')).getByText(/from Owner to Member/)).toBeTruthy()
    await answer('Confirm')
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenLastCalledWith(W, 'u2', 'MEMBER'))
  })

  it('says the change was not applied when a role change fails, and keeps that on screen', async () => {
    api.changeMemberRole.mockRejectedValueOnce(new Error('boom'))
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Mina', 'Make Admin')
    await answer('Confirm')
    expect(await screen.findByText('The change for Mina was not applied')).toBeTruthy()
    expect(screen.queryByText('Mina is now Admin')).toBeNull()
    // The row says so too and offers Retry, which asks again and sends the same change.
    expect(within(row('Mina')).getByText(/Could not save/)).toBeTruthy()
    fireEvent.click(within(row('Mina')).getByRole('button', { name: 'Retry' }))
    await answer('Confirm')
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenCalledTimes(2))
    expect(await screen.findByText('Mina is now Admin')).toBeTruthy()
  })

  it('lets an owner change a role, remove a member and disable an account, but not touch themself', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    expect(within(row('Olivia')).getByText('Owner')).toBeTruthy()
    expect(within(row('Olivia')).queryByRole('button')).toBeNull()
    // A removed member is listed as removed and cannot be removed again.
    expect(within(row('Gone')).getByText('Removed')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /^Actions for Gone/ }))
    expect(screen.queryByRole('menuitem', { name: 'Remove' })).toBeNull()
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' })

    await choose('Mina', 'Make Admin')
    await answer('Confirm')
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenCalledWith(W, 'u2', 'ADMIN'))

    await choose('Mina', 'Remove')
    await answer('Remove')
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledWith(W, 'u2'))
    expect(await screen.findByText('Mina was removed')).toBeTruthy()

    await choose('Mina', 'Disable account')
    await answer('Disable account')
    await waitFor(() => expect(api.setAccountDisabled).toHaveBeenCalledWith(W, 'u2', true))
  })

  it('does not call the API when the confirmation is declined', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Mina', 'Remove')
    await answer('Cancel')
    expect(api.removeMember).not.toHaveBeenCalled()
  })

  it('warns which projects lose their last admin before a removal or a disabling, and still allows it', async () => {
    api.getOffboardingImpact.mockResolvedValue({
      projects: [
        { projectId: 'p1', name: 'Alpha', kind: 'NO_ADMIN' },
        { projectId: 'p2', name: 'Beta', kind: 'NO_DIRECT_ADMIN' },
      ],
      channels: [{ channelId: 'c1', name: 'leads', projectId: null }],
    })
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Mina', 'Remove')
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText(/This leaves these projects without an admin: Alpha \(no admin at all\); Beta \(no direct admin; a team or group admin stays\)/)).toBeTruthy()
    expect(within(dialog).getByText(/This leaves these private channels without a manager: leads\./)).toBeTruthy()
    expect(api.getOffboardingImpact).toHaveBeenCalledWith(W, 'u2')
    await answer('Remove')
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledWith(W, 'u2'))

    await choose('Mina', 'Disable account')
    expect(within(await screen.findByRole('dialog')).getByText(/Alpha \(no admin at all\)/)).toBeTruthy()
    await answer('Disable account')
    await waitFor(() => expect(api.setAccountDisabled).toHaveBeenCalledWith(W, 'u2', true))
  })

  it('does not hold the removal back when the warning cannot be loaded', async () => {
    api.getOffboardingImpact.mockRejectedValue(new Error('down'))
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Mina', 'Remove')
    expect(within(await screen.findByRole('dialog')).queryByText(/without an admin/)).toBeNull()
    await answer('Remove')
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledWith(W, 'u2'))
  })

  it('shows no section while every project has an admin', async () => {
    mount('OWNER', 'u1', undefined, 'recovery')
    await waitFor(() => expect(api.listProjectsWithoutAdmin).toHaveBeenCalled())
    await screen.findByText(/appear here/)
    expect(screen.queryByText('Projects without an admin')).toBeNull()
  })

  it('names an admin for a project without one, after saying that the person gets its content', async () => {
    api.listProjectsWithoutAdmin.mockResolvedValue([
      { id: 'p1', name: 'Alpha', ticketPrefix: 'ALP', formerAdmins: [{ userId: 'u9', displayName: 'Lena' }] },
    ])
    mount('OWNER', 'u1', undefined, 'recovery')
    expect(await screen.findByText('Projects without an admin')).toBeTruthy()
    expect(screen.getByText('Former admins: Lena')).toBeTruthy()
    const pick = screen.getByLabelText('Person to name admin: Alpha') as HTMLSelectElement
    // Only people who can act are offered: not the removed member, not a disabled account.
    api.listMembers.mockResolvedValue(people)
    const labels = Array.from(pick.options).map((option) => option.textContent)
    expect(labels).toContain('Mina (mina@example.com)')
    expect(labels).not.toContain('Gone (gone@example.com)')

    const assign = screen.getByRole('button', { name: 'Name admin: Alpha' }) as HTMLButtonElement
    expect(assign.disabled).toBe(true)
    fireEvent.change(pick, { target: { value: 'u2' } })
    fireEvent.click(assign)
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText(/Name Mina admin of the project "Alpha"\? They will be able to access its content/)).toBeTruthy()
    await answer('Name admin')
    await waitFor(() => expect(api.recoverProjectAdmin).toHaveBeenCalledWith('p1', 'u2'))
    expect(await screen.findByText('The admin was named. This person could not access the project before.')).toBeTruthy()
  })

  it('says so when naming yourself, and when somebody else got there first', async () => {
    api.listProjectsWithoutAdmin.mockResolvedValue([
      { id: 'p1', name: 'Alpha', ticketPrefix: 'ALP', formerAdmins: [] },
    ])
    api.recoverProjectAdmin.mockRejectedValueOnce(new ApiError(409, 'project_has_admin', 'x'))
    mount('OWNER', 'u1', undefined, 'recovery')
    await screen.findByText('Projects without an admin')
    expect(screen.getByText('No former admin is on record.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Person to name admin: Alpha'), { target: { value: 'u1' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name admin: Alpha' }))
    expect(within(await screen.findByRole('dialog')).getByText(/Name yourself admin of the project "Alpha"\? You will be able to access its content/)).toBeTruthy()
    await answer('Name admin')
    expect(await screen.findByText('This project already has an active admin, so it cannot be recovered this way.')).toBeTruthy()
  })

  it('lists the private channels a removal left without a manager right away, on the recovery page', async () => {
    // Members and Recovery are separate pages; here both are open so the refresh after a removal is visible.
    mount('OWNER', 'u1', <><WorkspaceAdmin workspaceId={W} section="members" /><WorkspaceAdmin workspaceId={W} section="recovery" /></>)
    await screen.findByText('Mina')
    expect(screen.queryByText('Private channels without a manager')).toBeNull()
    api.listChannelsWithoutManager.mockResolvedValue([
      { id: 'c1', name: 'leads', projectId: null, projectName: null, memberCount: 1, formerManagers: [], createdAt: '2026-11-01T00:00:00Z' },
    ])
    await choose('Mina', 'Remove')
    await answer('Remove')
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledWith(W, 'u2'))
    expect(await screen.findByText('Private channels without a manager')).toBeTruthy()
  })

  it('shows no channel section while every private channel has a manager', async () => {
    mount('OWNER', 'u1', undefined, 'recovery')
    await waitFor(() => expect(api.listChannelsWithoutManager).toHaveBeenCalled())
    await screen.findByText(/appear here/)
    expect(screen.queryByText('Private channels without a manager')).toBeNull()
  })

  it('names a manager for a private channel without one, after saying that the person can read it', async () => {
    api.listChannelsWithoutManager.mockResolvedValue([
      { id: 'c1', name: 'leads', projectId: null, projectName: null, memberCount: 3, formerManagers: [{ userId: 'u9', displayName: 'Lena' }], createdAt: '2026-11-01T00:00:00Z' },
      { id: 'c2', name: 'alpha-leads', projectId: 'p1', projectName: 'Alpha', memberCount: 1, formerManagers: [], createdAt: '2026-11-02T00:00:00Z' },
    ])
    api.recoverChannelManager.mockResolvedValue({ newAccess: true })
    mount('OWNER', 'u1', undefined, 'recovery')
    expect(await screen.findByText('Private channels without a manager')).toBeTruthy()
    expect(screen.getByText('Former managers: Lena')).toBeTruthy()
    expect(screen.getByText('Workspace-wide channel · 3 members')).toBeTruthy()
    expect(screen.getByText('Channel of the project Alpha · 1 members')).toBeTruthy()
    const pick = screen.getByLabelText('Person to name manager: leads') as HTMLSelectElement
    const labels = Array.from(pick.options).map((option) => option.textContent)
    expect(labels).toContain('Mina (mina@example.com)')
    expect(labels).not.toContain('Gone (gone@example.com)')
    const assign = screen.getByRole('button', { name: 'Name manager: leads' }) as HTMLButtonElement
    expect(assign.disabled).toBe(true)
    fireEvent.change(pick, { target: { value: 'u2' } })
    fireEvent.click(assign)
    expect(within(await screen.findByRole('dialog')).getByText(/Name Mina manager of the private channel "leads"\? They will be able to read its messages/)).toBeTruthy()
    await answer('Name manager')
    await waitFor(() => expect(api.recoverChannelManager).toHaveBeenCalledWith('c1', 'u2'))
    expect(await screen.findByText('The manager was named. This person could not access the channel before.')).toBeTruthy()
  })

  it('says so when naming yourself a channel manager, and when somebody else got there first', async () => {
    api.listChannelsWithoutManager.mockResolvedValue([
      { id: 'c1', name: 'leads', projectId: null, projectName: null, memberCount: 0, formerManagers: [], createdAt: '2026-11-01T00:00:00Z' },
    ])
    api.recoverChannelManager.mockRejectedValueOnce(new ApiError(409, 'channel_has_manager', 'x'))
    mount('OWNER', 'u1', undefined, 'recovery')
    await screen.findByText('Private channels without a manager')
    expect(screen.getByText('No former manager is on record.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Person to name manager: leads'), { target: { value: 'u1' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name manager: leads' }))
    expect(within(await screen.findByRole('dialog')).getByText(/Name yourself manager of the private channel "leads"\? You will be able to read its messages/)).toBeTruthy()
    await answer('Name manager')
    expect(await screen.findByText('This channel already has an active manager or is public, so it cannot be recovered this way.')).toBeTruthy()
  })

  it('says why a person outside the channel\'s Project cannot be named, and keeps the row', async () => {
    api.listChannelsWithoutManager.mockResolvedValue([
      { id: 'c2', name: 'alpha-leads', projectId: 'p1', projectName: 'Alpha', memberCount: 0, formerManagers: [], createdAt: '2026-11-02T00:00:00Z' },
    ])
    api.recoverChannelManager.mockRejectedValueOnce(new ApiError(422, 'not_in_project', 'x'))
    mount('OWNER', 'u1', undefined, 'recovery')
    await screen.findByText('Private channels without a manager')
    fireEvent.change(screen.getByLabelText('Person to name manager: alpha-leads'), { target: { value: 'u2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name manager: alpha-leads' }))
    await answer('Name manager')
    expect(await screen.findByText(/does not take part in this channel's Project/)).toBeTruthy()
    expect(screen.getByLabelText('Person to name manager: alpha-leads')).toBeTruthy()
  })

  it('reads the lists again after a 409 and keeps the outcome when the row goes away', async () => {
    const project = { id: 'p1', name: 'Alpha', ticketPrefix: 'ALP', formerAdmins: [] }
    const channel = { id: 'c1', name: 'leads', projectId: null, projectName: null, memberCount: 1, formerManagers: [], createdAt: '2026-11-01T00:00:00Z' }
    // Somebody else named an admin and a manager first: the next reading has neither row.
    api.listProjectsWithoutAdmin.mockResolvedValueOnce([project]).mockResolvedValue([])
    api.listChannelsWithoutManager.mockResolvedValueOnce([channel]).mockResolvedValue([])
    api.recoverProjectAdmin.mockRejectedValueOnce(new ApiError(409, 'project_has_admin', 'x'))
    api.recoverChannelManager.mockRejectedValueOnce(new ApiError(409, 'channel_has_manager', 'x'))
    mount('OWNER', 'u1', undefined, 'recovery')
    await screen.findByText('Projects without an admin')
    fireEvent.change(screen.getByLabelText('Person to name admin: Alpha'), { target: { value: 'u2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name admin: Alpha' }))
    await answer('Name admin')
    // The stale row is gone and the reason stays on the section.
    await waitFor(() => expect(screen.queryByLabelText('Person to name admin: Alpha')).toBeNull())
    expect(screen.getByText('This project already has an active admin, so it cannot be recovered this way.')).toBeTruthy()
    expect(api.listProjectsWithoutAdmin.mock.calls.length).toBeGreaterThan(1)

    fireEvent.change(screen.getByLabelText('Person to name manager: leads'), { target: { value: 'u2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name manager: leads' }))
    await answer('Name manager')
    await waitFor(() => expect(screen.queryByLabelText('Person to name manager: leads')).toBeNull())
    expect(screen.getByText('This channel already has an active manager or is public, so it cannot be recovered this way.')).toBeTruthy()
    expect(api.listChannelsWithoutManager.mock.calls.length).toBeGreaterThan(1)
  })

  it('keeps the success message after the named row leaves the list, with and without new access', async () => {
    const project = { id: 'p1', name: 'Alpha', ticketPrefix: 'ALP', formerAdmins: [] }
    const channel = { id: 'c1', name: 'leads', projectId: null, projectName: null, memberCount: 1, formerManagers: [], createdAt: '2026-11-01T00:00:00Z' }
    api.listProjectsWithoutAdmin.mockResolvedValueOnce([project]).mockResolvedValue([])
    api.listChannelsWithoutManager.mockResolvedValueOnce([channel]).mockResolvedValue([])
    api.recoverProjectAdmin.mockResolvedValue({ newAccess: false })
    api.recoverChannelManager.mockResolvedValue({ newAccess: true })
    mount('OWNER', 'u1', undefined, 'recovery')
    await screen.findByText('Projects without an admin')
    fireEvent.change(screen.getByLabelText('Person to name admin: Alpha'), { target: { value: 'u2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name admin: Alpha' }))
    await answer('Name admin')
    await waitFor(() => expect(screen.queryByLabelText('Person to name admin: Alpha')).toBeNull())
    expect(screen.getByText('The admin was named.')).toBeTruthy()

    fireEvent.change(screen.getByLabelText('Person to name manager: leads'), { target: { value: 'u2' } })
    fireEvent.click(screen.getByRole('button', { name: 'Name manager: leads' }))
    await answer('Name manager')
    await waitFor(() => expect(screen.queryByLabelText('Person to name manager: leads')).toBeNull())
    expect(screen.getByText('The manager was named. This person could not access the channel before.')).toBeTruthy()
  })

  it('gives an admin only the controls an admin may use', async () => {
    mount('ADMIN', 'u3')
    await screen.findByText('Mina')
    // An admin may remove a plain member and nothing else: no role change, no account disabling.
    fireEvent.click(screen.getByRole('button', { name: /^Actions for Mina/ }))
    expect(screen.getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['Remove'])
    // Not the owner, not another admin, not themself.
    expect(within(row('Olivia')).queryByRole('button')).toBeNull()
    expect(within(row('Adam')).queryByRole('button')).toBeNull()
  })

  it('says why the last owner stays, and asks for a recent sign-in when the server does', async () => {
    api.changeMemberRole.mockRejectedValueOnce(new ApiError(409, 'last_owner', 'x'))
    mount('OWNER')
    await screen.findByText('Mina')
    await choose('Adam', 'Make Member')
    await answer('Confirm')
    expect(await screen.findByText('A workspace needs at least one owner.')).toBeTruthy()

    api.removeMember
      .mockRejectedValueOnce(new ApiError(403, 'reauth_required', 'x', { methods: ['password'] }))
      .mockResolvedValueOnce(undefined)
    api.reauthPassword.mockResolvedValue(undefined)
    await choose('Mina', 'Remove')
    await answer('Remove')
    fireEvent.change(await screen.findByLabelText('Password'), { target: { value: 'secret' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledTimes(2))
  })

  it('lists invitations by status and offers revoke only where it applies', async () => {
    const invite = (over: object) => ({
      id: 'i1', email: 'a@example.com', role: 'MEMBER', status: 'PENDING', expiresAt: '2026-11-01T00:00:00Z',
      createdAt: '2026-10-20T00:00:00Z', createdBy: 'u1', createdByName: 'Olivia', acceptedAt: null, revokedAt: null, ...over,
    })
    api.listInvitations.mockResolvedValue([
      invite({}),
      invite({ id: 'i2', email: 'admin-invite@example.com', role: 'ADMIN' }),
      invite({ id: 'i3', email: 'done@example.com', status: 'ACCEPTED' }),
      invite({ id: 'i4', email: 'late@example.com', status: 'EXPIRED' }),
      invite({ id: 'i5', email: 'no@example.com', status: 'REVOKED' }),
    ])
    api.revokeInvitation.mockResolvedValue(undefined)
    mount('ADMIN', 'u3', undefined, 'invitations')
    await screen.findByText('a@example.com')
    for (const label of ['Pending', 'Accepted', 'Expired', 'Revoked']) expect(screen.getAllByText(label).length).toBeGreaterThan(0)
    // An admin may not revoke an invitation for an admin; finished ones cannot be revoked.
    expect(screen.queryByRole('button', { name: 'Revoke admin-invite@example.com' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Revoke done@example.com' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Revoke late@example.com' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Revoke a@example.com' }))
    await answer('Revoke')
    await waitFor(() => expect(api.revokeInvitation).toHaveBeenCalledWith(W, 'i1'))
    cleanup()
    api.listInvitations.mockResolvedValue([invite({ id: 'i2', email: 'admin-invite@example.com', role: 'ADMIN' })])
    mount('OWNER', 'u1', undefined, 'invitations')
    expect(await screen.findByRole('button', { name: 'Revoke admin-invite@example.com' })).toBeTruthy()
  })

  it('adds Team members with a role, changes a role and removes a member', async () => {
    api.listTeamMembers.mockResolvedValue([{ userId: 'u2', displayName: 'Mina', email: 'mina@example.com', role: 'MEMBER' }])
    api.addTeamMember.mockResolvedValue(undefined)
    api.removeTeamMember.mockResolvedValue(undefined)
    // People already in the Team are not offered again.
    api.searchDirectory.mockResolvedValue({
      results: ['u1', 'u2'].map((id) => ({ type: 'person', person: { id, displayName: id === 'u1' ? 'Olivia' : 'Mina', username: null, email: '', organizationId: 'o', organizationName: 'O', affiliation: [] } })),
    })
    mount('OWNER', 'u1', undefined, 'teams')
    await waitFor(() => expect(api.listTeamMembers).toHaveBeenCalledWith(W, 't1'))
    const teamCard = (await screen.findByRole('heading', { name: 'Team members' })).closest('div') as HTMLElement
    expect(await within(teamCard).findByRole('button', { name: /Olivia/ })).toBeTruthy()
    expect(within(teamCard).queryByRole('button', { name: 'Mina' })).toBeNull()
    fireEvent.click(within(teamCard).getByRole('button', { name: /Olivia/ }))
    await waitFor(() => expect(api.addTeamMember).toHaveBeenCalledWith(W, 't1', 'u1', 'MEMBER'))
    fireEvent.change(within(teamCard).getByLabelText('Mina: Role'), { target: { value: 'MANAGER' } })
    await waitFor(() => expect(api.addTeamMember).toHaveBeenCalledWith(W, 't1', 'u2', 'MANAGER'))
    fireEvent.click(within(teamCard).getByRole('button', { name: 'Remove Mina' }))
    await waitFor(() => expect(api.removeTeamMember).toHaveBeenCalledWith(W, 't1', 'u2'))
  })

  it('says why the last team manager cannot be demoted or removed', async () => {
    api.listTeamMembers.mockResolvedValue([{ userId: 'u2', displayName: 'Mina', email: 'mina@example.com', role: 'MANAGER' }])
    api.addTeamMember.mockRejectedValue(new ApiError(409, 'last_team_manager', 'a Team keeps at least one manager'))
    api.removeTeamMember.mockRejectedValue(new ApiError(409, 'last_team_manager', 'a Team keeps at least one manager'))
    api.searchDirectory.mockResolvedValue({ results: [] })
    mount('OWNER', 'u1', undefined, 'teams')
    const teamCard = (await screen.findByRole('heading', { name: 'Team members' })).closest('div') as HTMLElement
    fireEvent.change(await within(teamCard).findByLabelText('Mina: Role'), { target: { value: 'MEMBER' } })
    expect(await within(teamCard).findByText(/keeps at least one manager/)).toBeTruthy()
    // The list still shows the server's role, not the one that was refused.
    expect((within(teamCard).getByLabelText('Mina: Role') as HTMLSelectElement).value).toBe('MANAGER')
    fireEvent.click(within(teamCard).getByRole('button', { name: 'Remove Mina' }))
    await waitFor(() => expect(api.removeTeamMember).toHaveBeenCalledWith(W, 't1', 'u2'))
    expect(await within(teamCard).findByText(/keeps at least one manager/)).toBeTruthy()
  })

  it('shows the audit log with readable actions, filters by kind and loads more', async () => {
    const entry = (id: string, action: string) => ({
      id, occurredAt: '2026-10-28T01:00:00Z', actorUserId: 'u1', actorName: 'Olivia', action, outcome: 'OK',
      targetKind: 'USER', targetId: 'u2', targetLabel: 'Mina', subjectLabel: null, data: {},
    })
    api.listAudit.mockImplementation(async (_w: string, query: { action?: string; cursor?: string }) => {
      if (query.cursor) return { items: [entry('e3', 'something.new')], nextCursor: null }
      if (query.action === 'ticket.') return { items: [entry('e9', 'ticket.purged')], nextCursor: null }
      return { items: [entry('e1', 'member.removed'), entry('e2', 'user.disabled')], nextCursor: 'c1' }
    })
    mount('OWNER', 'u1', undefined, 'audit')
    expect(await screen.findByText('Member removed')).toBeTruthy()
    expect(screen.getByText('Account disabled')).toBeTruthy()
    // Who it was about is shown next to the action.
    expect(screen.getAllByText('Mina').length).toBeGreaterThan(0)
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }))
    // An action this client does not know yet is shown as its code, not hidden.
    expect(await screen.findByText('something.new')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'ticket.' } })
    expect(await screen.findByText('Ticket purged')).toBeTruthy()
    await waitFor(() => expect(api.listAudit).toHaveBeenCalledWith(W, expect.objectContaining({ action: 'ticket.' })))
  })
})

describe('integrations page', () => {
  it('says so when the server offers no MCP, instead of staying blank', async () => {
    api.serverInfo.mockResolvedValue({ features: [] })
    mount('OWNER', 'u1', undefined, 'integrations')
    expect(await screen.findByText('This server does not offer MCP, so there is nothing to set here.')).toBeTruthy()
  })

  it('shows the MCP policy there and nowhere else when the server offers MCP', async () => {
    api.serverInfo.mockResolvedValue({ features: ['mcp'] })
    api.mcpPolicy.mockResolvedValue({ mcpEnabled: true, allowWrite: false, allowDestructive: false, allowAdministrative: false })
    mount('OWNER', 'u1', undefined, 'integrations')
    await waitFor(() => expect(api.mcpPolicy).toHaveBeenCalled())
    expect(screen.queryByText(/does not offer MCP/)).toBeNull()
    cleanup()
    mount('OWNER', 'u1', undefined, 'members')
    await screen.findByText('Mina')
    expect(api.mcpPolicy).toHaveBeenCalledTimes(1)
  })
})

describe('administration pages and the member list', () => {
  const many = Array.from({ length: 60 }, (_, i) => member({ userId: `m${i}`, displayName: `Person ${String(i).padStart(2, '0')}`, email: `p${i}@example.com`, teams: i === 40 ? [{ id: 't1', name: 'Core' }, { id: 't2', name: 'Ops' }, { id: 't3', name: 'Legal' }, { id: 't4', name: 'Sales' }] : [] }))

  it('shows one page per section, and a strip that reaches the others', async () => {
    mount('OWNER', 'u1', undefined, 'invitations')
    expect(await screen.findByRole('heading', { name: 'Manage W' })).toBeTruthy()
    const strip = screen.getByRole('navigation', { name: 'Administration sections' })
    expect(within(strip).getAllByRole('link').map((link) => link.textContent)).toEqual(['Workspace information', 'Members', 'Team members', 'Invitations', 'Recovery', 'Integrations (MCP)', 'Audit log'])
    expect(screen.queryByRole('heading', { name: 'Members' })).toBeNull()
    expect(screen.queryByLabelText('Search by name, e-mail or team')).toBeNull()
  })

  it('pages the members on the client, 25 at a time, and says which ones are shown', async () => {
    api.listMembers.mockResolvedValue(many)
    mount('OWNER')
    await screen.findByText('Person 00')
    expect(screen.queryByText('Person 25')).toBeNull()
    expect(screen.getByText('1–25 of 60')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Previous' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(await screen.findByText('Person 25')).toBeTruthy()
    expect(screen.getByText('26–50 of 60')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByText('51–60 of 60')).toBeTruthy()
    expect((screen.getByRole('button', { name: 'Next' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('searches name, e-mail and team, goes back to the first page, and says when nothing matches', async () => {
    api.listMembers.mockResolvedValue(many)
    mount('OWNER')
    await screen.findByText('Person 00')
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    const box = screen.getByLabelText('Search by name, e-mail or team')
    fireEvent.change(box, { target: { value: 'person 07' } })
    expect(await screen.findByText('Person 07')).toBeTruthy()
    expect(screen.queryByText('Person 08')).toBeNull()
    expect(screen.queryByRole('navigation', { name: 'Pages' })).toBeNull()
    fireEvent.change(box, { target: { value: 'legal' } })
    expect(await screen.findByText('Person 40')).toBeTruthy()
    fireEvent.change(box, { target: { value: 'p3@' } })
    expect(await screen.findByText('Person 03')).toBeTruthy()
    fireEvent.change(box, { target: { value: 'nobody here' } })
    expect(await screen.findByText('No member matches the search')).toBeTruthy()
  })

  it('shows two teams and a count instead of a long list, with the full list on hover', async () => {
    api.listMembers.mockResolvedValue(many)
    mount('OWNER')
    fireEvent.change(await screen.findByLabelText('Search by name, e-mail or team'), { target: { value: 'person 40' } })
    const cell = (await screen.findByText(/Core, Ops/)).closest('td') as HTMLElement
    expect(cell.getAttribute('title')).toBe('Core, Ops, Legal, Sales')
    expect(within(cell).getByText('+2 more')).toBeTruthy()
  })
})

describe('ticket trash', () => {
  const trashed = [{ id: 't1', key: 'PEN-1', title: 'Old plan', hierarchyKind: 'TASK', trashedAt: '2026-10-27T00:00:00Z', trashedBy: 'u1', trashedByName: 'Olivia', archived: false }]

  const rich = [
    { id: 'a', key: 'PEN-2', title: 'Alpha plan', hierarchyKind: 'TASK', trashedAt: '2026-10-20T00:00:00Z', trashedBy: 'u1', trashedByName: 'Olivia', archived: true, purgeAt: '2026-11-19T00:00:00Z', purgeBlocked: null },
    { id: 'b', key: 'PEN-10', title: 'Beta parent', hierarchyKind: 'TASK', trashedAt: '2026-10-25T00:00:00Z', trashedBy: 'u1', trashedByName: 'Olivia', archived: false, purgeAt: '2026-11-24T00:00:00Z', purgeBlocked: 'children' },
    { id: 'c', key: 'PEN-3', title: 'Gamma note', hierarchyKind: 'TASK', trashedAt: '2026-10-22T00:00:00Z', trashedBy: 'u1', trashedByName: 'Olivia', archived: false, purgeAt: '2026-11-21T00:00:00Z', purgeBlocked: null },
  ]

  it('shows when each ticket is purged, what is on hold and why, and that a restore comes back archived', async () => {
    api.listProjectTrash.mockResolvedValue(rich)
    mount('ADMIN', 'u3', <TrashSection projectId="p1" canPurge />)
    await screen.findByText('Alpha plan')
    expect(screen.getByText('Archived')).toBeTruthy()
    expect(screen.getByText('Restoring it brings it back archived.')).toBeTruthy()
    expect(screen.getAllByText(/Purged automatically on/)).toHaveLength(2)
    expect(screen.getByText(/Automatic purge on hold: It still has child tickets/)).toBeTruthy()
    expect(screen.queryByText(/Automatic purge is off/)).toBeNull()
    // A held ticket cannot be purged by hand either.
    expect((screen.getByRole('button', { name: 'Purge PEN-10' }) as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByRole('button', { name: 'Purge PEN-2' }) as HTMLButtonElement).disabled).toBe(false)
  })

  it('says once that the automatic purge is off, instead of repeating it on every ticket', async () => {
    api.listProjectTrash.mockResolvedValue(rich.map((ticket) => ({ ...ticket, purgeAt: null })))
    mount('ADMIN', 'u3', <TrashSection projectId="p1" canPurge />)
    await screen.findByText('Alpha plan')
    expect(screen.getAllByText(/Automatic purge is off/)).toHaveLength(1)
    expect(screen.queryByText(/Purged automatically on/)).toBeNull()
    // With the sweep off, a ticket that cannot be purged says why without claiming an automatic purge is on hold.
    expect(screen.getByText('It still has child tickets, so it cannot be purged. Deal with them first.')).toBeTruthy()
    expect(screen.queryByText(/Automatic purge on hold/)).toBeNull()
  })

  it('searches, filters to archived and sorts', async () => {
    api.listProjectTrash.mockResolvedValue(rich)
    mount('ADMIN', 'u3', <TrashSection projectId="p1" canPurge />)
    await screen.findByText('Alpha plan')
    const keys = () => screen.getAllByRole('button', { name: /^Restore / }).map((button) => button.getAttribute('aria-label'))
    expect(keys()).toEqual(['Restore PEN-10', 'Restore PEN-3', 'Restore PEN-2'])
    fireEvent.change(screen.getByLabelText('Sort'), { target: { value: 'key' } })
    expect(keys()).toEqual(['Restore PEN-2', 'Restore PEN-3', 'Restore PEN-10'])
    fireEvent.change(screen.getByLabelText('Search the trash'), { target: { value: 'gamma' } })
    expect(keys()).toEqual(['Restore PEN-3'])
    fireEvent.change(screen.getByLabelText('Search the trash'), { target: { value: 'nothing' } })
    expect(screen.getByText('No tickets match.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search the trash'), { target: { value: '' } })
    fireEvent.click(screen.getByLabelText('Archived only'))
    expect(keys()).toEqual(['Restore PEN-2'])
  })


  it('lets a Project admin restore and purge after confirming', async () => {
    api.listProjectTrash.mockResolvedValue(trashed)
    api.restoreTicketFromTrash.mockResolvedValue(undefined)
    api.purgeTicket.mockResolvedValue(undefined)
    mount('ADMIN', 'u3', <TrashSection projectId="p1" canPurge />)
    await screen.findByText('Old plan')
    fireEvent.click(screen.getByRole('button', { name: 'Restore PEN-1' }))
    await waitFor(() => expect(api.restoreTicketFromTrash).toHaveBeenCalledWith('t1'))
    fireEvent.click(screen.getByRole('button', { name: 'Purge PEN-1' }))
    expect(within(await screen.findByRole('dialog')).getByText(/PEN-1/)).toBeTruthy()
    await answer('Purge')
    await waitFor(() => expect(api.purgeTicket).toHaveBeenCalledWith('t1'))
  })

  it('offers purge only when the caller may (the server allows only the Project admin), and explains a refused purge', async () => {
    api.listProjectTrash.mockResolvedValue(trashed)
    api.restoreTicketFromTrash.mockResolvedValue(undefined)
    mount('MEMBER', 'u2', <TrashSection projectId="p1" canPurge={false} />)
    await screen.findByText('Old plan')
    expect(screen.queryByRole('button', { name: 'Purge PEN-1' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Restore PEN-1' })).toBeTruthy()
    cleanup()

    api.purgeTicket.mockRejectedValue(new ApiError(409, 'ticket_has_children', 'x'))
    // A Project admin who is only a plain Workspace member can purge: the Workspace role does not matter.
    mount('MEMBER', 'u2', <TrashSection projectId="p1" canPurge />)
    await screen.findByText('Old plan')
    fireEvent.click(screen.getByRole('button', { name: 'Purge PEN-1' }))
    await answer('Purge')
    expect(await screen.findByText(/child tickets|하위 티켓/)).toBeTruthy()
  })

  it('moves a Ticket to the trash after confirming and then closes the panel', async () => {
    api.trashTicket.mockResolvedValue(undefined)
    const onDone = vi.fn()
    mount('ADMIN', 'u3', <TrashTicketButton ticketId="t1" ticketKey="PEN-1" projectId="p1" onDone={onDone} />)
    fireEvent.click(await screen.findByRole('button', { name: 'Move to trash' }))
    await answer('Move to trash')
    await waitFor(() => expect(api.trashTicket).toHaveBeenCalledWith('t1'))
    await waitFor(() => expect(onDone).toHaveBeenCalled())
  })
})
