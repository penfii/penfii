import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Channel, Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('../shell/WorkspaceShell', () => ({ OpenCreate: () => null }))

import { ChatHomePage } from './ChatHomePage'
import { renderWithProviders } from '../test/renderWithProviders'

const channel = (over: Partial<Channel>): Channel => ({
  id: 'c', conversationId: 'c', workspaceId: 'w', name: 'general', description: null, topic: null, visibility: 'PUBLIC',
  teamId: null, projectId: null, allowMemberInvites: false, archivedAt: null, role: 'MEMBER', unreadCount: 0, ...over,
} as Channel)

const space = (channels: Channel[]): Space => ({
  teams: [],
  projects: [{ id: 'p1', workspaceId: 'w', name: 'Alpha', ticketPrefix: 'ALP' }] as unknown as Space['projects'],
  channels,
  dms: [],
})

function mount() {
  return renderWithProviders(<ChatHomePage />, { route: '/w/w/chat', path: '/w/:workspaceId/chat' })
}

beforeEach(() => localStorage.setItem('penfii.locale', 'ko-KR'))
afterEach(cleanup)

describe('Chat entry point', () => {
  it('lists the channels of Projects next to the others, opening the same conversation the Project screen opens', async () => {
    api.getSpace.mockResolvedValue(space([channel({ id: 'g', conversationId: 'conv-g', name: 'general' }), channel({ id: 'a', conversationId: 'conv-a', name: 'Alpha', projectId: 'p1' })]))
    mount()
    const project = await screen.findByRole('link', { name: /# Alpha/ })
    expect(project.getAttribute('href')).toBe('/w/w/c/conv-a')
    expect(project.textContent).toContain('Alpha · 프로젝트 멤버')
    expect(screen.getByRole('link', { name: /# general/ }).getAttribute('href')).toBe('/w/w/c/conv-g')
  })

  it('does not show the empty state when only a Project has channels', async () => {
    api.getSpace.mockResolvedValue(space([channel({ id: 'a', conversationId: 'conv-a', name: 'Alpha', projectId: 'p1' })]))
    mount()
    expect(await screen.findByRole('link', { name: /# Alpha/ })).toBeTruthy()
    expect(screen.queryByText('채널을 만들거나 다이렉트 메시지를 시작해 보세요.')).toBeNull()
  })
})
