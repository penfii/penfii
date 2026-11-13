import { cleanup, fireEvent, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ listProjectArchive: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { ArchivedTickets } from './ArchivedTickets'
import { renderWithProviders } from '../../test/renderWithProviders'

const row = (key: string, title: string, kind: string, archivedAt: string, trashed = false) => ({ id: key, key, title, hierarchyKind: kind, archivedAt, archivedBy: null, archivedByName: 'felix', trashed })
const rows = [
  row('PEN-2', 'Old release', 'TASK', '2026-10-01T00:00:00Z'),
  row('PEN-10', 'New feature epic', 'EPIC', '2026-11-01T00:00:00Z'),
  row('PEN-3', 'Fix login', 'SUBTASK', '2026-10-15T00:00:00Z', true),
]

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.listProjectArchive.mockReset()
})
afterEach(cleanup)

describe('Archived tickets list', () => {
  const mount = (onOpen = vi.fn()) => renderWithProviders(<ArchivedTickets projectId="p" onOpen={onOpen} onClose={() => undefined} />)
  const keys = () => screen.getAllByRole('button', { name: /^PEN-/ }).map((button) => button.textContent?.split(' ')[0])

  it('sorts newest first, by oldest and by key, and counts what is shown', async () => {
    api.listProjectArchive.mockResolvedValue(rows)
    mount()
    await screen.findByText('Old release')
    expect(keys()).toEqual(['PEN-10', 'PEN-3', 'PEN-2'])
    fireEvent.change(screen.getByLabelText('Sort'), { target: { value: 'oldest' } })
    expect(keys()).toEqual(['PEN-2', 'PEN-3', 'PEN-10'])
    fireEvent.change(screen.getByLabelText('Sort'), { target: { value: 'key' } })
    expect(keys()).toEqual(['PEN-2', 'PEN-3', 'PEN-10'])
    expect(screen.getByText('3 of 3')).toBeTruthy()
  })

  it('searches key and title, filters by kind, says when nothing matches and opens a ticket', async () => {
    api.listProjectArchive.mockResolvedValue(rows)
    const onOpen = vi.fn()
    mount(onOpen)
    await screen.findByText('Old release')
    fireEvent.change(screen.getByLabelText('Search archived tickets'), { target: { value: 'login' } })
    expect(keys()).toEqual(['PEN-3'])
    expect(screen.getByText('Restore it from the trash', { exact: false })).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search archived tickets'), { target: { value: 'pen-10' } })
    expect(keys()).toEqual(['PEN-10'])
    fireEvent.change(screen.getByLabelText('Search archived tickets'), { target: { value: '' } })
    fireEvent.change(screen.getByLabelText('Kind'), { target: { value: 'EPIC' } })
    expect(keys()).toEqual(['PEN-10'])
    expect(screen.getByText('1 of 3')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search archived tickets'), { target: { value: 'zzz' } })
    expect(screen.getByText('No tickets match.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search archived tickets'), { target: { value: '' } })
    fireEvent.click(screen.getByRole('button', { name: /PEN-10/ }))
    expect(onOpen).toHaveBeenCalledWith('PEN-10')
  })

  it('has an empty state', async () => {
    api.listProjectArchive.mockResolvedValue([])
    mount()
    expect(await screen.findByText('No archived tickets')).toBeTruthy()
    expect(screen.queryByLabelText('Search archived tickets')).toBeNull()
  })
})
