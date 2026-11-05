import { describe, expect, it } from 'vitest'
import { claimOnce, createSection, deleteSection, moveConversation, parseSections, pruneSections, reorderSections, visibleSlice } from './chatGroups'

describe('chat sections', () => {
  it('ignores a broken save and keeps one place per conversation', () => {
    expect(parseSections('nope')).toEqual({ sections: [] })
    expect(parseSections(JSON.stringify({ sections: [{ id: 'a', name: 'Design', conversationIds: ['c1', 'c1', 3] }] })).sections[0]).toEqual({
      id: 'a',
      name: 'Design',
      conversationIds: ['c1'],
    })
    const doubled = claimOnce({
      sections: [
        { id: 'a', name: 'A', conversationIds: ['c1'] },
        { id: 'b', name: 'B', conversationIds: ['c1', 'c2'] },
      ],
    })
    expect(doubled.sections[1]?.conversationIds).toEqual(['c2'])
  })

  it('moves a conversation into one section and sends it home when the section is removed', () => {
    const created = createSection({ sections: [] }, '  Design  ')
    const id = created.sections[0]?.id ?? ''
    const moved = moveConversation(created, 'c1', id)
    expect(moved.sections[0]?.conversationIds).toEqual(['c1'])
    const shifted = moveConversation(moved, 'c1', id, 0)
    expect(shifted.sections[0]?.conversationIds).toEqual(['c1'])
    const home = moveConversation(moved, 'c1', null)
    expect(home.sections[0]?.conversationIds).toEqual([])
    expect(deleteSection(moved, id).sections).toEqual([])
  })

  it('reorders sections and drops conversations that no longer exist', () => {
    const layout = {
      sections: [
        { id: 'a', name: 'A', conversationIds: ['gone', 'c1'] },
        { id: 'b', name: 'B', conversationIds: ['c2'] },
      ],
    }
    expect(reorderSections(layout, 'b', 'a').sections.map((section) => section.id)).toEqual(['b', 'a'])
    expect(pruneSections(layout, new Set(['c1', 'c2'])).sections[0]?.conversationIds).toEqual(['c1'])
  })

  it('keeps unread rows visible when the rest is folded', () => {
    const items = Array.from({ length: 12 }, (_, index) => index)
    const folded = visibleSlice(items, false, (item) => item === 11)
    expect(folded.shown[0]).toBe(11)
    expect(folded.shown).toHaveLength(10)
    expect(folded.more).toBe(2)
    expect(visibleSlice(items, true, () => false).more).toBe(0)
  })
})
