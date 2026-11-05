import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  me: vi.fn(), listWorkspaces: vi.fn(), getSpace: vi.fn(),
  listMembers: vi.fn(), changeMemberRole: vi.fn(), removeMember: vi.fn(), setAccountDisabled: vi.fn(),
  listInvitations: vi.fn(), revokeInvitation: vi.fn(),
  listTeamMembers: vi.fn(), addTeamMember: vi.fn(), removeTeamMember: vi.fn(), searchDirectory: vi.fn(),
  listAudit: vi.fn(),
  listProjectTrash: vi.fn(), restoreTicketFromTrash: vi.fn(), purgeTicket: vi.fn(), trashTicket: vi.fn(),
  reauthPassword: vi.fn(), reauthProviderStart: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))
vi.mock('../../shell/WorkspaceShell', () => ({
  OpenCreate: ({ label }: { label: string }) => <button type="button">{label}</button>,
}))

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

function mount(role: 'OWNER' | 'ADMIN' | 'MEMBER', meId = 'u1', element: React.ReactNode = <WorkspaceAdmin workspaceId={W} />) {
  api.listWorkspaces.mockResolvedValue([{ id: W, name: 'W', timeZone: 'UTC', role, createdAt: '2026-10-01T00:00:00Z' }])
  api.me.mockResolvedValue({ id: meId, email: 'me@example.com', displayName: 'Me' })
  return renderWithProviders(element, { route: '/' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  for (const fn of Object.values(api)) fn.mockReset()
  api.listMembers.mockResolvedValue(people)
  api.listInvitations.mockResolvedValue([])
  api.getSpace.mockResolvedValue({ teams: [{ id: 't1', name: 'Core' }], projects: [], channels: [] })
  api.listTeamMembers.mockResolvedValue([])
  api.listAudit.mockResolvedValue({ items: [], nextCursor: null })
  api.removeMember.mockResolvedValue(undefined)
  api.changeMemberRole.mockResolvedValue(undefined)
  api.setAccountDisabled.mockResolvedValue(undefined)
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

/** Answers the confirmation dialog the way a person does, by pressing one of its buttons. */
async function answer(label: string) {
  fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: label }))
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
    mount('OWNER')
    expect(await screen.findByRole('heading', { name: 'Manage W' })).toBeTruthy()
  })

  it('asks twice before ownership changes hands', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    fireEvent.change(screen.getByLabelText(/^Role of Mina/), { target: { value: 'OWNER' } })
    await answer('Cancel')
    expect(api.changeMemberRole).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText(/^Role of Mina/), { target: { value: 'OWNER' } })
    await answer('Confirm')
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenCalledWith(W, 'u2', 'OWNER'))
  })

  it('lets an owner change a role, remove a member and disable an account, but not touch themself', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    expect(within(row('Olivia')).queryByRole('button')).toBeNull()
    expect(within(row('Olivia')).queryByRole('combobox')).toBeNull()
    // A removed member is listed as removed and cannot be removed again.
    expect(within(row('Gone')).getByText('Removed')).toBeTruthy()
    expect(within(row('Gone')).queryByRole('button', { name: /Remove/ })).toBeNull()

    fireEvent.change(screen.getByLabelText(/^Role of Mina/), { target: { value: 'ADMIN' } })
    await waitFor(() => expect(api.changeMemberRole).toHaveBeenCalledWith(W, 'u2', 'ADMIN'))

    fireEvent.click(screen.getByRole('button', { name: /^Remove Mina/ }))
    await answer('Remove')
    await waitFor(() => expect(api.removeMember).toHaveBeenCalledWith(W, 'u2'))

    fireEvent.click(screen.getByRole('button', { name: /^Disable account Mina/ }))
    await answer('Disable account')
    await waitFor(() => expect(api.setAccountDisabled).toHaveBeenCalledWith(W, 'u2', true))
  })

  it('does not call the API when the confirmation is declined', async () => {
    mount('OWNER')
    await screen.findByText('Mina')
    fireEvent.click(screen.getByRole('button', { name: /^Remove Mina/ }))
    await answer('Cancel')
    expect(api.removeMember).not.toHaveBeenCalled()
  })

  it('gives an admin only the controls an admin may use', async () => {
    mount('ADMIN', 'u3')
    await screen.findByText('Mina')
    expect(screen.queryByLabelText(/^Role of Mina/)).toBeNull()
    expect(screen.getByRole('button', { name: /^Remove Mina/ })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /Disable account/ })).toBeNull()
    // Not the owner, not another admin, not themself.
    expect(within(row('Olivia')).queryByRole('button')).toBeNull()
    expect(within(row('Adam')).queryByRole('button')).toBeNull()
  })

  it('says why the last owner stays, and asks for a recent sign-in when the server does', async () => {
    api.changeMemberRole.mockRejectedValueOnce(new ApiError(409, 'last_owner', 'x'))
    mount('OWNER')
    await screen.findByText('Mina')
    fireEvent.change(screen.getByLabelText(/^Role of Adam/), { target: { value: 'MEMBER' } })
    expect(await screen.findByText('A workspace needs at least one owner.')).toBeTruthy()

    api.removeMember
      .mockRejectedValueOnce(new ApiError(403, 'reauth_required', 'x', { methods: ['password'] }))
      .mockResolvedValueOnce(undefined)
    api.reauthPassword.mockResolvedValue(undefined)
    fireEvent.click(screen.getByRole('button', { name: /^Remove Mina/ }))
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
    mount('ADMIN', 'u3')
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
    mount('OWNER')
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
    mount('OWNER')
    await waitFor(() => expect(api.listTeamMembers).toHaveBeenCalledWith(W, 't1'))
    const teamCard = (await screen.findByText('Team members')).closest('div') as HTMLElement
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
    mount('OWNER')
    const teamCard = (await screen.findByText('Team members')).closest('div') as HTMLElement
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
    mount('OWNER')
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

describe('ticket trash', () => {
  const trashed = [{ id: 't1', key: 'PEN-1', title: 'Old plan', hierarchyKind: 'TASK', trashedAt: '2026-10-27T00:00:00Z', trashedBy: 'u1', trashedByName: 'Olivia' }]

  it('lets a Project admin restore, and a Workspace admin purge after confirming', async () => {
    api.listProjectTrash.mockResolvedValue(trashed)
    api.restoreTicketFromTrash.mockResolvedValue(undefined)
    api.purgeTicket.mockResolvedValue(undefined)
    mount('ADMIN', 'u3', <TrashSection workspaceId={W} projectId="p1" />)
    await screen.findByText('Old plan')
    fireEvent.click(screen.getByRole('button', { name: 'Restore PEN-1' }))
    await waitFor(() => expect(api.restoreTicketFromTrash).toHaveBeenCalledWith('t1'))
    fireEvent.click(screen.getByRole('button', { name: 'Purge PEN-1' }))
    expect(within(await screen.findByRole('dialog')).getByText(/PEN-1/)).toBeTruthy()
    await answer('Purge')
    await waitFor(() => expect(api.purgeTicket).toHaveBeenCalledWith('t1'))
  })

  it('hides purge from someone who only administers the Project, and explains a refused purge', async () => {
    api.listProjectTrash.mockResolvedValue(trashed)
    api.restoreTicketFromTrash.mockResolvedValue(undefined)
    mount('MEMBER', 'u2', <TrashSection workspaceId={W} projectId="p1" />)
    await screen.findByText('Old plan')
    expect(screen.queryByRole('button', { name: 'Purge PEN-1' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Restore PEN-1' })).toBeTruthy()
    cleanup()

    api.purgeTicket.mockRejectedValue(new ApiError(409, 'ticket_has_children', 'x'))
    mount('OWNER', 'u1', <TrashSection workspaceId={W} projectId="p1" />)
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
