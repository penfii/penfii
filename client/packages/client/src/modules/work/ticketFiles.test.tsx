import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Ticket, TicketAttachment } from '@penfii/contracts'

const api = vi.hoisted(() => ({
  me: vi.fn(),
  getTicketAttachments: vi.fn(),
  attachTicketFile: vi.fn(),
  deleteTicketAttachment: vi.fn(),
  getTicketComments: vi.fn(),
  createTicketComment: vi.fn(),
  createFile: vi.fn(),
  deleteFile: vi.fn(),
}))
const transfers = vi.hoisted(() => ({ putFile: vi.fn() }))
vi.mock('../../api', () => ({ api }))
vi.mock('../files/upload', () => transfers)

import { CommentComposer, CommentList } from './CommentsTab'

function CommentsTab({ ticketId, workspaceId }: { ticketId: string; workspaceId: string }) {
  return (
    <>
      <CommentList ticketId={ticketId} workspaceId={workspaceId} />
      <CommentComposer ticketId={ticketId} workspaceId={workspaceId} />
    </>
  )
}
import { TicketAttachments } from './TicketAttachments'
import { renderWithProviders } from '../../test/renderWithProviders'

const attachment = (id: string, filename: string, extra: Partial<TicketAttachment> = {}): TicketAttachment => ({
  id,
  fileId: `file-${id}`,
  filename,
  byteSize: 10,
  mime: 'application/octet-stream',
  state: 'AVAILABLE',
  ticketId: 't1',
  commentId: null,
  attachedBy: 'u1',
  attachedAt: '2026-10-29T00:00:00Z',
  ...extra,
})
const ticket = { id: 't1', workspaceId: 'w1' } as Ticket

function view(node: React.ReactNode) {
  return renderWithProviders(node)
}

