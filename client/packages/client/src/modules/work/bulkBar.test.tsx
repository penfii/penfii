import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkTicket } from './viewModel'

const api = vi.hoisted(() => ({ bulkTickets: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { BulkBar } from './BulkBar'
import { renderWithProviders } from '../../test/renderWithProviders'

const selected = [{ id: 't1', projectId: 'p' }] as WorkTicket[]

function mount(onResult: (result: unknown) => void) {
  return renderWithProviders(<BulkBar workspaceId="w" selected={selected} result={null} names={new Map()} onClear={() => undefined} onResult={onResult} />)
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.bulkTickets.mockReset()
})
afterEach(cleanup)

describe('Bulk bar', () => {
  it('locks Apply while the change is sent and hands the result to the page', async () => {
    let finish: (value: unknown) => void = () => undefined
    api.bulkTickets.mockReturnValue(new Promise((resolve) => { finish = resolve }))
    const onResult = vi.fn()
    mount(onResult)
    const apply = screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement
    fireEvent.click(apply)
    expect(onResult).toHaveBeenCalledWith(null)
    await waitFor(() => expect(apply.disabled).toBe(true))
    expect(api.bulkTickets).toHaveBeenCalledWith('w', ['t1'], { type: 'PRIORITY', priority: null }, expect.any(String))
    const outcome = { total: 1, succeeded: 1, failed: 0, items: [] }
    finish(outcome)
    await waitFor(() => expect(onResult).toHaveBeenLastCalledWith(outcome))
    expect(apply.disabled).toBe(false)
  })

  it('says so when the request fails, and clears that on the next try', async () => {
    api.bulkTickets.mockRejectedValueOnce(new Error('offline'))
    mount(() => undefined)
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(await screen.findByText('Could not send the bulk change. Try again.')).toBeTruthy()
    api.bulkTickets.mockReturnValue(new Promise(() => undefined))
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    await waitFor(() => expect(screen.queryByText('Could not send the bulk change. Try again.')).toBeNull())
  })
})
