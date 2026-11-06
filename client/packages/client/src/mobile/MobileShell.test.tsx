import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { Link } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getSpace: vi.fn(),
  listWorkspaces: vi.fn(),
  getInbox: vi.fn(),
  me: vi.fn(),
  getNotificationPreferences: vi.fn(),
  saveNotificationPreferences: vi.fn(),
  logout: vi.fn(),
  getDocTree: vi.fn(),
  getProjectPlanning: vi.fn(),
  listDrafts: vi.fn(),
}))
vi.mock('../api', () => ({ api }))
vi.mock('../shell/create', () => ({ useCreate: () => () => undefined, CreateProvider: ({ children }: { children: unknown }) => children }))
// The real lists need the whole Space; what is under test is how the shell treats a link in them.
vi.mock('../shell/ContextSidebar', () => ({
  ContextSidebar: ({ section }: { section: string }) => (
    <aside>
      context list: {section}
      <Link to="/w/w/inbox?box=mentions">Mentions</Link>
      <Link to="/w/w/directory">Organization</Link>
      <Link to="/w/w/directory/members">Members</Link>
    </aside>
  ),
}))
vi.mock('../shell/DetailPanel', () => ({ DetailPanel: () => null }))
vi.mock('../modules/attention/useUserSocket', () => ({ useUserSocket: () => undefined }))
vi.mock('../modules/attention/CommandPalette', () => ({ CommandPalette: () => null }))
vi.mock('../modules/help/ShortcutHelp', () => ({ ShortcutHelpProvider: ({ children }: { children: unknown }) => children }))

import { MobileShell } from './MobileShell'
import { renderWithProviders } from '../test/renderWithProviders'
import { PHONE, setViewport } from '../test/viewport'

type Section = 'inbox' | 'chat' | 'projects' | 'docs' | 'teams' | 'settings' | 'search'

function mount(path: string, section: Section, route = '/w/:workspaceId/*') {
  return renderWithProviders(<MobileShell section={section}><p>content</p></MobileShell>, { route: path, path: route })
}

