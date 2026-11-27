import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  getSpace: vi.fn(),
  me: vi.fn(),
  listWorkspaces: vi.fn(),
  getWorkspaces: vi.fn(),
  getProjectMembers: vi.fn(),
  getChannelMembers: vi.fn(),
  serverInfo: vi.fn(),
}))
vi.mock('../api', () => ({ api }))
vi.mock('../modules/attention/MuteButton', () => ({ MuteButton: () => null }))
// The pane's parts that talk to the server are replaced by stand-ins that can say "not found", the way a failed read does.
vi.mock('../modules/conversations', () => ({
  Composer: ({ label }: { label: string }) => <div data-testid="composer">{label}</div>,
  ConversationLiveProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  ThreadPanel: ({ onNotFound }: { onNotFound: () => void }) => <button type="button" onClick={onNotFound}>thread read fails</button>,
  Timeline: ({ onNotFound }: { onNotFound: () => void }) => <button type="button" onClick={onNotFound}>timeline read fails</button>,
  useOutboundMessage: () => ({ attempt: null, submit: vi.fn() }),
}))
vi.mock('../modules/conversations/useServerDraft', () => ({ useServerDraft: vi.fn() }))

import { ConversationFrame } from './SpacePage'
import { ConversationPane } from './ConversationPane'
import { renderWithProviders } from '../test/renderWithProviders'

const space = (over: Partial<Space> = {}) =>
  ({
    teams: [],
    projects: [{ id: 'p', workspaceId: 'w', name: 'Alpha', teamIds: [], participating: true }],
    channels: [{ id: 'c1', conversationId: 'c1', name: 'room', visibility: 'PUBLIC', teamId: null, projectId: null, unreadCount: 0, role: 'MEMBER', joined: true }],
    dms: [],
    ...over,
  }) as unknown as Space

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockReset()
  api.me.mockResolvedValue({ id: 'me', displayName: 'Me' })
  api.listWorkspaces.mockResolvedValue([])
  api.getWorkspaces.mockResolvedValue([])
  api.getProjectMembers.mockResolvedValue([])
  api.getChannelMembers.mockResolvedValue([])
  api.serverInfo.mockResolvedValue({ limits: { messageBodyMaxBytes: 4000 }, features: [] })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function mountChat(id: string) {
  return renderWithProviders(<ConversationFrame />, { route: `/w/w/c/${id}`, path: '/w/:workspaceId/c/:conversationId' })
}
function mountProject(id: string) {
  return renderWithProviders(<ConversationFrame />, { route: `/w/w/projects/p/chat/${id}`, path: '/w/:workspaceId/projects/:projectId/chat/:conversationId' })
}
const unavailable = () => screen.queryByText('This conversation cannot be opened')

