import { cleanup, screen, waitFor } from '@testing-library/react'
import { createElement, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Channel, Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), getServerInfo: vi.fn(), getDraft: vi.fn(), saveDraft: vi.fn(), deleteDraft: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('../modules/conversations', () => ({
  Composer: () => createElement('div', null, 'COMPOSER'),
  ConversationLiveProvider: ({ children }: { children: ReactNode }) => createElement('div', null, children),
  ThreadPanel: () => null,
  Timeline: () => null,
  useOutboundMessage: () => ({ send: vi.fn(), pending: [], retry: vi.fn(), discard: vi.fn() }),
}))

import { ConversationPane } from './ConversationPane'
import { renderWithProviders } from '../test/renderWithProviders'

const channel = (over: Partial<Channel>): Channel => ({
  id: 'c', conversationId: 'conv', workspaceId: 'w', name: 'general', description: null, topic: null, visibility: 'PUBLIC',
  teamId: null, projectId: null, allowMemberInvites: false, archivedAt: null, role: 'MEMBER', joined: true, unreadCount: 0, ...over,
} as Channel)

const space = (channels: Channel[]): Space => ({ teams: [], projects: [], channels, dms: [] })

function mount() {
  return renderWithProviders(<ConversationPane workspaceId="w" conversationId="conv" threadRootId={null} currentUserId="u" names={{}} showDmWarning={false} heading="# general" threadBase="/w/w/c/conv" />, { route: '/w/w/c/conv' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getServerInfo.mockResolvedValue({ limits: { messageBodyMaxBytes: 65536 } })
  api.getDraft.mockResolvedValue(null)
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Drafts of a conversation', () => {
  it('are loaded in a channel the person has joined', async () => {
    api.getSpace.mockResolvedValue(space([channel({ joined: true })]))
    mount()
    await waitFor(() => expect(api.getDraft).toHaveBeenCalledWith('CHANNEL_COMPOSE', 'conv'))
  })

  it('are not requested in a public channel the person has not joined, where the server would answer 404', async () => {
    api.getSpace.mockResolvedValue(space([channel({ joined: false, role: null })]))
    mount()
    await screen.findByRole('button', { name: /join/i })
    expect(api.getDraft).not.toHaveBeenCalled()
  })
})
