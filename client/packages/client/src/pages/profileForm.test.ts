import { describe, expect, it } from 'vitest'
import type { Me } from '@penfii/contracts'
import { changedGroups, mergeLatest, payloadFor, takeLatest, valuesFromMe, type ProfileValues } from './profileForm'

const me = (patch: Partial<Me> = {}): Me => ({
  id: 'u1', email: 'a@x', displayName: 'Alice', locale: null, timeZone: 'Asia/Seoul', jobTitle: null, statusEmoji: null, statusText: null, statusClearAt: null, hasAvatar: false, ...patch,
})
const base = valuesFromMe(me(), 'UTC')
const edited = (patch: Partial<ProfileValues>): ProfileValues => ({ ...base, ...patch })

describe('what counts as a change', () => {
  it('ignores surrounding spaces and counts the status as one item', () => {
    expect(changedGroups(edited({ displayName: ' Alice ' }), base)).toEqual([])
    expect(changedGroups(edited({ displayName: 'Alicia', jobTitle: 'Dev' }), base)).toEqual(['displayName', 'jobTitle'])
    expect(changedGroups(edited({ statusEmoji: '🏠', statusText: 'Home', statusClear: '2026-12-01T10:00' }), base)).toEqual(['status'])
  })

  it('is no change again once a value is typed back', () => {
    expect(changedGroups(edited({ jobTitle: 'Dev' }), base)).toEqual(['jobTitle'])
    expect(changedGroups(edited({ jobTitle: '' }), base)).toEqual([])
  })
})

describe('a new profile from the server', () => {
  const later = valuesFromMe(me({ statusEmoji: '🍱', statusText: 'Lunch' }), 'UTC')

  it('changes the items the person left alone and keeps the one being edited', () => {
    const typed = edited({ displayName: 'Alicia' })
    const merged = mergeLatest(typed, base, later, {}, {})
    expect(merged.form.displayName).toBe('Alicia')
    expect([merged.form.statusEmoji, merged.form.statusText]).toEqual(['🍱', 'Lunch'])
    expect(merged.conflicts).toEqual({})
  })

  it('notes a conflict when an edited item also changed elsewhere, and keeps the typed value', () => {
    const typed = edited({ statusText: 'Mine', statusEmoji: '🏠' })
    const merged = mergeLatest(typed, base, later, {}, {})
    expect(merged.form.statusText).toBe('Mine')
    expect(Object.keys(merged.conflicts)).toEqual(['status'])
    expect(merged.conflicts.status?.latest.statusText).toBe('Lunch')
    expect(merged.conflicts.status?.mine.statusText).toBe('Mine')
  })

  it('is no conflict when the server now holds what was typed', () => {
    const typed = edited({ statusEmoji: '🍱', statusText: 'Lunch' })
    expect(mergeLatest(typed, base, later, {}, {}).conflicts).toEqual({})
  })

  it('does not ask again about a server value the person already chose to override', () => {
    const typed = edited({ statusText: 'Mine' })
    const first = mergeLatest(typed, base, later, {}, {})
    expect(Object.keys(first.conflicts)).toEqual(['status'])
    const again = mergeLatest(first.form, later, later, first.conflicts, { status: JSON.stringify(['🍱', 'Lunch', '']) })
    expect(again.conflicts).toEqual({})
    // A different server value is a new question.
    const newer = valuesFromMe(me({ statusEmoji: '🌴', statusText: 'Away' }), 'UTC')
    expect(Object.keys(mergeLatest(first.form, later, newer, {}, { status: JSON.stringify(['🍱', 'Lunch', '']) }).conflicts)).toEqual(['status'])
  })

  it('keeps an open conflict while the server value stays the same', () => {
    const typed = edited({ statusText: 'Mine' })
    const first = mergeLatest(typed, base, later, {}, {})
    expect(Object.keys(mergeLatest(first.form, later, later, first.conflicts, {}).conflicts)).toEqual(['status'])
  })

  it('takes the latest value for one item', () => {
    const typed = edited({ statusText: 'Mine', jobTitle: 'Dev' })
    const taken = takeLatest(typed, later, 'status')
    expect(taken.statusText).toBe('Lunch')
    expect(taken.jobTitle).toBe('Dev')
  })
})

describe('what is sent', () => {
  it('sends typed items and the latest known value of every other item', () => {
    const latest = valuesFromMe(me({ statusEmoji: '🍱', statusText: 'Lunch', jobTitle: 'Lead' }), 'UTC')
    const body = payloadFor(edited({ displayName: ' Alicia ', jobTitle: 'Old' }), edited({ jobTitle: 'Old' }))
    expect(body).toMatchObject({ displayName: 'Alicia', jobTitle: 'Old', timeZone: 'Asia/Seoul', statusEmoji: null, statusText: null, statusClearAt: null })
    // Against a newer base, the untouched status is the server's, not an old copy.
    const typed = edited({ displayName: 'Alicia', jobTitle: '' })
    const sent = payloadFor(mergeLatest(typed, base, latest, {}, {}).form, latest)
    expect(sent).toMatchObject({ displayName: 'Alicia', jobTitle: 'Lead', statusEmoji: '🍱', statusText: 'Lunch' })
  })
})
