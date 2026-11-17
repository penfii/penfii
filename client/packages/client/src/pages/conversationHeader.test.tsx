import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  getSpace: vi.fn(),
  me: vi.fn(),
  listWorkspaces: vi.fn(),
  getWorkspaces: vi.fn(),
  getProjectMembers: vi.fn(),
  getChannelMembers: vi.fn(),
}))
vi.mock('../api', () => ({ api }))
vi.mock('./ConversationPane', () => ({ ConversationPane: () => null }))
vi.mock('../modules/attention/MuteButton', () => ({ MuteButton: () => null }))

import { ConversationFrame } from './SpacePage'
import { renderWithProviders } from '../test/renderWithProviders'

const person = (userId: string, displayName: string) => ({ userId, displayName })
const space = {
  teams: [],
  projects: [{ id: 'p', workspaceId: 'w', name: 'Alpha', teamIds: [], participating: true }],
  channels: [
    { id: 'pub', conversationId: 'pub', name: 'room', visibility: 'PUBLIC', teamId: null, projectId: 'p', unreadCount: 0 },
    { id: 'sec', conversationId: 'sec', name: 'secret', visibility: 'PRIVATE', teamId: null, projectId: 'p', role: 'MEMBER', joined: true, unreadCount: 0 },
  ],
  dms: [],
} as unknown as Space

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue(space)
  api.me.mockResolvedValue({ id: 'me', displayName: 'Me' })
  api.listWorkspaces.mockResolvedValue([])
  api.getWorkspaces.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([person('me', 'Me'), person('a', 'Ann'), person('b', 'Bo')])
  api.getChannelMembers.mockResolvedValue([{ ...person('me', 'Me'), role: 'MANAGER', joinedAt: '2026-01-01T00:00:00Z' }])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function mount(id: string) {
  return renderWithProviders(<ConversationFrame />, { route: `/w/w/projects/p/chat/${id}`, path: '/w/:workspaceId/projects/:projectId/chat/:conversationId' })
}

describe('the header of a Project channel', () => {
  it('counts the Project for a public channel, which everyone in the Project reads', async () => {
    mount('pub')
    expect(await screen.findByText(/3 members/)).toBeTruthy()
    expect(api.getChannelMembers).not.toHaveBeenCalled()
  })

  it('counts the channel members for a private channel, not the Project members', async () => {
    mount('sec')
    expect(await screen.findByText(/1 members/)).toBeTruthy()
    expect(screen.queryByText(/3 members/)).toBeNull()
    expect(api.getProjectMembers).not.toHaveBeenCalled()
  })
})
