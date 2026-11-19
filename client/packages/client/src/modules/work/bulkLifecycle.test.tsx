import { cleanup, fireEvent, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ bulkTickets: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { BulkLifecycleBar } from './BulkLifecycleBar'
import { BULK_REQUEST_LIMIT, archivePlan, chunk, finishedSelection, runLifecycle } from './bulkLifecycle'
import { fixtureBoard, fixtureTicket } from './testFixtures'
import { renderWithProviders } from '../../test/renderWithProviders'

const ok = (ticketId: string) => ({ ticketId, key: ticketId.toUpperCase(), ok: true, code: null, message: null })
const plan = { ids: ['a'], unfinished: 0, openChildren: 0, noPermission: 0 }
const ids = (count: number) => Array.from({ length: count }, (_, index) => `t${String(index)}`)

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.bulkTickets.mockReset()
})
afterEach(cleanup)

describe('Bulk archive and trash as one operation', () => {
  it('splits a large selection into requests under the server limit and reports one result', async () => {
    api.bulkTickets.mockImplementation((_workspace: string, batch: string[]) =>
      Promise.resolve({ total: batch.length, succeeded: batch.length, failed: 0, items: batch.map(ok) }),
    )
    const result = await runLifecycle('w', 'ARCHIVE', ids(250))
    expect(api.bulkTickets.mock.calls.map((call) => (call[1] as string[]).length)).toEqual([BULK_REQUEST_LIMIT, BULK_REQUEST_LIMIT, 50])
    expect(api.bulkTickets.mock.calls.every((call) => (call[2] as { type: string }).type === 'ARCHIVE')).toBe(true)
    expect(result).toMatchObject({ total: 250, succeeded: 250, failed: 0 })
    expect(chunk([1, 2, 3], 2)).toEqual([[1, 2], [3]])
  })

  it('does not end as a success when a request fails: the rest are reported as not processed', async () => {
    api.bulkTickets
      .mockResolvedValueOnce({ total: 100, succeeded: 100, failed: 0, items: ids(100).map(ok) })
      .mockRejectedValueOnce(new Error('offline'))
    const result = await runLifecycle('w', 'TRASH', ids(250))
    expect(api.bulkTickets).toHaveBeenCalledTimes(2)
    expect(result.total).toBe(250)
    expect(result.succeeded).toBe(100)
    expect(result.failed).toBe(150)
    expect(result.items.filter((item) => item.code === 'not_processed')).toHaveLength(150)
  })

  it('asks the server to archive only finished work, and says why a reopened ticket was refused', async () => {
    api.bulkTickets.mockResolvedValue({ total: 2, succeeded: 1, failed: 1, items: [ok('a'), { ticketId: 'b', key: null, ok: false, code: 'not_finished', message: 'x' }] })
    await runLifecycle('w', 'ARCHIVE', ['a', 'b'])
    expect(api.bulkTickets.mock.calls[0]?.[2]).toEqual({ type: 'ARCHIVE', onlyFinished: true })
    api.bulkTickets.mockClear()
    // The trash is not part of this condition.
    await runLifecycle('w', 'TRASH', ['a'])
    expect(api.bulkTickets.mock.calls[0]?.[2]).toEqual({ type: 'TRASH' })
    renderWithProviders(
      <BulkLifecycleBar count={1} plan={{ ids: ['b'], unfinished: 0, openChildren: 0, noPermission: 0 }} scope="" running={null} last="ARCHIVE" names={new Map([['b', { key: 'PEN-3', title: 'Reopened' }]])} onRun={() => undefined} onClear={() => undefined} result={{ total: 2, succeeded: 1, failed: 1, items: [ok('a'), { ticketId: 'b', key: null, ok: false, code: 'not_finished', message: 'x' }] }} />,
    )
    expect(screen.getByText(/it is no longer Completed or Canceled \(it was reopened\)/)).toBeTruthy()
  })

  it('keeps a per-ticket refusal as a failure and counts only the processed ones', async () => {
    api.bulkTickets.mockResolvedValue({
      total: 2,
      succeeded: 1,
      failed: 1,
      items: [ok('a'), { ticketId: 'b', key: null, ok: false, code: 'open_children', message: 'x' }],
    })
    const result = await runLifecycle('w', 'ARCHIVE', ['a', 'b'])
    expect(result).toMatchObject({ total: 2, succeeded: 1, failed: 1 })
  })
})

