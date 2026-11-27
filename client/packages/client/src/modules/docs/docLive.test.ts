// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { QueryClient } from '@tanstack/react-query'
import { ApiError } from '@penfii/api-client'
import type { DocPage } from '@penfii/contracts'
import { queryKeys } from '../../queryKeys'

type Options = { onFrame: (data: string) => void; onStop?: (status: number) => void; path: () => string; probe?: () => Promise<unknown> }

const hoisted = vi.hoisted(() => ({
  options: null as null | Options,
  close: vi.fn(),
  getDoc: vi.fn(),
}))

vi.mock('../../realtime/liveSocket', () => ({
  openLiveSocket: (options: Options) => {
    hoisted.options = options
    return { close: hoisted.close }
  },
}))
vi.mock('../../api', () => ({ api: { getDoc: hoisted.getDoc } }))

import { startDocLive } from './docLive'

const page = (over: Partial<DocPage> = {}): DocPage => ({ id: 'd1', revisionNo: 1, title: 'T', latestVersionNo: null, fullWidth: false, ...over }) as unknown as DocPage
const frame = (type: string, docId = 'd1') => JSON.stringify({ v: 1, type, docId })
const notFound = () => new ApiError(404, 'not_found', 'gone', undefined)

let client: QueryClient
let stop: () => void

async function open(initial: DocPage = page()) {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  hoisted.getDoc.mockResolvedValue(initial)
  // The page's own query: what it read, and how it reads again.
  client.setQueryDefaults(queryKeys.doc('d1'), { queryFn: () => hoisted.getDoc('d1') as Promise<DocPage> })
  client.setQueryData(queryKeys.doc('d1'), initial)
  stop = startDocLive(client, 'd1', 'w1')
  return () => stop()
}

async function settle() {
  await vi.advanceTimersByTimeAsync(400)
}

beforeEach(() => {
  vi.useFakeTimers()
  hoisted.getDoc.mockReset()
  hoisted.close.mockReset()
  hoisted.options = null
})
afterEach(() => vi.useRealTimers())

