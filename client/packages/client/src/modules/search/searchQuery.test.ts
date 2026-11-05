import { describe, expect, it } from 'vitest'
import { hasToken, replaceToken, setKey, toggleToken, tokens, valueOf } from './searchQuery'

describe('search text helpers', () => {
  it('keeps quoted values together', () => {
    expect(tokens('배포 in:"my room" "release notes"')).toEqual(['배포', 'in:"my room"', '"release notes"'])
  })
  it('toggles a chip token on and off without touching the words', () => {
    const on = toggleToken('배포', 'assignee:me')
    expect(on).toBe('배포 assignee:me')
    expect(hasToken(on, 'ASSIGNEE:me')).toBe(true)
    expect(toggleToken(on, 'assignee:me')).toBe('배포')
  })
  it('sets one value per key', () => {
    expect(setKey('a status:done b', 'status', 'in-progress')).toBe('a b status:in-progress')
    expect(setKey('a status:done', 'status', null)).toBe('a')
    expect(valueOf('x project:PEN', 'project')).toBe('PEN')
    expect(setKey('x', 'in', 'my room')).toBe('x in:"my room"')
  })
  it('replaces an ambiguous token with its id', () => {
    expect(replaceToken('from:kim a', 'from:kim', 'from:7d0e')).toBe('from:7d0e a')
  })
})
