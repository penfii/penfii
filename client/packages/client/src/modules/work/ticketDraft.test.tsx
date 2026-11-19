import { StrictMode } from 'react'
import { useNavigate } from 'react-router'
import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'
import { DRAFT_TTL_MS, readTicketDraft, restorable, ticketDraftKey, writeTicketDraft, type TicketDraft } from './ticketDraft'

const api = vi.hoisted(() => ({
  me: vi.fn(), getProjectBoard: vi.fn(), getFavorites: vi.fn(), getLater: vi.fn(), getProjectPins: vi.fn(), getSpace: vi.fn(), getTicket: vi.fn(), getTicketChildren: vi.fn(), getProjectLabels: vi.fn(),
  getProjectMembers: vi.fn(), getProjectTicketTypes: vi.fn(), getProjectViews: vi.fn(), getProjectPlanning: vi.fn(), createTicket: vi.fn(), createFile: vi.fn(), deleteFile: vi.fn(),
}))
const transfers = vi.hoisted(() => ({ putFile: vi.fn() }))
vi.mock('../../api', () => ({ api }))
vi.mock('../files/upload', () => transfers)

import { DetailPanelHost } from './testHost'
import { NoticeHost, dismissNotice } from '../../design/notices'
import { renderWithProviders } from '../../test/renderWithProviders'

const PATH = '/w/w/projects/p/work'
const CREATE = `${PATH}?panel=ticket-new&newProject=p`
const KEY = ticketDraftKey('u1', 'w', 'p', {})

function mount(route = CREATE) {
  // The router here is in memory; "Reopen" builds its URL from the address bar, as it does in the browser.
  window.history.replaceState({}, '', PATH)
  return renderWithProviders(<><DetailPanelHost /><NoticeHost /></>, { route, path: '/w/:workspaceId/projects/:projectId/work' })
}

const draft = (patch: Partial<TicketDraft> = {}): TicketDraft => ({
  title: 'Kept title', description: 'Kept body', kind: 'TASK', parentId: null, channelId: null, assigneeId: null, priority: 'HIGH', dueOn: null, typeId: null,
  labelIds: [], estimate: '', hadAttachments: false, updatedAt: Date.now() - 60_000, ...patch,
})

beforeEach(() => {
  localStorage.clear()
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  URL.createObjectURL = vi.fn(() => 'blob:preview')
  URL.revokeObjectURL = vi.fn()
  api.me.mockResolvedValue({ id: 'u1', email: 'a@x', displayName: 'Alice' })
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
  api.getProjectLabels.mockResolvedValue([{ id: 'l1', name: 'backend', color: null }])
  api.getProjectMembers.mockResolvedValue([{ userId: 'u1', displayName: 'John Park' }])
  api.getProjectTicketTypes.mockResolvedValue([])
  api.getTicketChildren.mockResolvedValue([])
  api.getProjectViews.mockResolvedValue([])
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
  api.getProjectPins.mockResolvedValue({ canManage: false, pins: [] })
  api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
  api.getProjectBoard.mockResolvedValue(fixtureBoard([]))
  api.createFile.mockResolvedValue({ id: 'f1' })
  api.deleteFile.mockResolvedValue(undefined)
  transfers.putFile.mockImplementation(() => ({ done: Promise.resolve({ state: 'AVAILABLE', mime: 'text/plain' }), abort: vi.fn() }))
})
afterEach(() => { act(() => dismissNotice()); cleanup() })

describe('the draft store', () => {
  it('keeps a draft per person, Workspace, Project and origin', () => {
    expect(ticketDraftKey('u1', 'w', 'p', {})).toBe('penfii.ticketDraft.v1:u1:w:p:project')
    expect(ticketDraftKey('u1', 'w', 'p', { parentId: 't9' })).toBe('penfii.ticketDraft.v1:u1:w:p:parent:t9')
    expect(ticketDraftKey('u1', 'w', 'p', { messageId: 'm1', parentId: 't9' })).toBe('penfii.ticketDraft.v1:u1:w:p:chat:m1')
    expect(new Set([ticketDraftKey('u1', 'w', 'p', {}), ticketDraftKey('u2', 'w', 'p', {}), ticketDraftKey('u1', 'w2', 'p', {}), ticketDraftKey('u1', 'w', 'p2', {})]).size).toBe(4)
  })

  it('drops a draft that has not been edited for 14 days when it is read', () => {
    const now = Date.now()
    writeTicketDraft(KEY, draft({ updatedAt: now - DRAFT_TTL_MS + 1000 }))
    expect(readTicketDraft(KEY, now)?.title).toBe('Kept title')
    writeTicketDraft(KEY, draft({ updatedAt: now - DRAFT_TTL_MS - 1000 }))
    expect(readTicketDraft(KEY, now)).toBeNull()
    expect(localStorage.getItem(KEY)).toBeNull()
  })

  it('ignores damaged data', () => {
    localStorage.setItem(KEY, '{nope')
    expect(readTicketDraft(KEY)).toBeNull()
    localStorage.setItem(KEY, JSON.stringify({ title: 'x' }))
    expect(readTicketDraft(KEY)).toBeNull()
  })

  it('leaves out values that no longer exist and says so', () => {
    const valid = { labelIds: new Set(['l1']), memberIds: new Set(['u1']), typeIds: new Set<string>(), ticketIds: new Set<string>(), channelIds: new Set<string>() }
    const kept = restorable(draft({ labelIds: ['l1', 'gone'], assigneeId: 'u1' }), valid)
    expect(kept.dropped).toBe(true)
    expect(kept.draft.labelIds).toEqual(['l1'])
    expect(kept.draft.assigneeId).toBe('u1')
    const lost = restorable(draft({ assigneeId: 'x', parentId: 'p9', channelId: 'c9', typeId: 't9' }), valid)
    expect([lost.draft.assigneeId, lost.draft.parentId, lost.draft.channelId, lost.draft.typeId]).toEqual([null, null, null, null])
    expect(restorable(draft(), valid).dropped).toBe(false)
  })
})

