import { QueryClient } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../../queryKeys'
import { refreshTicket } from './refresh'

describe('refreshTicket', () => {
  it('also marks the Project Timeline list stale, so a saved schedule shows without a reload', async () => {
    const client = new QueryClient()
    const spy = vi.spyOn(client, 'invalidateQueries')
    await refreshTicket(client, 't1', 'p1')
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.projectSchedule('p1') })
  })

  it("marks the Ticket's comments stale, so a teammate's new comment shows in an open panel", async () => {
    const client = new QueryClient()
    const spy = vi.spyOn(client, 'invalidateQueries')
    await refreshTicket(client, 't1', 'p1')
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.ticketComments('t1') })
  })

  it("marks the Ticket's body files stale, so files a teammate adds or removes show in an open panel", async () => {
    const client = new QueryClient()
    const spy = vi.spyOn(client, 'invalidateQueries')
    await refreshTicket(client, 't1', 'p1')
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.ticketAttachments('t1') })
  })
})