describe('a conversation that is not in the list (FEA-196)', () => {
  it('says it cannot be opened, gives the reason in the product’s words and the way back to Chat', async () => {
    api.getSpace.mockResolvedValue(space())
    mountChat('gone')
    expect(await screen.findByText('This conversation cannot be opened')).toBeTruthy()
    expect(screen.getByText('It was deleted, or you do not have access to it.')).toBeTruthy()
    const back = screen.getByRole('link', { name: 'Back to Chat' })
    expect(back.getAttribute('href')).toBe('/w/w/chat')
    // Not the bare "Not found" of the other screens.
    expect(screen.queryByText('Not found')).toBeNull()
  })

  it('goes back to the Project chat from a Project channel the person can still open', async () => {
    api.getSpace.mockResolvedValue(space())
    mountProject('gone')
    const back = await screen.findByRole('link', { name: 'Back to the Project chat' })
    expect(back.getAttribute('href')).toBe('/w/w/projects/p/chat')
  })

  it('does not point back into a Project that is closed to the person, nor one that is not there', async () => {
    api.getSpace.mockResolvedValue(space({ projects: [{ id: 'p', workspaceId: 'w', name: 'Alpha', teamIds: [], participating: false }] as never }))
    mountProject('gone')
    expect((await screen.findByRole('link', { name: 'Back to Chat' })).getAttribute('href')).toBe('/w/w/chat')
    cleanup()
    api.getSpace.mockResolvedValue(space({ projects: [] }))
    mountProject('gone')
    expect((await screen.findByRole('link', { name: 'Back to Chat' })).getAttribute('href')).toBe('/w/w/chat')
  })

  it('does not decide from a list that was read before: it reads the list again first, and shows "loading" meanwhile', async () => {
    let second: (value: Space) => void = () => undefined
    api.getSpace.mockResolvedValue(space())
    api.getSpace.mockResolvedValueOnce(space()).mockImplementationOnce(() => new Promise<Space>((resolve) => { second = resolve }))
    mountChat('new')
    expect((await screen.findByRole('status')).textContent).toBe('Loading')
    expect(unavailable()).toBeNull()
    await waitFor(() => expect(api.getSpace).toHaveBeenCalledTimes(2))
    // The channel was made, or the person put into it, after the first list: the second one has it.
    await act(async () => second(space({ channels: [...space().channels, { id: 'new', conversationId: 'new', name: 'fresh', visibility: 'PUBLIC', teamId: null, projectId: null, unreadCount: 0, role: 'MEMBER', joined: true }] as never })))
    expect(await screen.findByRole('heading', { name: '# fresh' })).toBeTruthy()
    expect(unavailable()).toBeNull()
  })

  it('says "unavailable" only once the fresh list also lacks it', async () => {
    let second: (value: Space) => void = () => undefined
    api.getSpace.mockResolvedValue(space())
    api.getSpace.mockResolvedValueOnce(space()).mockImplementationOnce(() => new Promise<Space>((resolve) => { second = resolve }))
    mountChat('gone')
    await screen.findByRole('status')
    await waitFor(() => expect(api.getSpace).toHaveBeenCalledTimes(2))
    expect(unavailable()).toBeNull()
    await act(async () => second(space()))
    expect(await screen.findByText('This conversation cannot be opened')).toBeTruthy()
  })

  it('does not call a list that failed to load a missing conversation: it says so and offers to try again', async () => {
    api.getSpace.mockRejectedValueOnce(new ApiError(503, 'db_unavailable', 'the database is temporarily unavailable'))
    mountChat('c1')
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/database is temporarily unavailable/i)
    expect(unavailable()).toBeNull()
    api.getSpace.mockResolvedValue(space())
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(await screen.findByRole('heading', { name: '# room' })).toBeTruthy()
  })

  it('shows an ordinary conversation as before', async () => {
    api.getSpace.mockResolvedValue(space())
    mountChat('c1')
    expect(await screen.findByRole('heading', { name: '# room' })).toBeTruthy()
    expect(unavailable()).toBeNull()
  })
})

describe('a conversation or thread that cannot be read once it is open (FEA-196)', () => {
  function pane(threadRootId: string | null, projectId?: string) {
    api.getSpace.mockResolvedValue(space())
    return renderWithProviders(
      <ConversationPane workspaceId="w" conversationId="c1" threadRootId={threadRootId} currentUserId="me" names={{}} showDmWarning={false} heading="# room" threadBase="/w/w/c/c1" projectId={projectId} />,
      { route: '/w/w/c/c1' },
    )
  }

  it('replaces the messages and the box with the unavailable state, with the way back', async () => {
    pane(null)
    await screen.findByRole('button', { name: 'timeline read fails' })
    await waitFor(() => expect(screen.getByTestId('composer')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'timeline read fails' }))
    expect(await screen.findByText('This conversation cannot be opened')).toBeTruthy()
    expect(screen.queryByTestId('composer')).toBeNull()
    expect(screen.queryByRole('button', { name: 'timeline read fails' })).toBeNull()
    expect(screen.getByRole('link', { name: 'Back to Chat' }).getAttribute('href')).toBe('/w/w/chat')
  })

  it('goes back to the Project chat when it was opened from a Project', async () => {
    pane(null, 'p')
    fireEvent.click(await screen.findByRole('button', { name: 'timeline read fails' }))
    expect((await screen.findByRole('link', { name: 'Back to the Project chat' })).getAttribute('href')).toBe('/w/w/projects/p/chat')
  })

  it('a thread that is gone leaves the conversation as it is and offers the way back to it', async () => {
    pane('root1')
    fireEvent.click(await screen.findByRole('button', { name: 'thread read fails' }))
    expect(await screen.findByText('This thread cannot be opened')).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Back to the conversation' }).getAttribute('href')).toBe('/w/w/c/c1')
    // The conversation itself is still there, with its box; the thread's box is not.
    expect(screen.getByRole('button', { name: 'timeline read fails' })).toBeTruthy()
    expect(screen.getAllByTestId('composer')).toHaveLength(1)
    expect(unavailable()).toBeNull()
  })
})
