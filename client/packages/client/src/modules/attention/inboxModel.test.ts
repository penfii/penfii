import { describe, expect, it } from 'vitest'
import type { InboxItem } from '@penfii/contracts'
import { emptyKind, groupByDay, inboxSearch, parseInboxLocation, readState, toggleType } from './inboxModel'

const parse = (q: string) => parseInboxLocation(new URLSearchParams(q))

describe('inbox location', () => {
  it('reads the View, the types and the unread Filter, and writes them back', () => {
    const location = parse('?view=archived&type=replies,mentions,bogus')
    expect(location).toEqual({ view: 'archived', types: ['mentions', 'replies'], unread: false })
    expect(inboxSearch(location)).toBe('?view=archived&type=mentions%2Creplies')
    expect(inboxSearch({ view: 'inbox', types: [], unread: true })).toBe('?view=inbox&unread=1')
  })

  it('keeps old ?box= links working, opening the retired watching box without a type', () => {
    expect(parse('?box=mentions')).toEqual({ view: 'inbox', types: ['mentions'], unread: false })
    expect(parse('?box=unread')).toEqual({ view: 'inbox', types: [], unread: true })
    expect(parse('?box=watching')).toEqual({ view: 'inbox', types: [], unread: false })
    expect(parse('?box=all')).toEqual({ view: 'inbox', types: [], unread: false })
    expect(parse('')).toEqual({ view: 'inbox', types: [], unread: false })
  })

  it('does not put the unread Filter in the archive', () => {
    expect(inboxSearch({ view: 'archived', types: [], unread: true })).toBe('?view=archived')
  })

  it('toggles a type in chip order', () => {
    const one = toggleType(parse('?type=replies'), 'mentions')
    expect(one.types).toEqual(['mentions', 'replies'])
    expect(toggleType(one, 'replies').types).toEqual(['mentions'])
  })
})

describe('read state and empty screens', () => {
  it('lets the unread Filter win over "show read"', () => {
    expect(readState(true, true)).toBe('unread')
    expect(readState(true, false)).toBe('unread')
    expect(readState(false, true)).toBe('all')
    expect(readState(false, false)).toBe('unread')
  })

  it('counts only conditions that narrow the result; grouping does not', () => {
    const base = { showRead: true, showSnoozed: false, groupByDay: false }
    expect(emptyKind(parse('?view=inbox'), base)).toBe('none')
    expect(emptyKind(parse('?view=inbox&type=mentions'), base)).toBe('filtered')
    expect(emptyKind(parse('?view=inbox&unread=1'), base)).toBe('filtered')
    expect(emptyKind(parse('?view=inbox'), { ...base, showRead: false })).toBe('filtered')
    expect(emptyKind(parse('?view=inbox'), { ...base, showSnoozed: true })).toBe('filtered')
    expect(emptyKind(parse('?view=archived'), base)).toBe('none')
  })
})

describe('grouping by day', () => {
  const at = (iso: string) => ({ id: iso, createdAt: iso }) as InboxItem
  it('titles today and yesterday and continues a group across pages', () => {
    const now = new Date(2026, 9, 24, 12, 0, 0)
    const items = [new Date(2026, 9, 24, 9), new Date(2026, 9, 24, 8), new Date(2026, 9, 23, 20), new Date(2026, 9, 1, 8)].map((d) => at(d.toISOString()))
    const groups = groupByDay(items, (i) => i.createdAt, now)
    expect(groups.map((g) => [g.day, g.items.length])).toEqual([['today', 2], ['yesterday', 1], ['date', 1]])
  })
})
