import { cleanup, createEvent, fireEvent, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => () => undefined }))

import { ContextSidebar } from './ContextSidebar'
import { renderWithProviders } from '../test/renderWithProviders'

const channel = { id: 'c1', conversationId: 'conv1', name: 'general', unreadCount: 0, teamIds: [], isPrivate: false }

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [channel], dms: [] })
})
afterEach(cleanup)

function mount() {
  return renderWithProviders(
    <>
      <ContextSidebar workspaceId="w" section="chat" />
      <ContextSidebar workspaceId="w" section="chat" />
    </>,
    { route: '/w/w/chat', path: '/w/:workspaceId/chat' },
  )
}

describe('Dragging a conversation in the chat list', () => {
  it('is known only to the list it started in', async () => {
    mount()
    const slots = await screen.findAllByText('general')
    const [first, second] = slots.map((node) => node.closest('li') as HTMLElement)
    expect(second).not.toBe(first)
    fireEvent.dragStart(first, { dataTransfer: { setData: vi.fn(), effectAllowed: '' } })
    // The same list accepts a drop over it …
    const own = createEvent.dragOver(first)
    fireEvent(first, own)
    expect(own.defaultPrevented).toBe(true)
    // … another list, which did not start this drag, does not.
    const other = createEvent.dragOver(second)
    fireEvent(second, other)
    expect(other.defaultPrevented).toBe(false)
  })
})
