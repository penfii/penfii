import { beforeEach, describe, expect, it } from 'vitest'
import { hasPrefs, prefsKey, readPrefs, writePrefs } from './displayPrefs'

beforeEach(() => localStorage.clear())

describe('A person\'s own display of a View', () => {
  it('is kept apart per person, place and View, so two people on one browser never share it', () => {
    writePrefs(prefsKey('ana', 'p1', 'v1'), { layout: 'TABLE', density: 'compact', columns: ['KEY', 'TITLE'] })
    expect(readPrefs(prefsKey('ana', 'p1', 'v1'))).toEqual({ layout: 'TABLE', density: 'compact', columns: ['KEY', 'TITLE'] })
    expect(hasPrefs(readPrefs(prefsKey('ben', 'p1', 'v1')))).toBe(false)
    expect(hasPrefs(readPrefs(prefsKey('ana', 'p1', 'v2')))).toBe(false)
    expect(hasPrefs(readPrefs(prefsKey('ana', 'p1', null)))).toBe(false)
  })

  it('ignores damaged or unknown stored values and clears when emptied', () => {
    localStorage.setItem(prefsKey('ana', 'p', null), '{"layout":"LIST","density":"wide","columns":["KEY","NOPE"]}')
    expect(readPrefs(prefsKey('ana', 'p', null))).toEqual({ columns: ['KEY'] })
    localStorage.setItem(prefsKey('ana', 'p', null), 'not json')
    expect(readPrefs(prefsKey('ana', 'p', null))).toEqual({})
    writePrefs(prefsKey('ana', 'p', null), {})
    expect(localStorage.getItem(prefsKey('ana', 'p', null))).toBeNull()
  })
})
