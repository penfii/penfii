import { describe, expect, it } from 'vitest'
import type { Channel, Dm, DocNode, Project, ScopedTicket } from '@penfii/contracts'
import { localDay, recentChats, recentDocs, recentProjects, todayList, toDoList } from './homeModel'

function ticket(over: Partial<ScopedTicket>): ScopedTicket {
  return { id: 't', key: 'P-1', title: 'x', projectId: 'p', projectName: 'P', stateCategory: 'NOT_STARTED', dueOn: null, updatedAt: '2026-10-01T00:00:00Z', schedule: null, ...over } as ScopedTicket
}

describe('Home lists', () => {
  it('lists open tickets, due ones first and soonest first', () => {
    const list = toDoList([
      ticket({ id: 'a', updatedAt: '2026-10-05T00:00:00Z' }),
      ticket({ id: 'b', dueOn: '2026-10-20' }),
      ticket({ id: 'c', dueOn: '2026-10-10' }),
      ticket({ id: 'd', stateCategory: 'DONE', dueOn: '2026-10-01' }),
    ])
    expect(list.map((item) => item.id)).toEqual(['c', 'b', 'a'])
  })

  it('shows today: due today or a schedule that covers today, never finished work', () => {
    const today = localDay(new Date('2026-10-27T12:00:00'))
    const list = todayList(
      [
        ticket({ id: 'due', dueOn: today }),
        ticket({ id: 'span', schedule: { plannedStartAt: '2026-10-26T09:00:00', plannedEndAt: '2026-10-28T18:00:00', durationMinutes: null, isScheduleMilestone: false } }),
        ticket({ id: 'past', schedule: { plannedStartAt: '2026-10-01T09:00:00', plannedEndAt: '2026-10-02T18:00:00', durationMinutes: null, isScheduleMilestone: false } }),
        ticket({ id: 'done', dueOn: today, stateCategory: 'DONE' }),
      ],
      today,
    )
    expect(list.map((item) => item.id)).toEqual(['due', 'span'])
  })

  it('orders recent documents by change time', () => {
    const nodes = [
      { id: 'a', title: 'A', updatedAt: '2026-10-01T00:00:00Z' },
      { id: 'b', title: 'B', updatedAt: '2026-10-03T00:00:00Z' },
    ] as DocNode[]
    const items = recentDocs(nodes, 'w', 5)
    expect(items.map((item) => item.id)).toEqual(['b', 'a'])
    expect(items[0]?.to).toBe('/w/w/docs?doc=b')
  })

  it('links Direct messages and Channels to their conversations, dated ones first', () => {
    const dms = [{ id: 'd1', conversationId: 'cd1', peers: [{ displayName: 'Kim' }], lastMessageAt: '2026-10-03T00:00:00Z', lastMessagePreview: 'hi' }] as unknown as Dm[]
    const channels = [
      { id: 'c1', name: 'general', conversationId: 'cc1', projectId: null, unreadCount: 2 },
      { id: 'c2', name: 'pj', conversationId: 'cc2', projectId: 'p9', unreadCount: 1 },
      { id: 'c3', name: 'quiet', conversationId: 'cc3', projectId: null, unreadCount: 0 },
    ] as Channel[]
    const items = recentChats(dms, channels, 'w', 5)
    expect(items.map((item) => item.to)).toEqual(['/w/w/c/cd1', '/w/w/c/cc1', '/w/w/projects/p9/chat/cc2'])
  })

  it('orders Projects by the latest change to my tickets in them and skips Projects I do not take part in', () => {
    const projects = [
      { id: 'p1', name: 'One' },
      { id: 'p2', name: 'Two' },
      { id: 'p3', name: 'Three', participating: false },
    ] as Project[]
    const tickets = [ticket({ projectId: 'p1', updatedAt: '2026-10-01T00:00:00Z' }), ticket({ projectId: 'p2', updatedAt: '2026-10-09T00:00:00Z' }), ticket({ projectId: 'p3', updatedAt: '2026-10-10T00:00:00Z' })]
    expect(recentProjects(tickets, projects, 'w', 5).map((item) => item.id)).toEqual(['p2', 'p1'])
  })
})
