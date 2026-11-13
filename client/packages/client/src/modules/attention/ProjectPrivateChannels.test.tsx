import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  listProjectPrivateChannels: vi.fn(), getChannelMembers: vi.fn(), addChannelMember: vi.fn(), setChannelMemberRole: vi.fn(),
  removeChannelMember: vi.fn(), updateChannel: vi.fn(), me: vi.fn(), searchDirectory: vi.fn(), getPeople: vi.fn(),
  listChannelWebhooks: vi.fn(), createChannelWebhook: vi.fn(), rotateWebhook: vi.fn(), revokeWebhook: vi.fn(), reauthPassword: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { ProjectPrivateChannels } from './ProjectPrivateChannels'
import { renderWithProviders } from '../../test/renderWithProviders'

const channel = (over: object = {}) => ({
  id: 'c1', name: 'leads', description: 'Leads only', memberCount: 3, managerCount: 1, allowMemberInvites: false, archived: false, isMember: false, ...over,
})

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  HTMLDialogElement.prototype.showModal = HTMLDialogElement.prototype.showModal ?? function (this: HTMLDialogElement) { this.setAttribute('open', '') }
  api.me.mockResolvedValue({ id: 'u1', displayName: 'Admin' })
  api.getChannelMembers.mockResolvedValue([{ userId: 'u2', displayName: 'Bob', role: 'MANAGER', joinedAt: '2026-11-04T00:00:00Z' }])
  api.getPeople.mockResolvedValue([])
  api.searchDirectory.mockResolvedValue({ results: [] })
  api.listChannelWebhooks.mockResolvedValue({ items: [] })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('The private channels of a Project, for its admin', () => {
  it('lists them with names and counts and says which ones the admin cannot read', async () => {
    api.listProjectPrivateChannels.mockResolvedValue([channel(), channel({ id: 'c2', name: 'ops', isMember: true, archived: true })])
    renderWithProviders(<ProjectPrivateChannels workspaceId="w" projectId="p1" />)
    expect(await screen.findByText('# leads')).toBeTruthy()
    expect(screen.getByText(/3 members · 1 managers · content not visible to you/)).toBeTruthy()
    expect(screen.getByText(/you are in it · archived/)).toBeTruthy()
    expect(screen.getAllByText('Leads only').length).toBe(2)
    expect(api.listProjectPrivateChannels).toHaveBeenCalledWith('p1')
  })

  it('shows nothing while the Project has no private channel', async () => {
    api.listProjectPrivateChannels.mockResolvedValue([])
    renderWithProviders(<ProjectPrivateChannels workspaceId="w" projectId="p1" />)
    await waitFor(() => expect(api.listProjectPrivateChannels).toHaveBeenCalled())
    expect(screen.queryByText('Private channels')).toBeNull()
  })

  it('opens the people of a channel the admin cannot read, with the removal and the switch', async () => {
    api.listProjectPrivateChannels.mockResolvedValue([channel()])
    renderWithProviders(<ProjectPrivateChannels workspaceId="w" projectId="p1" />)
    fireEvent.click(await screen.findByRole('button', { name: 'People and settings: leads' }))
    expect(await screen.findByText('Bob')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Remove: Bob' })).toBeTruthy()
    expect(screen.getByRole('checkbox', { name: 'Let members add other people' })).toBeTruthy()
  })

  it('lets an admin who cannot read the channel see its webhooks and revoke one, not create or rotate', async () => {
    api.listProjectPrivateChannels.mockResolvedValue([channel()])
    api.listChannelWebhooks.mockResolvedValue({
      items: [{ id: 'w1', channelId: 'c1', name: 'deploys', authMode: 'URL_SECRET', actorId: 'a1', actorDisplayName: 'Bot', revoked: false, createdAt: '2026-11-09T00:00:00Z', rotatedAt: null }],
    })
    renderWithProviders(<ProjectPrivateChannels workspaceId="w" projectId="p1" />)
    await screen.findByText('# leads')
    fireEvent.click(screen.getByRole('button', { name: 'Webhooks' }))
    expect(await screen.findByText('deploys')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Revoke' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Rotate secret' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Create webhook' })).toBeNull()
  })

  it('offers an admin who is in the channel the full webhook controls', async () => {
    api.listProjectPrivateChannels.mockResolvedValue([channel({ isMember: true })])
    renderWithProviders(<ProjectPrivateChannels workspaceId="w" projectId="p1" />)
    await screen.findByText('# leads')
    fireEvent.click(screen.getByRole('button', { name: 'Webhooks' }))
    expect(await screen.findByRole('button', { name: 'Create webhook' })).toBeTruthy()
  })
})
