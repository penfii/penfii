import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  getChannelMembers: vi.fn(), addChannelMember: vi.fn(), setChannelMemberRole: vi.fn(), searchDirectory: vi.fn(), getPeople: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { ChannelMembersDialog } from './ChannelMembersDialog'
import { renderWithProviders } from '../../test/renderWithProviders'

const member = (userId: string, displayName: string, role: 'MANAGER' | 'MEMBER') => ({ userId, displayName, role, joinedAt: '2026-11-04T00:00:00Z' })
const hit = (id: string, displayName: string) => ({ type: 'person', id, person: { id, displayName, username: null } })

function mount(canAdd = true, canSetRoles = false) {
  return renderWithProviders(<ChannelMembersDialog open onClose={() => undefined} workspaceId="w" channelId="ch" canAdd={canAdd} canSetRoles={canSetRoles} />)
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  HTMLDialogElement.prototype.showModal = HTMLDialogElement.prototype.showModal ?? function (this: HTMLDialogElement) { this.setAttribute('open', '') }
  api.getChannelMembers.mockResolvedValue([member('u1', 'Owner', 'MANAGER')])
  api.getPeople.mockResolvedValue([])
  api.searchDirectory.mockResolvedValue({ results: [hit('u1', 'Owner'), hit('u2', 'Bob')] })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Putting people into a private channel', () => {
  it('lists the members, offers only the people who are not in yet, and adds the one picked', async () => {
    api.addChannelMember.mockResolvedValue(member('u2', 'Bob', 'MEMBER'))
    mount()
    expect(await screen.findByText('Owner')).toBeTruthy()
    const pick = await screen.findByRole('button', { name: /Bob/ })
    expect(screen.queryByRole('button', { name: /Owner/ }), 'a member is not offered again').toBeNull()
    api.getChannelMembers.mockResolvedValue([member('u1', 'Owner', 'MANAGER'), member('u2', 'Bob', 'MEMBER')])
    fireEvent.click(pick)
    await waitFor(() => expect(api.addChannelMember).toHaveBeenCalledWith('ch', 'u2'))
    expect(await screen.findByText('Bob was added.')).toBeTruthy()
  })

  it("shows the server's reason when a person cannot be added", async () => {
    api.addChannelMember.mockRejectedValue(Object.assign(new Error('that person does not take part in the Project of this channel'), { status: 400, code: 'validation' }))
    mount()
    fireEvent.click(await screen.findByRole('button', { name: /Bob/ }))
    expect(await screen.findByRole('alert')).toBeTruthy()
  })

  it('only lists the members to someone who cannot add', async () => {
    mount(false)
    expect(await screen.findByText('Owner')).toBeTruthy()
    expect(screen.queryByText('Find people to add')).toBeNull()
  })
})

describe('Giving a private channel another manager', () => {
  const two = () => api.getChannelMembers.mockResolvedValue([member('u1', 'Owner', 'MANAGER'), member('u2', 'Bob', 'MEMBER')])

  it('lets a manager change a role from the member list and reads the list again', async () => {
    two()
    api.setChannelMemberRole.mockResolvedValue(member('u2', 'Bob', 'MANAGER'))
    mount(true, true)
    const select = await screen.findByRole('combobox', { name: 'Bob: Role' })
    api.getChannelMembers.mockResolvedValue([member('u1', 'Owner', 'MANAGER'), member('u2', 'Bob', 'MANAGER')])
    fireEvent.change(select, { target: { value: 'MANAGER' } })
    await waitFor(() => expect(api.setChannelMemberRole).toHaveBeenCalledWith('ch', 'u2', 'MANAGER'))
    expect(await screen.findByText("Bob's role was changed.")).toBeTruthy()
    await waitFor(() => expect((screen.getByRole('combobox', { name: 'Bob: Role' }) as HTMLSelectElement).value).toBe('MANAGER'))
  })

  it('shows a plain member the roles as text, with nothing to change', async () => {
    two()
    mount(false, false)
    expect(await screen.findByText('Bob')).toBeTruthy()
    expect(screen.queryByRole('combobox')).toBeNull()
    expect(screen.getByText(/Manager/)).toBeTruthy()
  })

  it('says why the last manager cannot be made a member, and keeps the list as the server has it', async () => {
    two()
    api.setChannelMemberRole.mockRejectedValue(new ApiError(409, 'last_manager', 'a channel needs at least one manager'))
    mount(true, true)
    fireEvent.change(await screen.findByRole('combobox', { name: 'Owner: Role' }), { target: { value: 'MEMBER' } })
    expect((await screen.findByRole('alert')).textContent).toContain('needs at least one manager')
    await waitFor(() => expect((screen.getByRole('combobox', { name: 'Owner: Role' }) as HTMLSelectElement).value).toBe('MANAGER'))
  })
})
