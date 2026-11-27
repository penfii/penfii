// @vitest-environment jsdom
import { cleanup, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { RichTextDoc } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getDraft: vi.fn(), saveDraft: vi.fn(), closeDraft: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { useServerDraft } from './useServerDraft'

const doc = (text: string): RichTextDoc => ({ type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] }) as RichTextDoc
const blank = { type: 'doc', content: [{ type: 'paragraph' }] } as RichTextDoc
const savedTexts = () => api.saveDraft.mock.calls.map(([request]) => [request.contextId, JSON.stringify(request.payload.body)])

beforeEach(() => {
  vi.useFakeTimers()
  api.getDraft.mockResolvedValue(null)
  api.saveDraft.mockImplementation(() => Promise.resolve({ id: 'd', version: 1, generation: 'g', payload: { schemaVersion: 1, body: blank } }))
  api.closeDraft.mockResolvedValue(undefined)
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.clearAllMocks()
})

describe('useServerDraft', () => {
  it('saves what was typed before the drafts of the box opened (the Space was still loading)', async () => {
    const setDoc = vi.fn()
    // The same object, as the page's state keeps it: the box does not change when the drafts open.
    const early = doc('typed early')
    const { rerender } = renderHook(({ context, text }: { context: string | null; text: RichTextDoc }) => useServerDraft('CHANNEL_COMPOSE', context, text, setDoc), {
      initialProps: { context: null as string | null, text: early },
    })
    await vi.advanceTimersByTimeAsync(2_000)
    expect(api.saveDraft).not.toHaveBeenCalled()
    rerender({ context: 'conv-a', text: early })
    await vi.advanceTimersByTimeAsync(2_000)
    expect(savedTexts()).toEqual([['conv-a', JSON.stringify(doc('typed early'))]])
  })

  it('never saves the text of the box that was left under the next conversation', async () => {
    const setDoc = vi.fn()
    const { rerender } = renderHook(({ context, text }: { context: string | null; text: RichTextDoc }) => useServerDraft('CHANNEL_COMPOSE', context, text, setDoc), {
      initialProps: { context: 'conv-a' as string | null, text: blank },
    })
    const forA = doc('for a')
    rerender({ context: 'conv-a', text: forA })
    // The switch: the new context arrives while the page still holds the old text; the page empties the box right after.
    rerender({ context: 'conv-b', text: forA })
    rerender({ context: 'conv-b', text: blank })
    await vi.advanceTimersByTimeAsync(2_000)
    expect(savedTexts()).toEqual([['conv-a', JSON.stringify(doc('for a'))]])
  })
})
