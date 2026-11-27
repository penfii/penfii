import { cleanup, createEvent, fireEvent, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => () => undefined }))

import { ContextSidebar } from './ContextSidebar'
import { renderWithProviders } from '../test/renderWithProviders'
import { sectionsKey } from './chatGroups'

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

  it('renders the conversation menu outside the nav and supports keyboard, Escape and focus return', async () => {
    localStorage.setItem(sectionsKey('w'), JSON.stringify({ sections: [{ id: 'one', name: 'One', conversationIds: [] }] }))
    mount()
    const trigger = (await screen.findAllByRole('button', { name: 'Conversation menu' }))[0]
    fireEvent.click(trigger)
    const menu = await screen.findByRole('menu')
    expect(menu.parentElement).toBe(document.body)
    expect(document.activeElement).toBe(await screen.findByRole('menuitem', { name: 'One' }))
    fireEvent.keyDown(menu, { key: 'ArrowDown' })
    // The arrows also reach the Favorite row at the end of the menu.
    expect(['menuitem', 'menuitemcheckbox']).toContain(document.activeElement?.getAttribute('role'))
    fireEvent.keyDown(menu, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(document.activeElement).toBe(trigger)
  })

  it('closes the portaled conversation menu on outside click', async () => {
    localStorage.setItem(sectionsKey('w'), JSON.stringify({ sections: [{ id: 'one', name: 'One', conversationIds: [] }] }))
    mount()
    const trigger = (await screen.findAllByRole('button', { name: 'Conversation menu' }))[0]
    fireEvent.click(trigger)
    expect(await screen.findByRole('menu')).toBeTruthy()
    fireEvent.pointerDown(document.body)
    expect(screen.queryByRole('menu')).toBeNull()
    expect(document.activeElement).toBe(trigger)
  })
})
