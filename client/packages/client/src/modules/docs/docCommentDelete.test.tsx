import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getDocComments: vi.fn(), deleteDocComment: vi.fn(), searchDirectory: vi.fn(), resolveReferences: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { DocComments } from './DocComments'
import { renderWithProviders } from '../../test/renderWithProviders'

const comment = { id: 'c1', authorId: 'u1', authorName: 'Mina', createdAt: '2026-10-20T00:00:00Z', editedAt: null,
  body: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }] }, attachments: [] }

function mount(over: { userId?: string; canManage?: boolean } = {}) {
  return renderWithProviders(<DocComments workspaceId="w" docId="d" userId={over.userId ?? 'u1'} canManage={over.canManage ?? false} refreshTick={0} />)
}

describe('deleting a Doc comment', () => {
  beforeEach(() => {
    api.getDocComments.mockResolvedValue({ items: [comment], nextCursor: null })
    api.deleteDocComment.mockResolvedValue(undefined)
    api.searchDirectory.mockResolvedValue({ results: [] })
    api.resolveReferences.mockResolvedValue([])
  })
  afterEach(() => { cleanup(); vi.clearAllMocks() })

  it('asks first, says it cannot be undone, and deletes nothing on Cancel', async () => {
    mount()
    fireEvent.click(await screen.findByRole('button', { name: '삭제' }))
    expect(await screen.findByText(/되돌릴 수 없습니다/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '취소' }))
    await waitFor(() => expect(screen.queryByText(/되돌릴 수 없습니다/)).toBeNull())
    expect(api.deleteDocComment).not.toHaveBeenCalled()
  })

  it('calls the API once after Confirm and reloads the list', async () => {
    mount()
    fireEvent.click(await screen.findByRole('button', { name: '삭제' }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(dialog.querySelector('button.btn-danger, button[class*="danger"]') as HTMLElement)
    await waitFor(() => expect(api.deleteDocComment).toHaveBeenCalledTimes(1))
    expect(api.deleteDocComment).toHaveBeenCalledWith('d', 'c1')
    await waitFor(() => expect(api.getDocComments.mock.calls.length).toBeGreaterThan(1))
  })

  it('shows an error and keeps the comment when the API fails', async () => {
    api.deleteDocComment.mockRejectedValue(new Error('x'))
    mount()
    fireEvent.click(await screen.findByRole('button', { name: '삭제' }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(dialog.querySelector('button[class*="danger"]') as HTMLElement)
    await waitFor(() => expect(api.deleteDocComment).toHaveBeenCalled())
    expect(await screen.findByText('hello')).toBeTruthy()
  })

  it('offers no Delete to someone who neither wrote it nor manages the page', async () => {
    mount({ userId: 'u2', canManage: false })
    await screen.findByText('hello')
    expect(screen.queryByRole('button', { name: '삭제' })).toBeNull()
  })
})
