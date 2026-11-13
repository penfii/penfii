// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError, endSession } from '@penfii/api-client'
import type { RichTextDoc } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getDraft: vi.fn(), saveDraft: vi.fn(), closeDraft: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { createDraftController } from './draftController'
import { newInputSession } from './draftSession'

const doc = (text: string): RichTextDoc => ({ type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] }) as RichTextDoc
const blank = { type: 'doc', content: [{ type: 'paragraph' }] } as RichTextDoc
const closed = () => new ApiError(409, 'draft_closed', 'over')
const saved = (version: number, generation = 'g') => ({ id: 'd1', version, generation, payload: { schemaVersion: 1, body: doc('x') } })

function box() {
  const filled: RichTextDoc[] = []
  const controller = createDraftController({ kind: 'CHANNEL_COMPOSE', contextId: 'c1', fill: (body) => filled.push(body) })
  return { controller, filled }
}
const settle = () => vi.advanceTimersByTimeAsync(0)

beforeEach(() => {
  vi.useFakeTimers()
  api.getDraft.mockResolvedValue(null)
  api.saveDraft.mockImplementation(() => Promise.resolve(saved(1)))
  api.closeDraft.mockResolvedValue(undefined)
})
afterEach(() => {
  vi.useRealTimers()
  vi.clearAllMocks()
})

describe('input sessions', () => {
  it('are uuid v7 values that sort in the order they were made, also within one millisecond and when the clock goes back', () => {
    const made = [newInputSession(1_000), newInputSession(1_000), newInputSession(1_000), newInputSession(999), newInputSession(2_000)]
    for (const value of made) expect(value).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
    expect([...made].sort()).toEqual(made)
    expect(new Set(made).size).toBe(made.length)
  })
})

describe('saving what is typed', () => {
  it('saves a moment after typing stops, naming one session for the whole time the box holds text', async () => {
    const { controller } = box()
    controller.update(doc('he'))
    controller.update(doc('hello'))
    await vi.advanceTimersByTimeAsync(799)
    expect(api.saveDraft).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
    const first = api.saveDraft.mock.calls[0]?.[0] as { generation: string; expectedVersion?: number }
    expect(first.expectedVersion).toBeUndefined()
    controller.update(doc('hello world'))
    await vi.advanceTimersByTimeAsync(800)
    const second = api.saveDraft.mock.calls[1]?.[0] as { generation: string; expectedVersion?: number }
    expect(second.generation).toBe(first.generation)
    expect(second.expectedVersion).toBe(1)
  })

  it('starts a new session for the next message after the box was emptied', async () => {
    const { controller } = box()
    controller.update(doc('first'))
    await vi.advanceTimersByTimeAsync(800)
    controller.update(blank)
    controller.update(doc('second'))
    await vi.advanceTimersByTimeAsync(800)
    const [one, two] = api.saveDraft.mock.calls.map((call) => (call[0] as { generation: string }).generation)
    expect(two).not.toBe(one)
    expect((two as string) > (one as string)).toBe(true)
  })
})

describe('sending while a save is on its way', () => {
  it('ends the session at once, without waiting for the first save to be answered, and the late save changes nothing here', async () => {
    let answer: (value: unknown) => void = () => undefined
    api.saveDraft.mockImplementationOnce(() => new Promise((resolve) => { answer = resolve }))
    const { controller } = box()
    controller.update(doc('hello'))
    await vi.advanceTimersByTimeAsync(800)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
    const generation = (api.saveDraft.mock.calls[0]?.[0] as { generation: string }).generation
    // Enter: the box is emptied while the first save (no version, no id yet) has not been answered.
    controller.update(blank)
    expect(api.closeDraft).toHaveBeenCalledWith('CHANNEL_COMPOSE', 'c1', generation)
    await act(async () => answer(saved(1, generation)))
    // The answer of the ended session does not start another save, nor a new session.
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
    expect(api.closeDraft).toHaveBeenCalledTimes(1)
  })

  it('ends the session at once also when a save that already has a version is in flight', async () => {
    const { controller } = box()
    controller.update(doc('one'))
    await vi.advanceTimersByTimeAsync(800)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
    let answer: (value: unknown) => void = () => undefined
    api.saveDraft.mockImplementationOnce(() => new Promise((resolve) => { answer = resolve }))
    controller.update(doc('one two'))
    await vi.advanceTimersByTimeAsync(800)
    expect(api.saveDraft).toHaveBeenCalledTimes(2)
    const generation = (api.saveDraft.mock.calls[1]?.[0] as { generation: string }).generation
    controller.update(blank)
    expect(api.closeDraft).toHaveBeenCalledWith('CHANNEL_COMPOSE', 'c1', generation)
    answer(saved(2, generation))
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).toHaveBeenCalledTimes(2)
  })

  it('keeps the newest text when the answers come in another order than the requests', async () => {
    let first: (value: unknown) => void = () => undefined
    api.saveDraft.mockImplementationOnce(() => new Promise((resolve) => { first = resolve }))
    const { controller } = box()
    controller.update(doc('one'))
    await vi.advanceTimersByTimeAsync(800)
    // Typing goes on while the first save is unanswered; its answer arrives after the next save was asked for.
    controller.update(doc('one two'))
    await vi.advanceTimersByTimeAsync(800)
    first(saved(1))
    await vi.advanceTimersByTimeAsync(5000)
    const bodies = api.saveDraft.mock.calls.map((call) => JSON.stringify((call[0] as { payload: unknown }).payload))
    expect(bodies.at(-1)).toContain('one two')
    // The newest save builds on the version the first answer gave.
    expect((api.saveDraft.mock.calls.at(-1)?.[0] as { expectedVersion?: number }).expectedVersion).toBe(1)
  })

  it('does not start a new session when the server says the ended session\'s late save is refused', async () => {
    let refuse: (reason: unknown) => void = () => undefined
    api.saveDraft.mockImplementationOnce(() => new Promise((_resolve, reject) => { refuse = reject }))
    const { controller } = box()
    controller.update(doc('hello'))
    await vi.advanceTimersByTimeAsync(800)
    controller.update(blank)
    refuse(closed())
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
  })

  it('tells the server nothing when no save was ever asked for (sent within the delay)', async () => {
    const { controller } = box()
    controller.update(doc('quick'))
    controller.update(blank)
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).not.toHaveBeenCalled()
    expect(api.closeDraft).not.toHaveBeenCalled()
  })
})