describe('startDocLive', () => {
  it('opens the socket of the page and probes with the page itself', async () => {
    const done = await open()
    expect(hoisted.options?.path()).toBe('/v1/ws/docs/d1')
    await hoisted.options?.probe?.()
    expect(hoisted.getDoc).toHaveBeenCalledWith('d1')
    hoisted.getDoc.mockClear()
    done()
    expect(hoisted.close).toHaveBeenCalled()
  })

  it('reads a burst of notices once', async () => {
    const done = await open()
    hoisted.getDoc.mockClear()
    for (let i = 0; i < 5; i += 1) hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(hoisted.getDoc).toHaveBeenCalledTimes(1)
    done()
  })

  it('reads once more for a notice that arrives while a read is under way, and not for more than that', async () => {
    const done = await open()
    hoisted.getDoc.mockClear()
    let release: (value: DocPage) => void = () => undefined
    hoisted.getDoc.mockImplementationOnce(() => new Promise<DocPage>((resolve) => { release = resolve }))
    hoisted.options?.onFrame(frame('doc.changed'))
    await vi.advanceTimersByTimeAsync(200)
    expect(hoisted.getDoc).toHaveBeenCalledTimes(1)
    // Three more while the first is still in flight.
    hoisted.options?.onFrame(frame('doc.changed'))
    hoisted.options?.onFrame(frame('doc.changed'))
    hoisted.options?.onFrame(frame('doc.changed'))
    await vi.advanceTimersByTimeAsync(200)
    expect(hoisted.getDoc).toHaveBeenCalledTimes(1)
    release(page({ revisionNo: 2 }))
    await settle()
    expect(hoisted.getDoc).toHaveBeenCalledTimes(2)
    await settle()
    expect(hoisted.getDoc).toHaveBeenCalledTimes(2)
    done()
  })

  it('reads when a connection says it is ready, and ignores other pages and malformed frames', async () => {
    const done = await open()
    hoisted.getDoc.mockClear()
    hoisted.options?.onFrame(frame('doc.changed', 'other'))
    hoisted.options?.onFrame('not json')
    hoisted.options?.onFrame(JSON.stringify({ v: 1, type: 'something.else', docId: 'd1' }))
    await settle()
    expect(hoisted.getDoc).not.toHaveBeenCalled()
    hoisted.options?.onFrame(frame('doc.ready'))
    await settle()
    expect(hoisted.getDoc).toHaveBeenCalledTimes(1)
    done()
  })

  it('refreshes the tree and the versions only when the revision, the title or the latest version changed', async () => {
    const done = await open()
    const invalidate = vi.spyOn(client, 'invalidateQueries')
    // The width changes: no revision, no title.
    hoisted.getDoc.mockResolvedValue(page({ fullWidth: true }))
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(client.getQueryData<DocPage>(queryKeys.doc('d1'))?.fullWidth).toBe(true)
    expect(invalidate).not.toHaveBeenCalled()
    // A Save by someone else.
    hoisted.getDoc.mockResolvedValue(page({ fullWidth: true, revisionNo: 2, title: 'New' }))
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    const keys = invalidate.mock.calls.map(([filters]) => JSON.stringify(filters?.queryKey))
    expect(keys).toContain(JSON.stringify(queryKeys.all.docTree('w1')))
    expect(keys).toContain(JSON.stringify(queryKeys.docVersions('d1')))
    // Who may read it changed: the tree shows that too.
    invalidate.mockClear()
    hoisted.getDoc.mockResolvedValue(page({ fullWidth: true, revisionNo: 2, title: 'New', visibility: 'RESTRICTED' } as Partial<DocPage>))
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(invalidate.mock.calls.map(([filters]) => JSON.stringify(filters?.queryKey))).toContain(JSON.stringify(queryKeys.all.docTree('w1')))
    done()
  })

  it('stops for good when the page can no longer be read', async () => {
    const done = await open()
    hoisted.getDoc.mockRejectedValue(notFound())
    hoisted.options?.onFrame(frame('doc.gone'))
    await settle()
    expect(hoisted.close).toHaveBeenCalled()
    expect(client.getQueryState(queryKeys.doc('d1'))?.status).toBe('error')
    hoisted.getDoc.mockClear()
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(hoisted.getDoc).not.toHaveBeenCalled()
    done()
  })

  it('closes the socket at once on "gone", and opens it again if the page turns out to be readable', async () => {
    const done = await open()
    hoisted.close.mockClear()
    hoisted.options?.onFrame(frame('doc.gone'))
    // Before the read has said anything: no reconnecting to a page that may answer 404.
    expect(hoisted.close).toHaveBeenCalledTimes(1)
    hoisted.options = null
    hoisted.getDoc.mockResolvedValue(page({ revisionNo: 2 }))
    await settle()
    // The read found the page: a new socket is opened for it.
    expect(hoisted.options).not.toBeNull()
    expect(client.getQueryData<DocPage>(queryKeys.doc('d1'))?.revisionNo).toBe(2)
    done()
  })

  it('keeps the page it has when a read fails for a passing reason, and reads again at the next notice', async () => {
    const done = await open()
    hoisted.getDoc.mockRejectedValueOnce(new ApiError(503, 'db_unavailable', 'down', undefined))
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(hoisted.close).not.toHaveBeenCalled()
    expect(client.getQueryData<DocPage>(queryKeys.doc('d1'))).toBeDefined()
    hoisted.getDoc.mockResolvedValue(page({ revisionNo: 3 }))
    hoisted.options?.onFrame(frame('doc.changed'))
    await settle()
    expect(client.getQueryData<DocPage>(queryKeys.doc('d1'))?.revisionNo).toBe(3)
    done()
  })

  it('reads when the socket stops because the probe was refused', async () => {
    const done = await open()
    hoisted.getDoc.mockClear()
    hoisted.getDoc.mockRejectedValue(notFound())
    hoisted.options?.onStop?.(404)
    await settle()
    expect(hoisted.getDoc).toHaveBeenCalledTimes(1)
    done()
  })

  it('does nothing after it is stopped', async () => {
    const done = await open()
    hoisted.getDoc.mockClear()
    hoisted.options?.onFrame(frame('doc.changed'))
    done()
    await settle()
    expect(hoisted.getDoc).not.toHaveBeenCalled()
  })
})