const pick = (name: string) => new File(['x'], name, { type: 'text/plain' })

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  URL.createObjectURL = vi.fn(() => 'blob:preview')
  URL.revokeObjectURL = vi.fn()
  api.me.mockResolvedValue({ id: 'u1', email: 'a@x', displayName: 'Alice' })
  api.createFile.mockResolvedValue({ id: 'new-file' })
  api.deleteFile.mockResolvedValue(undefined)
  transfers.putFile.mockImplementation(() => ({ done: Promise.resolve({ state: 'AVAILABLE', mime: 'text/plain' }), abort: vi.fn() }))
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Ticket body attachments', () => {
  it('lists the files and lets an editor remove one', async () => {
    api.getTicketAttachments.mockResolvedValue([attachment('a1', 'plan.pdf')])
    api.deleteTicketAttachment.mockResolvedValue(undefined)
    view(<TicketAttachments ticket={ticket} canEdit canManageProject={false} />)
    expect(await screen.findByText('plan.pdf')).toBeTruthy()
    api.getTicketAttachments.mockResolvedValue([])
    fireEvent.click(screen.getByRole('button', { name: 'Remove: plan.pdf' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Remove' }))
    await waitFor(() => expect(api.deleteTicketAttachment).toHaveBeenCalledWith('t1', 'a1'))
    await waitFor(() => expect(screen.queryByText('plan.pdf')).toBeNull())
  })

  it('lets an editor remove only their own files, and the Project admin any', async () => {
    api.getTicketAttachments.mockResolvedValue([attachment('a1', 'mine.pdf'), attachment('a2', 'theirs.pdf', { attachedBy: 'u2' })])
    const first = view(<TicketAttachments ticket={ticket} canEdit canManageProject={false} />)
    expect(await screen.findByText('theirs.pdf')).toBeTruthy()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Remove: mine.pdf' })).toBeTruthy())
    expect(screen.queryByRole('button', { name: 'Remove: theirs.pdf' })).toBeNull()
    first.unmount()
    view(<TicketAttachments ticket={ticket} canEdit canManageProject />)
    expect(await screen.findByRole('button', { name: 'Remove: theirs.pdf' })).toBeTruthy()
  })

  it('shows a reader the files without any way to add or remove', async () => {
    api.getTicketAttachments.mockResolvedValue([attachment('a1', 'plan.pdf')])
    view(<TicketAttachments ticket={ticket} canEdit={false} canManageProject={false} />)
    expect(await screen.findByText('plan.pdf')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Remove: plan.pdf' })).toBeNull()
    expect(screen.queryByLabelText('Attach file')).toBeNull()
    // Dropping does nothing for a reader.
    fireEvent.drop(screen.getByText('plan.pdf'), { dataTransfer: { types: ['Files'], files: [pick('x.txt')] } })
    expect(api.createFile).not.toHaveBeenCalled()
  })

  it('uploads dropped files and binds each to the Ticket', async () => {
    api.getTicketAttachments.mockResolvedValue([])
    api.attachTicketFile.mockResolvedValue(attachment('a9', 'a.txt'))
    view(<TicketAttachments ticket={ticket} canEdit canManageProject={false} />)
    const zone = (await screen.findByText('Drop or paste files here.')).closest('.file-drop')!
    fireEvent.drop(zone, { dataTransfer: { types: ['Files'], files: [pick('a.txt'), pick('b.txt')] } })
    await waitFor(() => expect(api.attachTicketFile).toHaveBeenCalledTimes(2))
    expect(api.createFile).toHaveBeenCalledWith('w1', { filename: 'a.txt', byteSize: 1 })
    expect(api.attachTicketFile).toHaveBeenCalledWith('t1', 'new-file')
  })
})

describe('Ticket comments with files', () => {
  it('shows the files of each comment', async () => {
    api.getTicketComments.mockResolvedValue([
      {
        id: 'c1', ticketId: 't1', authorId: 'u1', authorName: 'Alice', body: { type: 'doc', content: [] },
        createdAt: '2026-10-29T00:00:00Z', editedAt: null,
        attachments: [attachment('a1', 'proof.png', { commentId: 'c1', mime: 'image/png' })],
      },
    ])
    view(<CommentsTab ticketId="t1" workspaceId="w1" />)
    expect(await screen.findByAltText('proof.png')).toBeTruthy()
  })

  it('sends the uploaded files with the comment, even without text, and not before they finish', async () => {
    api.getTicketComments.mockResolvedValue([])
    api.createTicketComment.mockResolvedValue({})
    let finish: (value: unknown) => void = () => undefined
    transfers.putFile.mockImplementationOnce(() => ({ done: new Promise((resolve) => { finish = resolve }), abort: vi.fn() }))
    view(<CommentsTab ticketId="t1" workspaceId="w1" />)
    await screen.findByText('No comments yet.')
    const box = screen.getByLabelText('Write a comment')
    fireEvent.paste(box, { clipboardData: { files: [pick('a.txt')], types: ['Files'] } })
    const submit = screen.getByRole('button', { name: 'Comment' }) as HTMLButtonElement
    await waitFor(() => expect(api.createFile).toHaveBeenCalled())
    expect(submit.disabled).toBe(true)
    finish({ state: 'AVAILABLE', mime: 'text/plain' })
    await waitFor(() => expect(submit.disabled).toBe(false))
    fireEvent.click(submit)
    await waitFor(() => expect(api.createTicketComment).toHaveBeenCalled())
    expect(api.createTicketComment.mock.calls[0]![2]).toEqual(['new-file'])
    // The files were handed over, so nothing is deleted.
    expect(api.deleteFile).not.toHaveBeenCalled()
  })

  it('keeps the files and the text when posting fails', async () => {
    api.getTicketComments.mockResolvedValue([])
    api.createTicketComment.mockRejectedValue(new Error('boom'))
    view(<CommentsTab ticketId="t1" workspaceId="w1" />)
    await screen.findByText('No comments yet.')
    fireEvent.change(screen.getByLabelText('Write a comment'), { target: { value: 'hello' } })
    fireEvent.drop(screen.getByLabelText('Write a comment'), { dataTransfer: { types: ['Files'], files: [pick('a.txt')] } })
    await waitFor(() => expect(screen.getByText('a.txt')).toBeTruthy())
    await waitFor(() => expect((screen.getByRole('button', { name: 'Comment' }) as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(screen.getByRole('button', { name: 'Comment' }))
    await screen.findByRole('alert')
    expect(screen.getByText('a.txt')).toBeTruthy()
    expect((screen.getByLabelText('Write a comment') as HTMLTextAreaElement).value).toBe('hello')
    expect(api.deleteFile).not.toHaveBeenCalled()
  })
})
