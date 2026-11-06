import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '@penfii/contracts'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), putReaction: vi.fn(), deleteReaction: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { MessageRow } from './MessageRow'

const message = {
  id: 'm1', conversationId: 'c1', streamSeq: 2, authorId: 'u1', authorDisplayName: 'Alice', threadRootId: null, replyToId: null, version: 1,
  bodySchemaVersion: 1, body: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }] }, deleted: false,
  createdAt: '2026-11-05T02:00:00Z', editedAt: null, replyCount: 0, reactions: [], attachments: [],
} as unknown as Message

const onManualUnread = vi.fn()
function row() {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  return render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <MemoryRouter>
        <I18nProvider>
          <MessageRow message={message} currentUserId="u1" names={{}} scrollRoot={null} onVisible={() => undefined} onMessage={() => undefined} onNotFound={() => undefined} onManualUnread={onManualUnread} />
        </I18nProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const hold = (target: Element, type: string) => {
  fireEvent.pointerDown(target, { pointerType: type, clientX: 4, clientY: 4 })
  act(() => { vi.advanceTimersByTime(600) })
}

beforeEach(() => {
  vi.useFakeTimers()
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  onManualUnread.mockReset()
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe('a message on a phone', () => {
  beforeEach(() => setViewport(PHONE))

  it('opens its actions in a sheet after a touch is held, with the reactions and the menu', () => {
    const view = row()
    hold(view.container.querySelector('article') as Element, 'touch')
    const sheet = screen.getByRole('dialog', { name: 'Message actions' })
    expect(within(sheet).getAllByRole('menuitem').length).toBeGreaterThan(3)
    expect(within(sheet).getByRole('menuitem', { name: 'Copy' })).toBeTruthy()
    expect(within(sheet).getByRole('menuitem', { name: 'Edit' })).toBeTruthy()
    expect(sheet.querySelectorAll('.reaction-bar button').length).toBeGreaterThan(0)
  })

  it('does nothing for a held mouse button', () => {
    const view = row()
    hold(view.container.querySelector('article') as Element, 'mouse')
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('runs the chosen action and closes the sheet', () => {
    const view = row()
    hold(view.container.querySelector('article') as Element, 'touch')
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mark unread from here' }))
    expect(onManualUnread).toHaveBeenCalledWith(2)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('has a visible actions button, because there is no hover: it opens the same sheet', () => {
    const view = row()
    fireEvent.click(view.container.querySelector('.message-more') as Element)
    expect(screen.getByRole('dialog').classList.contains('m-sheet')).toBe(true)
  })

  it('sends a chosen reaction and closes the sheet', () => {
    api.putReaction.mockResolvedValue({})
    const view = row()
    hold(view.container.querySelector('article') as Element, 'touch')
    const dialog = screen.getByRole('dialog')
    fireEvent.click(within(dialog).getAllByRole('button').find((button) => button.getAttribute('aria-label')?.startsWith('React')) as Element)
    expect(api.putReaction).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('closes on Escape and on the backdrop', () => {
    const view = row()
    hold(view.container.querySelector('article') as Element, 'touch')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
    hold(view.container.querySelector('article') as Element, 'touch')
    fireEvent.click(screen.getByText('Close', { selector: '.m-sheet-close' }))
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('a message on a desktop-sized window', () => {
  beforeEach(() => {
    setViewport(DESKTOP)
    // jsdom has no scrollIntoView; the hover menu asks for it when it opens.
    Element.prototype.scrollIntoView = vi.fn()
  })

  it('does not react to a long press and keeps the hover menu', () => {
    const view = row()
    hold(view.container.querySelector('article') as Element, 'touch')
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(view.container.querySelector('.message-more')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Message actions' }))
    expect(screen.getByRole('menu', { name: 'Message actions' })).toBeTruthy()
  })
})
