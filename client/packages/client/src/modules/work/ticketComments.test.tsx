import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TicketComment } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  me: vi.fn(),
  getTicketComments: vi.fn(),
  createTicketComment: vi.fn(),
  editTicketComment: vi.fn(),
  deleteTicketComment: vi.fn(),
  createFile: vi.fn(),
  deleteFile: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { CommentComposer, CommentList } from './CommentsTab'
import { renderWithProviders } from '../../test/renderWithProviders'

function CommentsTab({ ticketId, workspaceId }: { ticketId: string; workspaceId: string }) {
  return (
    <>
      <CommentList ticketId={ticketId} workspaceId={workspaceId} />
      <CommentComposer ticketId={ticketId} workspaceId={workspaceId} />
    </>
  )
}

const text = (value: string) => ({ type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: value }] }] })
const comment = (id: string, value: string, rights: Partial<TicketComment> = {}): TicketComment => ({
  id,
  ticketId: 't1',
  authorId: 'u1',
  authorName: 'Alice',
  body: text(value),
  createdAt: '2026-10-29T00:00:00Z',
  editedAt: null,
  attachments: [],
  canEdit: false,
  canDelete: false,
  ...rights,
})

function view() {
  return renderWithProviders(<CommentsTab ticketId="t1" workspaceId="w1" />)
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Ticket comment edit and delete', () => {
  it('offers Edit and Delete only where the server says the person may', async () => {
    api.getTicketComments.mockResolvedValue([
      comment('mine', 'mine', { canEdit: true, canDelete: true }),
      comment('moderated', 'theirs', { canDelete: true }),
      comment('plain', 'read only'),
    ])
    view()
    await screen.findByText('mine')
    expect(screen.getAllByRole('button', { name: 'Edit' })).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: 'Delete' })).toHaveLength(2)
  })

  it('saves an edit and marks the comment as edited', async () => {
    api.getTicketComments.mockResolvedValueOnce([comment('c1', 'before', { canEdit: true, canDelete: true })])
    api.editTicketComment.mockResolvedValue({})
    api.getTicketComments.mockResolvedValue([comment('c1', 'after', { canEdit: true, canDelete: true, editedAt: '2026-10-29T01:00:00Z' })])
    view()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    fireEvent.change(screen.getByLabelText('Edit'), { target: { value: 'after' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledWith('t1', 'c1', text('after')))
    expect(await screen.findByText('after')).toBeTruthy()
    expect(screen.getByText(/edited/)).toBeTruthy()
  })

  it('deletes only after the person confirms', async () => {
    api.getTicketComments.mockResolvedValue([comment('c1', 'bye', { canDelete: true })])
    api.deleteTicketComment.mockResolvedValue(undefined)
    view()
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }))
    expect(api.deleteTicketComment).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(Array.from(dialog.querySelectorAll('button')).find((b) => b.textContent === 'Delete')!)
    await waitFor(() => expect(api.deleteTicketComment).toHaveBeenCalledWith('t1', 'c1'))
  })

  it('says so when the change is refused', async () => {
    api.getTicketComments.mockResolvedValue([comment('c1', 'x', { canEdit: true })])
    api.editTicketComment.mockRejectedValue(new Error('403'))
    view()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByRole('alert')).toBeTruthy()
  })
})

describe('Ticket comment list when long', () => {
  const many = Array.from({ length: 9 }, (_, i) => comment(`c${String(i + 1)}`, `comment ${String(i + 1)}`, { createdAt: `2026-10-29T0${String(i)}:00:00Z`, authorId: i >= 6 ? 'u2' : 'u1' }))

  it('shows the newest five and folds the earlier ones behind a button', async () => {
    api.me.mockResolvedValue({ id: 'me' })
    api.getTicketComments.mockResolvedValue(many)
    view()
    expect(await screen.findByText('comment 9')).toBeTruthy()
    expect(screen.queryByText('comment 4')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Show 4 earlier comments' }))
    expect(await screen.findByText('comment 1')).toBeTruthy()
  })

  it('marks where comments by others since the last visit begin, and never counts my own', async () => {
    api.me.mockResolvedValue({ id: 'u1' })
    localStorage.setItem('penfii.ticketSeen.t1', '2026-10-29T05:30:00Z')
    api.getTicketComments.mockResolvedValue(many)
    view()
    // u1 is me: only the u2 comments after the stored time count.
    expect((await screen.findByRole('separator')).textContent).toBe('3 new comments from here')
  })

  it('shows no marker the first time a Ticket is opened', async () => {
    api.me.mockResolvedValue({ id: 'me' })
    api.getTicketComments.mockResolvedValue(many)
    view()
    await screen.findByText('comment 9')
    expect(screen.queryByRole('separator')).toBeNull()
  })
})