describe('the create form keeps what was typed', () => {
  it('closes an empty form at once, with no notice and nothing kept', async () => {
    mount()
    await screen.findByLabelText('Title *')
    fireEvent.keyDown(screen.getByLabelText('Title *'), { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(screen.queryByText('Ticket draft kept')).toBeNull()
    expect(localStorage.getItem(KEY)).toBeNull()
  })

  it('closes without asking when nothing is attached, keeps the text, and Reopen brings it back with a way to discard it', async () => {
    mount()
    const title = await screen.findByLabelText('Title *')
    fireEvent.change(title, { target: { value: 'Half written' } })
    fireEvent.change(screen.getByLabelText('Priority'), { target: { value: 'HIGH' } })
    fireEvent.keyDown(title, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toMatchObject({ title: 'Half written', priority: 'HIGH', hadAttachments: false })
    const notice = screen.getByText('Ticket draft kept')
    expect(notice).toBeTruthy()
    fireEvent.click(within(notice.closest('[role]') as HTMLElement).getByRole('button', { name: 'Reopen' }))
    const again = (await screen.findByLabelText('Title *')) as HTMLInputElement
    expect(again.value).toBe('Half written')
    expect((screen.getByLabelText('Priority') as HTMLSelectElement).value).toBe('HIGH')
    expect(within(screen.getByRole('complementary', { name: 'Create ticket' })).getByRole('status').textContent).toContain('Restored the draft you were writing')
    expect(document.activeElement).toBe(again)
    // Discard asks first; "Keep writing" is the safe answer and has the first focus.
    fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }))
    const dialog = await screen.findByRole('dialog')
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Keep writing' }))
    fireEvent.click(within(dialog).getByRole('button', { name: 'Keep writing' }))
    expect(localStorage.getItem(KEY)).not.toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Discard draft' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Discard' }))
    await waitFor(() => expect(localStorage.getItem(KEY)).toBeNull())
    expect((screen.getByLabelText('Title *') as HTMLInputElement).value).toBe('')
    expect(within(screen.getByRole('complementary', { name: 'Create ticket' })).queryByRole('status')).toBeNull()
  })

  it('restores a kept draft when the form opens, and not another Project, origin or person', async () => {
    writeTicketDraft(KEY, draft({ labelIds: ['l1', 'deleted'], hadAttachments: true }))
    writeTicketDraft(ticketDraftKey('u2', 'w', 'p', {}), draft({ title: 'Someone else' }))
    writeTicketDraft(ticketDraftKey('u1', 'w', 'p', { parentId: 't9' }), draft({ title: 'Other origin' }))
    mount()
    const title = (await screen.findByLabelText('Title *')) as HTMLInputElement
    expect(title.value).toBe('Kept title')
    expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe('Kept body')
    const status = within(screen.getByRole('complementary', { name: 'Create ticket' })).getByRole('status').textContent ?? ''
    expect(status).toContain('Attachments are not kept in a draft')
    expect(status).toContain('Some properties could not be restored.')
    // No file card is drawn as if the files were back.
    expect(screen.queryByText('a.txt')).toBeNull()
  })

  it('does not restore a draft older than 14 days', async () => {
    writeTicketDraft(KEY, draft({ updatedAt: Date.now() - DRAFT_TTL_MS - 5000 }))
    mount()
    expect(((await screen.findByLabelText('Title *')) as HTMLInputElement).value).toBe('')
    expect(within(screen.getByRole('complementary', { name: 'Create ticket' })).queryByRole('status')).toBeNull()
    expect(localStorage.getItem(KEY)).toBeNull()
  })

  it('removes the draft when the ticket is created', async () => {
    writeTicketDraft(KEY, draft())
    api.createTicket.mockResolvedValue(ticket({ id: 'new', key: 'PFI-009', title: 'Kept title' }))
    mount()
    await screen.findByLabelText('Title *')
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    await waitFor(() => expect(api.createTicket).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(localStorage.getItem(KEY)).toBeNull())
    // The server's own message is never added to the failure text elsewhere; success leaves nothing behind.
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(localStorage.getItem(KEY)).toBeNull()
  })

  it('keeps the draft when creating fails, without showing the server message', async () => {
    const { ApiError } = await import('@penfii/api-client')
    api.createTicket.mockRejectedValue(new ApiError(400, 'validation', 'title is invalid'))
    mount()
    const title = await screen.findByLabelText('Title *')
    fireEvent.change(title, { target: { value: 'Keep me' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create' }))
    const alert = await screen.findByText(/Could not create the ticket/)
    expect(alert.textContent).not.toContain('title is invalid')
    await waitFor(() => expect(JSON.parse(localStorage.getItem(KEY) ?? '{}').title).toBe('Keep me'))
  })
})

describe('closing with files attached', () => {
  async function attach() {
    mount()
    const title = await screen.findByLabelText('Title *')
    fireEvent.change(title, { target: { value: 'With a file' } })
    const zone = screen.getByLabelText('Description').closest('.file-drop')!
    fireEvent.drop(zone, { dataTransfer: { types: ['Files'], files: [new File(['x'], 'a.txt', { type: 'text/plain' })] } })
    await screen.findByText('a.txt')
    return title
  }

  it('asks before Esc closes, keeps everything on "Keep writing", and keeps only the text on "Close"', async () => {
    const title = await attach()
    fireEvent.keyDown(title, { key: 'Escape' })
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('Attachments are not kept')).toBeTruthy()
    expect(dialog.textContent).toContain('Closing removes the files you attached (1).')
    expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Keep writing' }))
    fireEvent.click(within(dialog).getByRole('button', { name: 'Keep writing' }))
    expect((screen.getByLabelText('Title *') as HTMLInputElement).value).toBe('With a file')
    expect(screen.getByText('a.txt')).toBeTruthy()
    expect(screen.getByRole('complementary', { name: 'Create ticket' })).toBeTruthy()
    fireEvent.click(within(screen.getByRole('complementary', { name: 'Create ticket' })).getAllByRole('button', { name: 'Close' })[0])
    fireEvent.click(within(await screen.findByRole('dialog')).getByText('Close', { selector: 'button.btn' }))
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(JSON.parse(localStorage.getItem(KEY) ?? '{}')).toMatchObject({ title: 'With a file', hadAttachments: true })
    expect(screen.getByText('Ticket draft kept. Attachments were not kept.')).toBeTruthy()
    // The files go with the form, as before.
    await waitFor(() => expect(api.deleteFile).toHaveBeenCalledWith('f1'))
  })

  it('does not ask when only failed uploads are attached', async () => {
    transfers.putFile.mockImplementation(() => ({ done: Promise.resolve({ state: 'REJECTED', mime: null }), abort: vi.fn() }))
    mount()
    const title = await screen.findByLabelText('Title *')
    fireEvent.change(title, { target: { value: 'x' } })
    const zone = screen.getByLabelText('Description').closest('.file-drop')!
    fireEvent.drop(zone, { dataTransfer: { types: ['Files'], files: [new File(['x'], 'bad.txt', { type: 'text/plain' })] } })
    await screen.findByText('bad.txt')
    await waitFor(() => expect(within(screen.getByRole('complementary', { name: 'Create ticket' })).getAllByRole('alert').length).toBeGreaterThan(0))
    fireEvent.keyDown(title, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('the panel going away by itself', () => {
  function Away() {
    const navigate = useNavigate()
    return <button type="button" onClick={() => void navigate(PATH)}>elsewhere</button>
  }

  it('says the draft is kept when a navigation (the browser\'s Back, a link) removes the panel', async () => {
    window.history.replaceState({}, '', PATH)
    renderWithProviders(<><DetailPanelHost /><NoticeHost /><Away /></>, { route: CREATE, path: '/w/:workspaceId/projects/:projectId/work' })
    fireEvent.change(await screen.findByLabelText('Title *'), { target: { value: 'Left by a link' } })
    fireEvent.click(screen.getByRole('button', { name: 'elsewhere' }))
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'Create ticket' })).toBeNull())
    expect(await screen.findByText('Ticket draft kept')).toBeTruthy()
    expect(JSON.parse(localStorage.getItem(KEY) ?? '{}').title).toBe('Left by a link')
  })

  it('under React\'s development double mount, a restored draft gets no notice and keeps saving edits', async () => {
    writeTicketDraft(KEY, draft())
    window.history.replaceState({}, '', PATH)
    renderWithProviders(<StrictMode><DetailPanelHost /><NoticeHost /></StrictMode>, { route: CREATE, path: '/w/:workspaceId/projects/:projectId/work' })
    const title = (await screen.findByLabelText('Title *')) as HTMLInputElement
    expect(title.value).toBe('Kept title')
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)) })
    expect(screen.queryByText('Ticket draft kept')).toBeNull()
    fireEvent.change(title, { target: { value: 'Edited after restore' } })
    await waitFor(() => expect(JSON.parse(localStorage.getItem(KEY) ?? '{}').title).toBe('Edited after restore'))
  })
})
