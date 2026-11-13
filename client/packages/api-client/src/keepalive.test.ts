import { afterEach, describe, expect, it, vi } from 'vitest'
import { createPenfiiClient } from './index'

afterEach(() => vi.unstubAllGlobals())

describe('closing an input session', () => {
  it('asks the browser to finish the request even if the page is left right after', async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })))
    vi.stubGlobal('fetch', fetchMock)
    await createPenfiiClient().closeDraft('CHANNEL_COMPOSE', 'c1', 'g1')
    const init = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1]
    expect(init.keepalive).toBe(true)
    expect(init.method).toBe('DELETE')
  })

  it('does not ask for it on an ordinary read', async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response('null', { status: 200 })))
    vi.stubGlobal('fetch', fetchMock)
    await createPenfiiClient().getDraft('CHANNEL_COMPOSE', 'c1')
    const init = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1]
    expect(init.keepalive).toBeUndefined()
  })
})