describe('Chosen cards at the moment of the action', () => {
  it('keeps only the chosen Tickets that are still Completed or Canceled on the Board', () => {
    const board = fixtureBoard([
      fixtureTicket({ id: 'done', statusId: 's-done' }),
      fixtureTicket({ id: 'moved-on', statusId: 's-doing' }),
    ])
    // 'gone' left the Board; 'moved-on' was chosen while finished and has since been reopened.
    expect(finishedSelection(new Set(['done', 'moved-on', 'gone']), board)).toEqual(['done'])
    expect(finishedSelection(new Set(), board)).toEqual([])
  })
})

describe('Bulk lifecycle bar', () => {
  const names = new Map([['b', { key: 'PEN-2', title: 'Parent' }]])

  it('shows a partial run as a failure with the reason and offers a retry', () => {
    const onRun = vi.fn()
    renderWithProviders(
      <BulkLifecycleBar
        count={1}
        plan={plan}
        scope=""
        running={null}
        last="ARCHIVE"
        names={names}
        onRun={onRun}
        onClear={() => undefined}
        result={{ total: 2, succeeded: 1, failed: 1, items: [ok('a'), { ticketId: 'b', key: null, ok: false, code: 'open_children', message: 'x' }] }}
      />,
    )
    expect(screen.getByRole('alert').textContent).toContain('1 of 2 archived')
    expect(screen.getByText(/It has unfinished child tickets/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Retry the failed ones' }))
    expect(onRun).toHaveBeenCalledWith('ARCHIVE')
  })

  it('says all were processed only when none failed, and locks the buttons while running', () => {
    renderWithProviders(
      <BulkLifecycleBar count={0} plan={plan} scope="" running={null} last="TRASH" names={names} onRun={() => undefined} onClear={() => undefined} result={{ total: 2, succeeded: 2, failed: 0, items: [ok('a'), ok('b')] }} />,
    )
    expect(screen.getByRole('status').textContent).toContain('2 moved to the trash')
    expect(screen.queryByRole('button', { name: 'Retry the failed ones' })).toBeNull()
    // Nothing is chosen any more: no actions, just the result and a way to close it.
    expect(screen.queryByRole('button', { name: /^Archive/ })).toBeNull()
    expect(screen.getByRole('button', { name: 'Dismiss' })).toBeTruthy()
    cleanup()
    renderWithProviders(<BulkLifecycleBar count={3} plan={plan} scope="" running={{ done: 100, total: 250 }} last="TRASH" names={names} onRun={() => undefined} onClear={() => undefined} result={null} />)
    expect((screen.getByRole('button', { name: /^Archive/ }) as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText('Processing 100/250…')).toBeTruthy()
  })
})

describe('What an Archive would take', () => {
  const child = (open: number) => ({ total: open, done: 0, canceled: 0 })
  const t = (id: string, finished: boolean, open = 0, projectId = 'p') => ({ id, finished, projectId, childSummary: child(open) })

  it('counts only finished Tickets without unfinished children in a Project the person administers, with the reasons for the rest', () => {
    const plan = archivePlan([t('a', true), t('b', true, 2), t('c', false), t('d', true, 0, 'other')], (project) => project === 'p')
    expect(plan).toEqual({ ids: ['a'], unfinished: 1, openChildren: 1, noPermission: 1 })
  })

  it('names the reasons and the scope in the bar and archives only what can be archived', () => {
    const onRun = vi.fn()
    renderWithProviders(
      <BulkLifecycleBar count={3} plan={{ ids: ['a', 'b'], unfinished: 0, openChildren: 1, noPermission: 0 }} scope="Chosen from the Board" running={null} last={null} names={new Map()} onRun={onRun} onClear={() => undefined} result={null} />,
    )
    expect(screen.getByText(/1 will not be archived: 1 with unfinished child tickets/)).toBeTruthy()
    expect(screen.getByText('Chosen from the Board')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Archive 2' }))
    expect(onRun).toHaveBeenCalledWith('ARCHIVE')
  })
})
