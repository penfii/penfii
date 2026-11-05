import { describe, expect, it } from 'vitest'
import { continuesGroup, displayedMinute } from './group'

const alice = 'alice'
const bob = 'bob'

function at(hour: number, minute: number, second: number): string {
  return new Date(2026, 9, 16, hour, minute, second).toISOString()
}

describe('continuesGroup', () => {
  it('groups one sender across the same displayed minute', () => {
    const first = { authorId: alice, createdAt: at(10, 30, 1) }
    expect(continuesGroup(first, { authorId: alice, createdAt: at(10, 30, 59) })).toBe(true)
    expect(displayedMinute(first.createdAt)).toBe('10:30')
  })

  it('starts a group when the displayed minute changes', () => {
    expect(continuesGroup(
      { authorId: alice, createdAt: at(10, 30, 59) },
      { authorId: alice, createdAt: at(10, 31, 0) },
    )).toBe(false)
    expect(displayedMinute(at(10, 31, 0))).toBe('10:31')
  })

  it('starts a group when the sender changes inside the same minute', () => {
    expect(continuesGroup(
      { authorId: alice, createdAt: at(10, 30, 10) },
      { authorId: bob, createdAt: at(10, 30, 20) },
    )).toBe(false)
    expect(continuesGroup(
      { authorId: bob, createdAt: at(10, 30, 20) },
      { authorId: alice, createdAt: at(10, 30, 40) },
    )).toBe(false)
  })

  it('does not group the same minute on another day', () => {
    const today = new Date(2026, 9, 16, 10, 30, 10).toISOString()
    const tomorrow = new Date(2026, 9, 17, 10, 30, 10).toISOString()
    expect(continuesGroup({ authorId: alice, createdAt: today }, { authorId: alice, createdAt: tomorrow })).toBe(false)
  })
})