describe('a session that was ended elsewhere', () => {
  it('starts a newer session for text the person is still typing here', async () => {
    api.saveDraft.mockRejectedValueOnce(closed()).mockResolvedValueOnce(saved(7))
    const { controller } = box()
    controller.update(doc('typed here'))
    await vi.advanceTimersByTimeAsync(800)
    await settle()
    expect(api.saveDraft).toHaveBeenCalledTimes(2)
    const [old, fresh] = api.saveDraft.mock.calls.map((call) => (call[0] as { generation: string }).generation)
    expect((fresh as string) > (old as string)).toBe(true)
    expect((api.saveDraft.mock.calls[1]?.[0] as { expectedVersion?: number }).expectedVersion).toBeUndefined()
  })

  it('takes the stored version from another tab that saved under the same session and goes on', async () => {
    api.saveDraft.mockRejectedValueOnce(new ApiError(409, 'draft_conflict', 'changed', { currentVersion: 4 })).mockResolvedValueOnce(saved(5))
    const { controller } = box()
    controller.update(doc('mine'))
    await vi.advanceTimersByTimeAsync(800)
    await settle()
    expect((api.saveDraft.mock.calls[1]?.[0] as { expectedVersion?: number }).expectedVersion).toBe(4)
  })
})

describe('leaving the box', () => {
  it('saves what is waiting for the delay when the box goes away', async () => {
    const { controller } = box()
    controller.update(doc('typed just before switching'))
    controller.dispose()
    await settle()
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).toHaveBeenCalledTimes(1)
  })

  it('saves nothing for a box that was just sent or is empty', async () => {
    const { controller } = box()
    controller.update(doc('sent'))
    controller.update(blank)
    controller.dispose()
    await settle()
    expect(api.saveDraft).not.toHaveBeenCalled()
  })

  it('does not send what was typed before the session of the person ended', async () => {
    const { controller } = box()
    controller.update(doc('belongs to the person who signed out'))
    endSession()
    controller.dispose()
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).not.toHaveBeenCalled()
  })

  it('ignores a box that goes on being told about changes after it was disposed', async () => {
    const { controller } = box()
    controller.dispose()
    controller.update(doc('late'))
    await vi.advanceTimersByTimeAsync(5000)
    expect(api.saveDraft).not.toHaveBeenCalled()
  })
})

describe('the stored draft', () => {
  it('fills the box and goes on with the stored session, saving only what changed', async () => {
    api.getDraft.mockResolvedValue({ id: 'd9', generation: 'stored-g', version: 3, payload: { schemaVersion: 1, body: doc('stored') } })
    const { controller, filled } = box()
    await settle()
    expect(filled).toHaveLength(1)
    controller.update(filled[0] as RichTextDoc)
    await vi.advanceTimersByTimeAsync(2000)
    expect(api.saveDraft).not.toHaveBeenCalled()
    controller.update(doc('stored and more'))
    await vi.advanceTimersByTimeAsync(800)
    expect(api.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ generation: 'stored-g', expectedVersion: 3 }))
  })

  it('does not overwrite text typed before the stored draft arrived', async () => {
    let arrive: (value: unknown) => void = () => undefined
    api.getDraft.mockImplementation(() => new Promise((resolve) => { arrive = resolve }))
    const { controller, filled } = box()
    controller.update(doc('typed first'))
    await act(async () => arrive({ id: 'd', generation: 'old-g', version: 1, payload: { schemaVersion: 1, body: doc('older') } }))
    expect(filled).toHaveLength(0)
  })
})

async function act(run: () => Promise<void>) {
  await run()
  await vi.advanceTimersByTimeAsync(0)
}
