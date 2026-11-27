import { afterEach, describe, expect, it, vi } from 'vitest'
import { BACKGROUND_AFTER_MS, createPenfiiClient, noteUserInput, watchUserInput } from './index'

afterEach(() => vi.unstubAllGlobals())

async function headersOfNextRequest(): Promise<Headers> {
  const fetchMock = vi.fn(() => Promise.resolve(new Response('{"features":[],"limits":{}}', { status: 200 })))
  vi.stubGlobal('fetch', fetchMock)
  await createPenfiiClient().serverInfo()
  return (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].headers as Headers
}

describe('requests the page makes by itself', () => {
  it('are the person\'s while the page was used lately and background traffic after a quiet spell', async () => {
    noteUserInput()
    expect((await headersOfNextRequest()).has('X-Penfii-Background')).toBe(false)
    noteUserInput(Date.now() - BACKGROUND_AFTER_MS - 1000)
    expect((await headersOfNextRequest()).get('X-Penfii-Background')).toBe('1')
    // Using the page again makes the next request the person's.
    noteUserInput()
    expect((await headersOfNextRequest()).has('X-Penfii-Background')).toBe(false)
  })

  it('notes pointer, key, touch and wheel input on the target, and stops when asked', async () => {
    const target = new EventTarget()
    const stop = watchUserInput(target)
    noteUserInput(0)
    expect((await headersOfNextRequest()).get('X-Penfii-Background')).toBe('1')
    for (const name of ['pointerdown', 'keydown', 'touchstart', 'wheel']) {
      noteUserInput(0)
      target.dispatchEvent(new Event(name))
      expect((await headersOfNextRequest()).has('X-Penfii-Background')).toBe(false)
    }
    stop()
    noteUserInput(0)
    target.dispatchEvent(new Event('keydown'))
    expect((await headersOfNextRequest()).get('X-Penfii-Background')).toBe('1')
  })
})
