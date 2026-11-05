import { describe, expect, it } from 'vitest'
import type { ProjectBoard, Ticket } from '@penfii/contracts'
import { fixtureTicket } from './testFixtures'
import { childKind, descriptionText, eligibleParents, isOverdue, parentKind, progressPercent, sortFields, textToDescription } from './ticketModel'

const ticket = fixtureTicket

describe('hierarchy kinds', () => {
  it('follows EPIC > TASK > SUBTASK', () => {
    expect(childKind('EPIC')).toBe('TASK')
    expect(childKind('TASK')).toBe('SUBTASK')
    expect(childKind('SUBTASK')).toBeNull()
    expect(parentKind('SUBTASK')).toBe('TASK')
    expect(parentKind('TASK')).toBe('EPIC')
    expect(parentKind('EPIC')).toBeNull()
  })
})

describe('description text', () => {
  it('keeps one line per paragraph and round-trips', () => {
    const doc = textToDescription('first\n\nthird')
    expect(descriptionText(doc)).toBe('first\n\nthird')
    expect(descriptionText(textToDescription('  \n'))).toBe('')
    expect(textToDescription('').content).toEqual([])
  })

  it('reads text from documents with nested inline content', () => {
    const doc = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'a' }, { type: 'text', text: 'b' }] }] }
    expect(descriptionText(doc)).toBe('ab')
    expect(descriptionText(null)).toBe('')
  })
})

describe('child progress', () => {
  it('ignores canceled children and never divides by zero', () => {
    expect(progressPercent({ total: 4, done: 1, canceled: 0 })).toBe(25)
    expect(progressPercent({ total: 4, done: 1, canceled: 2 })).toBe(50)
    expect(progressPercent({ total: 2, done: 0, canceled: 2 })).toBe(0)
    expect(progressPercent({ total: 0, done: 0, canceled: 0 })).toBe(0)
  })
})

describe('eligible parents', () => {
  const column = (tickets: Ticket[]) => ({ id: 'c', name: 'c', category: 'NOT_STARTED' as const, position: 0, stateId: 's', stateIds: ['s'], ticketCount: tickets.length, wipLimit: null, tickets })
  const board = (tickets: Ticket[]): ProjectBoard => ({ projectId: 'p', workflowVersion: 1, canEdit: true, canManageWorkflow: false, states: [], columns: [column(tickets)] })

  it('offers Epics to a Task and same-channel Tasks to a Subtask', () => {
    const epic = ticket({ id: 'e', hierarchyKind: 'EPIC' })
    const taskA = ticket({ id: 'a', channelId: 'c1' })
    const taskB = ticket({ id: 'b', channelId: 'c2' })
    const sub = ticket({ id: 's1', hierarchyKind: 'SUBTASK', channelId: 'c1', parentId: 'a' })
    const all = board([epic, taskA, taskB, sub])
    expect(eligibleParents(taskA, all).map((item) => item.id)).toEqual(['e'])
    expect(eligibleParents(sub, all).map((item) => item.id)).toEqual(['a'])
    expect(eligibleParents(epic, all)).toEqual([])
  })
})

describe('dates and fields', () => {
  it('flags overdue only for unfinished work', () => {
    const today = new Date(2026, 9, 20)
    expect(isOverdue('2026-10-19', false, today)).toBe(true)
    expect(isOverdue('2026-10-20', false, today)).toBe(false)
    expect(isOverdue('2026-10-19', true, today)).toBe(false)
    expect(isOverdue(null, false, today)).toBe(false)
  })

  it('orders fields for display', () => {
    expect(sortFields(['labels', 'title', 'status'])).toEqual(['title', 'status', 'labels'])
    expect(sortFields(['schedule', 'estimate', 'parent', 'title'])).toEqual(['title', 'parent', 'estimate', 'schedule'])
  })
})
