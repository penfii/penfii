import { act, cleanup, fireEvent, screen } from '@testing-library/react'
import { useEffect, useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getSpace: vi.fn(),
  listWorkspaces: vi.fn(),
  getInbox: vi.fn(),
  me: vi.fn(),
  getNotificationPreferences: vi.fn(),
  listDrafts: vi.fn(),
}))
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => () => undefined, CreateProvider: ({ children }: { children: unknown }) => children }))
vi.mock('./ContextSidebar', () => ({ ContextSidebar: () => <aside>context list</aside> }))
vi.mock('../modules/attention/useUserSocket', () => ({ useUserSocket: () => undefined }))
vi.mock('../modules/attention/CommandPalette', () => ({ CommandPalette: () => null }))
vi.mock('../modules/help/ShortcutHelp', () => ({ ShortcutHelpProvider: ({ children }: { children: unknown }) => children }))
vi.mock('./DetailPanel', () => ({ DetailPanel: () => <Typed label="detail draft" /> }))

import { ResponsiveShell } from './ResponsiveShell'
import { renderWithProviders } from '../test/renderWithProviders'

const mounts = { page: 0, detail: 0 }

/** A form with text the person has typed and not saved: it lives only in this component's state. */
function Typed({ label }: { label: string }) {
  const [text, setText] = useState('')
  const kind = label === 'detail draft' ? 'detail' : 'page'
  useEffect(() => {
    mounts[kind] += 1
  }, [kind])
  return <input aria-label={label} value={text} onChange={(event) => setText(event.target.value)} />
}

/** A window whose width can change under a mounted tree, with the `change` event a browser sends. */
function resizableWindow(initial: number) {
  let width = initial
  const lists = new Set<{ query: string; listeners: Set<() => void> }>()
  const matches = (query: string) => {
    const max = /\(max-width:\s*(\d+)px\)/.exec(query)
    return max ? width <= Number(max[1]) : false
  }
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string) => {
      const entry = { query, listeners: new Set<() => void>() }
      lists.add(entry)
      return {
        get matches() { return matches(query) },
        media: query,
        addEventListener: (_type: string, listener: () => void) => entry.listeners.add(listener),
        removeEventListener: (_type: string, listener: () => void) => entry.listeners.delete(listener),
        addListener: () => undefined,
        removeListener: () => undefined,
        dispatchEvent: () => false,
        onchange: null,
      }
    },
  })
  return (next: number) => {
    width = next
    act(() => lists.forEach((entry) => entry.listeners.forEach((listener) => listener())))
  }
}

beforeEach(() => {
  mounts.page = 0
  mounts.detail = 0
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p1', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.listWorkspaces.mockResolvedValue([{ id: 'w', name: 'Acme', role: 'OWNER' }])
  api.getInbox.mockResolvedValue({ items: [], nextCursor: null, unreadCount: 0, unreadCapped: false })
  api.me.mockResolvedValue({ id: 'u1', displayName: 'Bob', email: 'bob@example.com', hasAvatar: false, statusEmoji: '', statusText: '', statusClearAt: '', jobTitle: '', timeZone: '' })
  api.getNotificationPreferences.mockResolvedValue({ level: 'ALL', dnd: { enabled: false, snoozeUntil: null }, external: {}, version: 1 })
  api.listDrafts.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  Reflect.deleteProperty(window, 'matchMedia')
})

function mount() {
  return renderWithProviders(
    <ResponsiveShell section="projects"><Typed label="page draft" /></ResponsiveShell>,
    { route: '/w/w/projects/p1/work', path: '/w/:workspaceId/projects/:projectId/*' },
  )
}

describe('the shell across the phone width', () => {
  it('keeps the page and the Detail Panel mounted, and what was typed in them, when the window is resized across 640px', async () => {
    const resize = resizableWindow(1280)
    mount()
    const page = (await screen.findByLabelText('page draft')) as HTMLInputElement
    const detail = (await screen.findByLabelText('detail draft')) as HTMLInputElement
    fireEvent.change(page, { target: { value: 'half a paragraph' } })
    fireEvent.change(detail, { target: { value: 'half a description' } })

    resize(390)
    expect(await screen.findByRole('button', { name: /^Back to / })).toBeTruthy()
    expect((screen.getByLabelText('page draft') as HTMLInputElement).value).toBe('half a paragraph')
    expect((screen.getByLabelText('detail draft') as HTMLInputElement).value).toBe('half a description')

    resize(1280)
    expect(screen.queryByRole('button', { name: /^Back to / })).toBeNull()
    expect((screen.getByLabelText('page draft') as HTMLInputElement).value).toBe('half a paragraph')
    expect((screen.getByLabelText('detail draft') as HTMLInputElement).value).toBe('half a description')
    // Each was mounted once, however often the frame changed.
    expect(mounts).toEqual({ page: 1, detail: 1 })
  })

  it('puts the page where the active frame expects it', async () => {
    const resize = resizableWindow(1280)
    const { container } = mount()
    await screen.findByLabelText('page draft')
    expect(container.querySelector('.product-shell > .product-main')).not.toBeNull()
    resize(390)
    await screen.findByRole('button', { name: /^Back to / })
    expect(container.querySelector('.mobile-main > .product-main')).not.toBeNull()
    expect(container.querySelector('.product-shell')).toBeNull()
  })
})
