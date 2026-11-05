import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../../queryKeys'
import { useUserSocket } from './useUserSocket'

class FakeSocket {
  static last: FakeSocket | null = null
  onmessage: ((event: { data: unknown }) => void) | null = null
  onclose: (() => void) | null = null
  constructor(readonly url: string) {
    FakeSocket.last = this
  }
  close() {}
  say(frame: object) {
    this.onmessage?.({ data: JSON.stringify(frame) })
  }
}

function Probe() {
  useUserSocket()
  return null
}

function open() {
  const client = new QueryClient()
  const spy = vi.spyOn(client, 'invalidateQueries')
  render(
    <QueryClientProvider client={client}>
      <Probe />
    </QueryClientProvider>,
  )
  return { spy, socket: async () => { await waitFor(() => expect(FakeSocket.last).not.toBeNull()); return FakeSocket.last as FakeSocket } }
}

beforeEach(() => {
  FakeSocket.last = null
  vi.stubGlobal('WebSocket', FakeSocket)
})
afterEach(() => {
  cleanup()
})

describe('the signed-in person’s socket', () => {
  it('reads the channel list again when it hears the channels changed', async () => {
    const { spy, socket } = open()
    ;(await socket()).say({ v: 1, type: 'space.changed' })
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.all.space })
    expect(spy).not.toHaveBeenCalledWith({ queryKey: queryKeys.all.inbox })
  })

  it('still reads the Inbox on inbox.changed, and leaves the channel list alone', async () => {
    const { spy, socket } = open()
    ;(await socket()).say({ v: 1, type: 'inbox.changed' })
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.all.inbox })
    expect(spy).not.toHaveBeenCalledWith({ queryKey: queryKeys.all.space })
  })

  it('reads both lists again when the socket opens again, since notices sent meanwhile were missed', async () => {
    const { spy, socket } = open()
    const first = await socket()
    first.say({ v: 1, type: 'user.ready' })
    expect(spy, 'the first connection is the page loading, not a catch-up').not.toHaveBeenCalled()
    first.onclose?.()
    await waitFor(() => expect(FakeSocket.last).not.toBe(first), { timeout: 5000 })
    ;(FakeSocket.last as FakeSocket).say({ v: 1, type: 'user.ready' })
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.all.space })
    expect(spy).toHaveBeenCalledWith({ queryKey: queryKeys.all.inbox })
  })
})
