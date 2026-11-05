import { describe, expect, it } from 'vitest'
import type { SearchResult } from '@penfii/contracts'
import { resultHref } from './resultHref'

const file = (fields: Record<string, unknown>): SearchResult => ({ kind: 'file', id: 'f1', title: 'a.txt', subtitle: null, snippet: [], createdAt: null, fields })

describe('where a file result opens', () => {
  it('opens the conversation of a message file and the Ticket of a Ticket file', () => {
    expect(resultHref('w1', file({ conversationId: 'c1', messageId: 'm1' }))).toBe('/w/w1/c/c1')
    expect(resultHref('w1', file({ ticketId: 't1', projectId: 'p1' }))).toBe('/w/w1/projects/p1/work?panel=ticket:t1')
    expect(resultHref('w1', file({}))).toBeNull()
  })
})