beforeEach(() => {
  setViewport(PHONE)
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p1', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.listWorkspaces.mockResolvedValue([{ id: 'w', name: 'Acme', role: 'OWNER' }, { id: 'w2', name: 'Beta', role: 'MEMBER' }])
  api.getInbox.mockResolvedValue({ items: [], nextCursor: null, unreadCount: 0, unreadCapped: false })
  api.me.mockResolvedValue({ id: 'u1', displayName: 'Bob', email: 'bob@example.com', hasAvatar: false, statusEmoji: '', statusText: '', statusClearAt: '', jobTitle: '', timeZone: '' })
  api.getNotificationPreferences.mockResolvedValue({ level: 'ALL', dnd: { enabled: false, snoozeUntil: null }, external: {}, version: 3 })
  api.saveNotificationPreferences.mockResolvedValue(undefined)
  api.logout.mockResolvedValue(undefined)
  api.getDocTree.mockResolvedValue([{ id: 'd1', title: 'Release notes' }])
  api.listDrafts.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('MobileShell roots and pushes', () => {
  it('shows the context list and the bottom tabs on a root', async () => {
    mount('/w/w/projects', 'projects')
    expect(await screen.findByText(/context list: projects/)).toBeTruthy()
    const tabs = screen.getByRole('navigation', { name: 'Main menu' })
    expect(tabs.querySelectorAll('a')).toHaveLength(8)
    expect(screen.queryByText('content')).toBeNull()
  })

  it('pushes the content with a back button and no tabs below a root, titled by the project', async () => {
    mount('/w/w/projects/p1/work', 'projects', '/w/:workspaceId/projects/:projectId/*')
    expect(await screen.findByText('content')).toBeTruthy()
    expect(screen.getByRole('button', { name: /^Back to / })).toBeTruthy()
    expect(screen.queryByRole('navigation', { name: 'Main menu' })).toBeNull()
    expect(await screen.findByRole('heading', { name: 'Penfii App' })).toBeTruthy()
  })

  it('opens an Inbox box instead of showing the list again (a query-driven screen)', async () => {
    mount('/w/w/inbox', 'inbox')
    fireEvent.click(await screen.findByRole('link', { name: 'Mentions' }))
    expect(await screen.findByText('content')).toBeTruthy()
    expect(screen.queryByRole('navigation', { name: 'Main menu' })).toBeNull()
  })

  it('opens Teams → Organization, a link to the root path itself, as a screen', async () => {
    mount('/w/w/directory', 'teams')
    fireEvent.click(await screen.findByRole('link', { name: 'Organization' }))
    expect(await screen.findByText('content')).toBeTruthy()
    expect(screen.queryByText(/context list/)).toBeNull()
    expect(screen.getByRole('button', { name: /^Back to / })).toBeTruthy()
  })

  it('opens a link below the root as before', async () => {
    mount('/w/w/directory', 'teams')
    fireEvent.click(await screen.findByRole('link', { name: 'Members' }))
    expect(await screen.findByText('content')).toBeTruthy()
  })

  it('keeps the document list as the root until a document is chosen, and names the document', async () => {
    mount('/w/w/docs', 'docs')
    expect(await screen.findByText(/context list: docs/)).toBeTruthy()
    cleanup()
    mount('/w/w/docs?doc=d1', 'docs')
    expect(await screen.findByText('content')).toBeTruthy()
    expect(await screen.findByRole('heading', { name: 'Release notes' })).toBeTruthy()
  })

  it('shows the search field on the search root and pushes the results when a query is typed', async () => {
    mount('/w/w/search', 'search')
    const field = await screen.findByRole('searchbox')
    fireEvent.change(field, { target: { value: 'alpha' } })
    fireEvent.submit(field.closest('form') as HTMLFormElement)
    expect(await screen.findByText('content')).toBeTruthy()
  })
})

describe('MobileShell thread screen', () => {
  it('is a screen of its own: titled Thread, with Back naming the channel, and no tabs', async () => {
    api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [{ id: 'ch', name: 'general', conversationId: 'c1', projectId: null, teamId: null }], dms: [] })
    mount('/w/w/c/c1/t/r1', 'chat', '/w/:workspaceId/c/:conversationId/t/:rootId')
    expect(await screen.findByRole('heading', { level: 1, name: 'Thread' })).toBeTruthy()
    expect((await screen.findByRole('button', { name: /^Back to # general/ })).textContent).toContain('# general')
    expect(document.querySelector('.mobile-tabs')).toBeNull()
    expect(document.querySelector('.mobile-shell')?.hasAttribute('data-thread')).toBe(true)
  })
})

describe('MobileShell workspace and account', () => {
  it('treats a workspace the person cannot open as not found', async () => {
    mount('/w/zzz/projects', 'projects', '/w/:workspaceId/*')
    expect(await screen.findByText('Go to your workspaces')).toBeTruthy()
    expect(screen.queryByRole('navigation', { name: 'Main menu' })).toBeNull()
  })

  it('remembers the workspace for the session', async () => {
    mount('/w/w/projects', 'projects')
    await screen.findByText(/context list/)
    expect(sessionStorage.getItem('penfii.lastWorkspace')).toBe('w')
  })

  it('offers the account menu in Settings: status, quiet hour, switch, help, sign out', async () => {
    mount('/w/w/settings', 'settings')
    expect(await screen.findByText('bob@example.com')).toBeTruthy()
    for (const name of ['Set a status', 'Quiet for 1 hour', 'Switch workspace', 'About', 'Sign out']) {
      expect(screen.getByRole('button', { name: new RegExp(name) }), name).toBeTruthy()
    }
  })

  it('turns Do Not Disturb on for an hour', async () => {
    mount('/w/w/settings', 'settings')
    fireEvent.click(await screen.findByRole('button', { name: /Quiet for 1 hour/ }))
    await waitFor(() => expect(api.saveNotificationPreferences).toHaveBeenCalled())
    const body = api.saveNotificationPreferences.mock.calls[0][0] as { dnd: { enabled: boolean; snoozeUntil: string }; expectedVersion: number }
    expect(body.dnd.enabled).toBe(true)
    expect(new Date(body.dnd.snoozeUntil).getTime()).toBeGreaterThan(Date.now())
    expect(body.expectedVersion).toBe(3)
  })

  it('lists the workspaces in a sheet and logs out', async () => {
    mount('/w/w/settings', 'settings')
    fireEvent.click(await screen.findByRole('button', { name: /Switch workspace/ }))
    const sheet = await screen.findByRole('dialog', { name: 'Switch workspace' })
    expect(sheet.textContent).toContain('Beta')
    fireEvent.click(screen.getByText('Close', { selector: '.m-sheet-close' }))
    fireEvent.click(screen.getByRole('button', { name: 'Sign out' }))
    await waitFor(() => expect(api.logout).toHaveBeenCalled())
  })
})

describe('MobileShell accessibility', () => {
  it('names the unread count inside the Inbox tab and not on a label-less element', async () => {
    api.getInbox.mockResolvedValue({ items: [], nextCursor: null, unreadCount: 4, unreadCapped: false })
    mount('/w/w/projects', 'projects')
    const tab = await screen.findByRole('link', { name: /Inbox/ })
    await waitFor(() => expect(tab.textContent).toMatch(/4/))
    expect(tab.querySelector('[aria-label]')).toBeNull()
    expect(tab.querySelector('.visually-hidden')?.textContent).toMatch(/4/)
  })

  it('brings the active tab into view when the area changes', async () => {
    const scroll = vi.fn()
    Element.prototype.scrollIntoView = scroll
    mount('/w/w/directory', 'teams')
    await screen.findByText(/context list/)
    await waitFor(() => expect(scroll).toHaveBeenCalled())
    expect((scroll.mock.contexts[0] as HTMLElement).getAttribute('aria-current')).toBe('page')
    Reflect.deleteProperty(Element.prototype, 'scrollIntoView')
  })

  it('keeps exactly one level-1 heading: the push bar title on a push, a hidden one on a root', async () => {
    renderWithProviders(<MobileShell section="projects"><h1>Page title</h1></MobileShell>, { route: '/w/w/projects/p1/work', path: '/w/:workspaceId/projects/:projectId/*' })
    await screen.findByText('Page title')
    await waitFor(() => expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1))
    expect(await screen.findByRole('heading', { level: 1, name: 'Penfii App' })).toBeTruthy()
    expect(screen.getByRole('heading', { level: 2, name: 'Page title' })).toBeTruthy()
    cleanup()
    mount('/w/w/projects', 'projects')
    await screen.findByText(/context list/)
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1)
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Projects')
  })

  it('gives the back button a name that contains the words it shows', async () => {
    mount('/w/w/projects/p1/work', 'projects', '/w/:workspaceId/projects/:projectId/*')
    const back = await screen.findByRole('button', { name: /^Back to / })
    expect(back.getAttribute('aria-label')).toContain((back.textContent ?? '').replace('‹', '').trim())
  })
})

describe('MobileShell offline', () => {
  it('says so on top while the browser is offline', async () => {
    mount('/w/w/projects', 'projects')
    await screen.findByText(/context list/)
    expect(screen.queryByText(/You are offline/)).toBeNull()
    Object.defineProperty(navigator, 'onLine', { configurable: true, value: false })
    window.dispatchEvent(new Event('offline'))
    expect(await screen.findByText(/You are offline/)).toBeTruthy()
    Object.defineProperty(navigator, 'onLine', { configurable: true, value: true })
    window.dispatchEvent(new Event('online'))
  })
})
