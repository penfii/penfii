import { describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ listChanges: vi.fn(), listMessages: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { loadNewestOverlap, planTimelineSync, readAllChanges } from './realtime'

describe('timeline sync plan', () => {
  it('names every loaded message a burst changed outside the newest pages', () => {
    const plan = planTimelineSync(
      [{ id: 'a' }, { id: 'b' }, { id: 'c' }],
      [{ id: 'c' }],
      [
        { messageId: 'a', changeKind: 'message.edited' },
        { messageId: 'b', changeKind: 'message.deleted' },
        { messageId: 'c', changeKind: 'message.edited' },
      ],
    )
    expect(plan.refreshLoadedIds).toEqual(['a', 'b'])
    expect(plan.refreshOlderReplyCounts).toBe(false)
  })

  it('refreshes older reply counts when a create is not a loaded root message', () => {
    const plan = planTimelineSync(
      [{ id: 'root' }],
      [{ id: 'root' }],
      [{ messageId: 'reply', changeKind: 'message.created' }],
    )
    expect(plan.refreshLoadedIds).toEqual([])
    expect(plan.refreshOlderReplyCounts).toBe(true)
  })

  it('leaves an unloaded edit alone', () => {
    const plan = planTimelineSync(
      [{ id: 'root' }],
      [{ id: 'root' }],
      [{ messageId: 'ancient', changeKind: 'message.edited' }],
    )
    expect(plan).toEqual({ refreshLoadedIds: [], refreshOlderReplyCounts: false })
  })
})

describe('change catch-up', () => {
  it('keeps every page, not only the last cursor', async () => {
    const first = Array.from({ length: 50 }, (_, index) => ({
      conversationId: 'c',
      changeSeq: index + 2,
      messageId: `m${String(index)}`,
      messageVersion: 1,
      changeKind: 'message.created',
    }))
    api.listChanges.mockResolvedValueOnce(first).mockResolvedValueOnce([
      { conversationId: 'c', changeSeq: 52, messageId: 'tail', messageVersion: 1, changeKind: 'message.deleted' },
    ])
    const changes = await readAllChanges('c', 1)
    expect(changes.map((change) => change.changeSeq)).toEqual([...first.map((change) => change.changeSeq), 52])
    expect(api.listChanges).toHaveBeenNthCalledWith(1, 'c', 1)
    expect(api.listChanges).toHaveBeenNthCalledWith(2, 'c', 51)
  })
})

describe('newest overlap', () => {
  it('walks one page older when a burst does not meet the loaded window', async () => {
    const loaded = [{ id: 'old', streamSeq: 1 }]
    api.listMessages
      .mockResolvedValueOnce(Array.from({ length: 50 }, (_, index) => ({ id: `n${String(index + 60)}`, streamSeq: index + 60 })))
      .mockResolvedValueOnce([{ id: 'old', streamSeq: 1 }, { id: 'n59', streamSeq: 59 }])
    const fetched = await loadNewestOverlap('c', loaded as never)
    expect(api.listMessages).toHaveBeenCalledTimes(2)
    expect(fetched.some((message) => message.id === 'old')).toBe(true)
    expect(fetched.some((message) => message.id === 'n60')).toBe(true)
  })
})
