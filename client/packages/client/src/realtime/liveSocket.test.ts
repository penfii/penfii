// @vitest-environment jsdom
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FakeWebSocket, installFakeWebSocket } from '../test/fakeWebSocket'
import { backoffMs, openLiveSocket, type LiveSocketOptions } from './liveSocket'

let sockets: FakeWebSocket[]
const frames: string[] = []

function start(extra: Partial<LiveSocketOptions> = {}) {
  return openLiveSocket({ path: () => '/v1/ws/test', onFrame: (data) => frames.push(data), random: () => 1, ...extra })
}
/** The server refusing or dropping: the socket closes without ever answering. */
function failOnce() {
  const last = sockets[sockets.length - 1] as FakeWebSocket
  last.drop()
}

beforeEach(() => {
  vi.useFakeTimers()
  frames.length = 0
  sockets = installFakeWebSocket()
  vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(true)
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('backoff', () => {
  it('doubles from one second up to thirty, with jitter between zero and the step', () => {
    expect([1, 2, 3, 4, 5, 6, 7, 20].map((n) => backoffMs(n, () => 1))).toEqual([1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000])
    expect(backoffMs(3, () => 0)).toBe(0)
    expect(backoffMs(3, () => 0.5)).toBe(2000)
  })

  it('waits longer after every failure, and starts again once the server answers', () => {
    start()
    vi.advanceTimersByTime(0)
    expect(sockets).toHaveLength(1)
    failOnce()
    vi.advanceTimersByTime(999)
    expect(sockets).toHaveLength(1)
    vi.advanceTimersByTime(1)
    expect(sockets).toHaveLength(2)
    failOnce()
    vi.advanceTimersByTime(1999)
    expect(sockets).toHaveLength(2)
    vi.advanceTimersByTime(1)
    expect(sockets).toHaveLength(3)
    // The server answers: the count is cleared, so the next failure waits one second again.
    const third = sockets[2] as FakeWebSocket
    third.open()
    third.receive({ type: 'ready' })
    third.drop()
    vi.advanceTimersByTime(999)
    expect(sockets).toHaveLength(3)
    vi.advanceTimersByTime(1)
    expect(sockets).toHaveLength(4)
  })

  it('counts a connection that stayed open for ten seconds as working, though the server sent nothing', () => {
    start()
    vi.advanceTimersByTime(0)
    failOnce()
    vi.advanceTimersByTime(1000)
    failOnce()
    vi.advanceTimersByTime(2000)
    const socket = sockets[2] as FakeWebSocket
    socket.open()
    vi.advanceTimersByTime(10000)
    socket.drop()
    vi.advanceTimersByTime(1000)
    expect(sockets).toHaveLength(4)
  })
})

describe('what is sent and told', () => {
  it('asks for the hello again on every connection and says which one is a reconnection', () => {
    let position = 5
    const opened: boolean[] = []
    start({ hello: () => ({ v: 1, type: 'hello', afterChangeSeq: position }), onOpen: (reconnected) => opened.push(reconnected) })
    vi.advanceTimersByTime(0)
    const first = sockets[0] as FakeWebSocket
    first.open()
    first.receive('x')
    first.drop()
    position = 9
    vi.advanceTimersByTime(1000)
    const second = sockets[1] as FakeWebSocket
    second.open()
    expect(first.sent).toEqual([{ v: 1, type: 'hello', afterChangeSeq: 5 }])
    expect(second.sent).toEqual([{ v: 1, type: 'hello', afterChangeSeq: 9 }])
    expect(opened).toEqual([false, true])
  })

  it('hands text frames to the caller', () => {
    start()
    vi.advanceTimersByTime(0)
    const socket = sockets[0] as FakeWebSocket
    socket.open()
    socket.receive({ a: 1 })
    expect(frames).toEqual(['{"a":1}'])
  })
})

describe('closing on purpose', () => {
  it('closes the socket and leaves no timer behind', () => {
    const live = start()
    vi.advanceTimersByTime(0)
    const socket = sockets[0] as FakeWebSocket
    socket.open()
    live.close()
    expect(socket.readyState).toBe(FakeWebSocket.CLOSED)
    vi.advanceTimersByTime(120000)
    expect(sockets).toHaveLength(1)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('does not retry after a failure that came while a retry was waiting', () => {
    const live = start()
    vi.advanceTimersByTime(0)
    failOnce()
    live.close()
    vi.advanceTimersByTime(120000)
    expect(sockets).toHaveLength(1)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('opens nothing when it is closed in the turn it was started', () => {
    start().close()
    vi.advanceTimersByTime(1000)
    expect(sockets).toHaveLength(0)
  })
})

describe('a socket the server keeps refusing', () => {
  async function refuse(count: number) {
    for (let n = 0; n < count; n += 1) {
      failOnce()
      await vi.advanceTimersByTimeAsync(30000)
    }
  }

  it.each([401, 403, 404])('stops for good when the probe answers %i', async (status) => {
    const onStop = vi.fn()
    start({ probe: () => Promise.reject(new ApiError(status, 'x', 'x')), onStop })
    await vi.advanceTimersByTimeAsync(0)
    await refuse(3)
    expect(onStop).toHaveBeenCalledWith(status)
    const opened = sockets.length
    await vi.advanceTimersByTimeAsync(300000)
    expect(sockets).toHaveLength(opened)
  })

  it('keeps trying when the probe says the server is merely down', async () => {
    const onStop = vi.fn()
    start({ probe: () => Promise.reject(new ApiError(503, 'db_unavailable', 'x')), onStop })
    await vi.advanceTimersByTimeAsync(0)
    await refuse(3)
    const opened = sockets.length
    await vi.advanceTimersByTimeAsync(60000)
    expect(onStop).not.toHaveBeenCalled()
    expect(sockets.length).toBeGreaterThan(opened - 1)
  })

  it('does not ask the probe while connections are being answered', async () => {
    const probe = vi.fn(() => Promise.resolve())
    start({ probe })
    await vi.advanceTimersByTimeAsync(0)
    for (let n = 0; n < 5; n += 1) {
      const socket = sockets[sockets.length - 1] as FakeWebSocket
      socket.open()
      socket.receive('ready')
      socket.drop()
      await vi.advanceTimersByTimeAsync(2000)
    }
    expect(probe).not.toHaveBeenCalled()
  })
})

describe('offline and visibility', () => {
  it('makes no attempt while offline and connects at once when the browser is back online', () => {
    const online = vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(true)
    start()
    vi.advanceTimersByTime(0)
    online.mockReturnValue(false)
    failOnce()
    vi.advanceTimersByTime(120000)
    expect(sockets).toHaveLength(1)
    online.mockReturnValue(true)
    window.dispatchEvent(new Event('online'))
    expect(sockets).toHaveLength(2)
  })

  it('tries at once when a hidden tab becomes visible while it was waiting', () => {
    start()
    vi.advanceTimersByTime(0)
    failOnce()
    vi.advanceTimersByTime(500)
    expect(sockets).toHaveLength(1)
    document.dispatchEvent(new Event('visibilitychange'))
    expect(sockets).toHaveLength(2)
  })
})
