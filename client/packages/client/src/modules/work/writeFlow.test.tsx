// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { createPenfiiClient } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  restoreTicketFromTrash: vi.fn(),
  updateTicket: vi.fn(),
  getTicket: vi.fn<(id: string) => Promise<unknown>>(),
}))
vi.mock('../../api', () => ({ api }))

import { queries } from '../../queries'
import { RESTORE_CONCURRENCY, restoreFromTrash } from './bulkLifecycle'
import { useTicketEditor } from './useTicketEditor'
import { fixtureTicket } from './testFixtures'

beforeEach(() => vi.clearAllMocks())
afterEach(() => vi.unstubAllGlobals())

describe('Undo of a bulk trash', () => {
  it('puts three hundred Tickets back without more than a handful of requests at a time', async () => {
    let running = 0
    let peak = 0
    api.restoreTicketFromTrash.mockImplementation(() => {
      running += 1
      peak = Math.max(peak, running)
      return new Promise<void>((resolve) => setTimeout(resolve, 0)).then(() => { running -= 1 })
    })
    const ids = Array.from({ length: 300 }, (_, index) => `t${String(index)}`)
    const result = await restoreFromTrash(ids)
    expect(result).toEqual({ ok: 300, failed: 0 })
    expect(api.restoreTicketFromTrash).toHaveBeenCalledTimes(300)
    expect(peak).toBeLessThanOrEqual(RESTORE_CONCURRENCY)
  })

  it('counts the ones that did not come back and goes on with the rest', async () => {
    api.restoreTicketFromTrash.mockImplementation((id: string) => (id === 't1' ? Promise.reject(new Error('no')) : Promise.resolve()))
    expect(await restoreFromTrash(['t0', 't1', 't2'])).toEqual({ ok: 2, failed: 1 })
  })
})

describe('saving a Ticket', () => {
  it('frees the controls when the server has answered, not when the lists have been read again', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    // A Board that is slow to come back: if saving waited for it, `busy` would stay true.
    queryClient.setQueryDefaults(['project-board', 'p'], { queryFn: () => new Promise(() => undefined) })
    void queryClient.prefetchQuery({ queryKey: ['project-board', 'p'] })
    const ticket = fixtureTicket({ id: 't1', projectId: 'p', revisionNo: 1 })
    api.updateTicket.mockResolvedValue({ ...ticket, title: 'New', revisionNo: 2 })
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    const { result } = renderHook(() => useTicketEditor(ticket), { wrapper })
    let saved = false
    await act(async () => {
      saved = await result.current.save({ title: 'New' })
    })
    expect(saved).toBe(true)
    expect(result.current.busy).toBe(false)
    expect(queryClient.getQueryData(['ticket', 't1'])).toMatchObject({ title: 'New', revisionNo: 2 })
  })
})

describe('a query that is no longer wanted', () => {
  it('is aborted on the wire when it is cancelled', async () => {
    let signal: AbortSignal | undefined
    vi.stubGlobal('fetch', vi.fn((_url: string, init: RequestInit): Promise<Response> => {
      signal = init.signal ?? undefined
      return new Promise<Response>(() => undefined)
    }))
    // The real client here: the module under test is `queries` and the signal it hands over.
    vi.mocked(api.getTicket).mockImplementation((id: string) => createPenfiiClient().getTicket(id))
    const queryClient = new QueryClient()
    void queryClient.fetchQuery(queries.ticket('t1')).catch(() => undefined)
    await Promise.resolve()
    expect(signal).toBeDefined()
    expect(signal?.aborted).toBe(false)
    await queryClient.cancelQueries({ queryKey: ['ticket', 't1'] })
    expect(signal?.aborted).toBe(true)
  })
})
