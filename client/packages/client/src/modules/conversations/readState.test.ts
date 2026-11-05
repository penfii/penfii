import { describe, expect, it } from 'vitest'
import { applyManualUnread, applyStaleRead, initialReadState, nextReadThroughSeq } from './readState'

describe('read state', () => {
  it('stale_read does not clear a local manual-unread flag', () => {
    let state = initialReadState({ generation: 3, readThroughSeq: 4 })
    state = applyManualUnread(state, 4)
    expect(state.manualUnread).toBe(true)
    state = applyStaleRead(state, 9)
    expect(state.manualUnread).toBe(true)
    expect(state.manualUnreadFromSeq).toBe(4)
    expect(state.generation).toBe(9)
    expect(state.readThroughSeq).toBe(4)
  })

  it('does not advance the read cursor across messages that were not shown', () => {
    expect(nextReadThroughSeq(0, [1, 2, 10])).toBe(2)
    expect(nextReadThroughSeq(0, [10])).toBeNull()
    expect(nextReadThroughSeq(9, [10, 11])).toBe(11)
    expect(nextReadThroughSeq(5, [8], [8])).toBeNull()
  })

  it('advances across thread-reply gaps when every root message is loaded', () => {
    expect(nextReadThroughSeq(3, [6, 7], [1, 2, 3, 7], {
      loadedSeqs: [1, 2, 3, 6, 7],
      hasOlder: false,
    })).toBe(7)
  })

  it('reads through the newest message once it is shown', () => {
    const loadedSeqs = Array.from({ length: 50 }, (_, index) => 71 + index)
    // 120 unread, only the newest page loaded and its tail on screen.
    expect(nextReadThroughSeq(0, [106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120], [], { loadedSeqs, hasOlder: true })).toBe(120)
  })

  it('does not jump an unread page that is not loaded', () => {
    expect(nextReadThroughSeq(0, [50], [], { loadedSeqs: [50, 51], hasOlder: true })).toBeNull()
    expect(nextReadThroughSeq(49, [50, 52], [], { loadedSeqs: [50, 52], hasOlder: true })).toBe(52)
  })
})
