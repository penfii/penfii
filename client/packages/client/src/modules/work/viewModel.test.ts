import { describe, expect, it } from 'vitest'
import type { Sprint, ViewDefinition } from '@penfii/contracts'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'
import {
  buildColumns,
  buildSections,
  canReorder,
  defaultDefinition,
  defaultLayout,
  definitionFromParams,
  emptyFilter,
  filterCount,
  fitLayout,
  groupsFor,
  layoutFromParams,
  layoutsFor,
  matches,
  paramsWithDraft,
  paramsWithLayout,
  sameDefinition,
  siblingIds,
  sortTickets,
  tableColumns,
  type GroupLabels,
  type WorkTicket,
} from './viewModel'

const labels: GroupLabels = { unassigned: 'None', noPriority: 'No priority', backlog: 'Backlog', priority: (value) => value }
const members = [
  { userId: 'u-b', displayName: 'Bea' },
  { userId: 'u-a', displayName: 'Al' },
]
const sprint = (partial: Partial<Sprint>): Sprint => ({
  id: 'sp1', projectId: 'p', name: 'Sprint 1', goal: null, state: 'PLANNED', startsOn: '2026-11-02', endsOn: '2026-11-13', version: 1, startedAt: null, endedAt: null, ...partial,
})
const withFilter = (filter: Partial<ViewDefinition['filter']>): ViewDefinition => ({ ...defaultDefinition(), filter: { ...emptyFilter(), ...filter } })

const backend = { id: 'l-be', name: 'backend', color: null }
const frontend = { id: 'l-fe', name: 'frontend', color: null }

describe('filters', () => {
  const a = ticket({ id: 'a', key: 'P-1', title: 'Login API', labels: [backend], assigneeId: 'u-a', priority: 'HIGH' })
  const b = ticket({ id: 'b', key: 'P-2', title: 'Login form', labels: [frontend], priority: 'LOW', hierarchyKind: 'EPIC' })
  const c = ticket({ id: 'c', key: 'P-3', title: 'Billing', dependencies: { openBlockers: 1, blockedBy: 1, blocks: 0 }, sprint: { id: 'sp1', name: 'Sprint 1', state: 'PLANNED', inherited: false } })

  it('matches everything with an empty filter', () => {
    expect([a, b, c].every((item) => matches(item, emptyFilter()))).toBe(true)
  })

  it('combines criteria with AND and values inside one criterion with OR', () => {
    expect(matches(a, { ...emptyFilter(), labelIds: ['l-be', 'l-fe'] })).toBe(true)
    expect(matches(b, { ...emptyFilter(), labelIds: ['l-be'] })).toBe(false)
    expect(matches(a, { ...emptyFilter(), labelIds: ['l-be'], priorities: ['LOW'] })).toBe(false)
    expect(matches(b, { ...emptyFilter(), kinds: ['EPIC', 'SUBTASK'] })).toBe(true)
  })

  it('treats unassigned and no-sprint as values of their criterion', () => {
    expect(matches(b, { ...emptyFilter(), assigneeIds: ['u-a'], unassigned: true })).toBe(true)
    expect(matches(a, { ...emptyFilter(), unassigned: true })).toBe(false)
    expect(matches(a, { ...emptyFilter(), noSprint: true })).toBe(true)
    expect(matches(c, { ...emptyFilter(), noSprint: true })).toBe(false)
    expect(matches(c, { ...emptyFilter(), sprintIds: ['sp1'] })).toBe(true)
  })

  it('filters on blocked state and on title or key text', () => {
    expect(matches(c, { ...emptyFilter(), blocked: true })).toBe(true)
    expect(matches(a, { ...emptyFilter(), blocked: true })).toBe(false)
    expect(matches(a, { ...emptyFilter(), blocked: false })).toBe(true)
    expect(matches(a, { ...emptyFilter(), text: 'LOGIN' })).toBe(true)
    expect(matches(c, { ...emptyFilter(), text: 'p-3' })).toBe(true)
    expect(matches(b, { ...emptyFilter(), text: 'billing' })).toBe(false)
  })

  it('counts active criteria', () => {
    expect(filterCount(emptyFilter())).toBe(0)
    expect(filterCount({ ...emptyFilter(), assigneeIds: ['x'], unassigned: true, text: ' ' })).toBe(1)
    expect(filterCount({ ...emptyFilter(), labelIds: ['x'], blocked: false, text: 'a' })).toBe(3)
  })
})

describe('sorting', () => {
  const one = ticket({ id: '1', title: 'b', priority: 'LOW', dueOn: '2026-12-01', createdAt: '2026-10-01' })
  const two = ticket({ id: '2', title: 'a', priority: 'URGENT', dueOn: null, createdAt: '2026-10-03' })
  const three = ticket({ id: '3', title: 'c', priority: null, dueOn: '2026-11-01', createdAt: '2026-10-02' })
  const ids = (items: ReturnType<typeof ticket>[]) => items.map((item) => item.id)

  it('keeps the shared order for rank, and can reverse it', () => {
    expect(ids(sortTickets([one, two, three], { field: 'RANK', dir: 'ASC' }))).toEqual(['1', '2', '3'])
    expect(ids(sortTickets([one, two, three], { field: 'RANK', dir: 'DESC' }))).toEqual(['3', '2', '1'])
  })

  it('puts missing values last whichever way it sorts', () => {
    expect(ids(sortTickets([one, two, three], { field: 'DUE_ON', dir: 'ASC' }))).toEqual(['3', '1', '2'])
    expect(ids(sortTickets([one, two, three], { field: 'DUE_ON', dir: 'DESC' }))).toEqual(['1', '3', '2'])
    expect(ids(sortTickets([one, two, three], { field: 'PRIORITY', dir: 'ASC' }))).toEqual(['2', '1', '3'])
  })

  it('sorts titles with numbers in natural order and keeps ties stable', () => {
    const t10 = ticket({ id: 'x', title: 'Task 10' })
    const t2 = ticket({ id: 'y', title: 'Task 2' })
    expect(ids(sortTickets([t10, t2], { field: 'TITLE', dir: 'ASC' }))).toEqual(['y', 'x'])
    const same = [ticket({ id: 'p', priority: 'LOW' }), ticket({ id: 'q', priority: 'LOW' })]
    expect(ids(sortTickets(same, { field: 'PRIORITY', dir: 'DESC' }))).toEqual(['p', 'q'])
  })
})

describe('columns', () => {
  const tickets = [
    ticket({ id: 'a', statusId: 's-todo', labels: [backend], assigneeId: 'u-b', assigneeName: 'Bea', priority: 'HIGH' }),
    ticket({ id: 'b', statusId: 's-todo', labels: [frontend], assigneeId: 'u-a', assigneeName: 'Al' }),
    ticket({ id: 'c', statusId: 's-doing', labels: [backend], sprint: { id: 'sp1', name: 'Sprint 1', state: 'PLANNED', inherited: false } }),
  ]
  const board = fixtureBoard(tickets)

  it('groups by State and keeps empty columns', () => {
    const columns = buildColumns(board, defaultDefinition(), [], members, labels)
    expect(columns.map((column) => [column.name, column.tickets.map((item) => item.id)])).toEqual([
      ['Todo', ['a', 'b']],
      ['In Progress', ['c']],
      ['Done', []],
    ])
  })

  it('filters inside the State columns and still reports the unfiltered total for WIP', () => {
    const columns = buildColumns(board, withFilter({ labelIds: ['l-be'] }), [], members, labels)
    expect(columns[0].tickets.map((item) => item.id)).toEqual(['a'])
    expect(columns[0].total).toBe(2)
  })

  it('groups by assignee with names in order and the unassigned last', () => {
    const columns = buildColumns(board, { ...defaultDefinition(), group: 'ASSIGNEE' }, [], members, labels)
    expect(columns.map((column) => [column.name, column.tickets.map((item) => item.id)])).toEqual([
      ['Al', ['b']],
      ['Bea', ['a']],
      ['None', ['c']],
    ])
    expect(columns.every((column) => column.stateId === null)).toBe(true)
  })

  it('groups by priority in a fixed order', () => {
    const columns = buildColumns(board, { ...defaultDefinition(), group: 'PRIORITY' }, [], members, labels)
    expect(columns.map((column) => column.name)).toEqual(['URGENT', 'HIGH', 'MEDIUM', 'LOW', 'No priority'])
    expect(columns[1].tickets.map((item) => item.id)).toEqual(['a'])
    expect(columns[4].tickets.map((item) => item.id)).toEqual(['b', 'c'])
  })

  it('groups by Sprint: backlog first, then every open Sprint even when empty', () => {
    const columns = buildColumns(board, { ...defaultDefinition(), group: 'SPRINT' }, [sprint({}), sprint({ id: 'sp2', name: 'Sprint 2', state: 'ACTIVE' }), sprint({ id: 'sp0', name: 'Old', state: 'COMPLETED' })], members, labels)
    expect(columns.map((column) => [column.name, column.sprintId, column.tickets.length])).toEqual([
      ['Backlog', null, 2],
      ['Sprint 1', 'sp1', 1],
      ['Sprint 2', 'sp2', 0],
    ])
  })

  it('allows reordering only where the order shown is the shared order', () => {
    expect(canReorder(defaultDefinition())).toBe(true)
    expect(canReorder({ ...defaultDefinition(), group: 'SPRINT' })).toBe(true)
    expect(canReorder({ ...defaultDefinition(), group: 'ASSIGNEE' })).toBe(false)
    expect(canReorder({ ...defaultDefinition(), sort: { field: 'TITLE', dir: 'ASC' } })).toBe(false)
    expect(canReorder({ ...defaultDefinition(), sort: { field: 'RANK', dir: 'DESC' } })).toBe(false)
  })
})

describe('the draft in the URL', () => {
  it('has no draft until one is written', () => {
    expect(definitionFromParams(new URLSearchParams('panel=ticket:1'))).toBeNull()
    expect(definitionFromParams(new URLSearchParams('layout=table'))).toBeNull()
  })

  it('reads a hand-written address the same way as one the page wrote', () => {
    const typed = definitionFromParams(new URLSearchParams('layout=table&sort=ESTIMATE:DESC&est=SET'))
    expect(typed?.sort).toEqual({ field: 'ESTIMATE', dir: 'DESC' })
    expect(typed?.filter.estimate).not.toBeNull()
    // Written back, it is the canonical form and reads to the same definition.
    const canonical = paramsWithDraft(new URLSearchParams('layout=table&sort=ESTIMATE:DESC&est=SET'), typed as ViewDefinition, defaultDefinition())
    expect(canonical.get('d')).toBe('1')
    expect(definitionFromParams(canonical)).toEqual(typed)
  })

  it('round-trips a definition and leaves other params alone', () => {
    const draft: ViewDefinition = {
      filter: { ...emptyFilter(), labelIds: ['l2', 'l1'], assigneeIds: ['u1'], unassigned: true, priorities: ['HIGH'], kinds: ['TASK'], sprintIds: ['s1'], noSprint: true, blocked: false, text: 'login', categories: ['DONE'], channelIds: ['c1'], projectIds: ['p1'] },
      group: 'PRIORITY',
      sort: { field: 'DUE_ON', dir: 'DESC' },
      display: { subtasks: false, compact: true, columns: ['KEY', 'TITLE', 'PROJECT'] },
    }
    const written = paramsWithDraft(new URLSearchParams('panel=ticket:1&v=abc'), draft, defaultDefinition())
    expect(written.get('panel')).toBe('ticket:1')
    expect(written.get('v')).toBe('abc')
    const read = definitionFromParams(written)
    expect(read && sameDefinition(read, draft)).toBe(true)
  })

  it('clears the draft when it equals the base, so the URL stays clean', () => {
    const dirty = paramsWithDraft(new URLSearchParams(), withFilter({ text: 'x' }), defaultDefinition())
    expect(dirty.get('d')).toBe('1')
    const clean = paramsWithDraft(dirty, defaultDefinition(), defaultDefinition())
    expect([...clean.keys()]).toEqual([])
  })

  it('keeps an emptied filter as a draft when the saved view has criteria', () => {
    const saved = withFilter({ labelIds: ['l1'] })
    const emptied = paramsWithDraft(new URLSearchParams('v=abc'), defaultDefinition(), saved)
    expect(emptied.get('d')).toBe('1')
    const read = definitionFromParams(emptied)
    expect(read && sameDefinition(read, defaultDefinition())).toBe(true)
  })

  it('ignores unknown group, sort and kind values', () => {
    const read = definitionFromParams(new URLSearchParams('d=1&group=COLOR&sort=SIZE:UP&kind=STORY,TASK&priority=SOON,HIGH'))
    expect(read?.group).toBe('STATUS')
    expect(read?.sort).toEqual({ field: 'RANK', dir: 'ASC' })
    expect(read?.filter.kinds).toEqual(['TASK'])
    expect(read?.filter.priorities).toEqual(['HIGH'])
  })

  it('treats list order as irrelevant when comparing', () => {
    expect(sameDefinition(withFilter({ labelIds: ['a', 'b'] }), withFilter({ labelIds: ['b', 'a'] }))).toBe(true)
    expect(sameDefinition(withFilter({ text: ' ' }), defaultDefinition())).toBe(true)
  })
})

describe('hierarchy-aware columns', () => {
  const parentRef = (id: string, key: string, title: string) => ({ id, key, title, hierarchyKind: 'TASK' as const })
  const child = (id: string, parent: string, over: Parameters<typeof ticket>[0] = {}) =>
    ticket({ id, key: id.toUpperCase(), title: id, hierarchyKind: 'SUBTASK', parentId: parent, parent: parentRef(parent, parent.toUpperCase(), parent), ...over })
  const names = (rows: ReturnType<typeof buildColumns>[number]['rows']) => rows.map((row) => (row.kind === 'context' ? `[${row.parent.key}${row.stateName ? ` ${row.stateName}` : ''}]` : `${row.nested ? '  ' : ''}${row.ticket.id}`))
  const todoColumn = (tickets: ReturnType<typeof ticket>[]) => buildColumns(fixtureBoard(tickets), defaultDefinition(), [], [], labels)[0]

  it('groups children under a parent in the same State, parents by their own update time', () => {
    const column = todoColumn([
      ticket({ id: 'a', updatedAt: '2026-10-19T10:00:00Z' }),
      ticket({ id: 'b', updatedAt: '2026-10-19T12:00:00Z' }),
      child('a1', 'a', { updatedAt: '2026-10-19T23:00:00Z' }),
      child('b1', 'b', { updatedAt: '2026-10-19T01:00:00Z' }),
      child('a2', 'a'),
      ticket({ id: 'c', updatedAt: '2026-10-19T11:00:00Z' }),
    ])
    // b newest, then c, then a — a child's late update does not lift its group.
    expect(names(column.rows)).toEqual(['b', '  b1', 'c', 'a', '  a1', '  a2'])
    expect(column.tickets.map((item) => item.id)).toEqual(['b', 'b1', 'c', 'a', 'a1', 'a2'])
    expect(column.hierarchical).toBe(true)
  })

  it('keeps shared order between parents updated at the same moment and between children', () => {
    const same = '2026-10-19T10:00:00Z'
    const column = todoColumn([ticket({ id: 'x', updatedAt: same }), ticket({ id: 'y', updatedAt: same }), child('y2', 'y'), child('y1', 'y')])
    expect(names(column.rows)).toEqual(['x', 'y', '  y2', '  y1'])
  })

  it('shows a parent that lives in another State as a context header, not a card', () => {
    const board = fixtureBoard([ticket({ id: 'a', statusId: 's-done' }), child('a2', 'a', { statusId: 's-doing' }), child('a1', 'a')])
    const [todo, doing, done] = buildColumns(board, defaultDefinition(), [], [], labels)
    expect(names(todo.rows)).toEqual(['[A Done]', '  a1'])
    expect(names(doing.rows)).toEqual(['[A Done]', '  a2'])
    expect(names(done.rows)).toEqual(['a'])
    // Counts and WIP see only real Tickets.
    expect(todo.tickets.map((item) => item.id)).toEqual(['a1'])
    expect(todo.total).toBe(1)
    expect(doing.tickets).toHaveLength(1)
  })

  it('anchors a Task that has Subtasks here instead of nesting it under its Epic', () => {
    const epic = parentRef('e', 'E', 'e')
    const column = todoColumn([ticket({ id: 't', parentId: 'e', parent: epic }), child('s', 't'), ticket({ id: 'u', parentId: 'e', parent: epic })])
    expect(names(column.rows)).toEqual(['t', '  s', '[E]', '  u'])
  })

  it('is a flat list in the shared order once the sort or the group is not the default', () => {
    const board = fixtureBoard([ticket({ id: 'a' }), child('a1', 'a')])
    const byTitle = buildColumns(board, { ...defaultDefinition(), sort: { field: 'TITLE', dir: 'ASC' } }, [], [], labels)[0]
    expect(byTitle.hierarchical).toBe(false)
    expect(byTitle.rows.every((row) => row.kind === 'ticket' && !row.nested)).toBe(true)
    const bySprint = buildColumns(board, { ...defaultDefinition(), group: 'SPRINT' }, [], [], labels)[0]
    expect(bySprint.hierarchical).toBe(false)
  })

  it('groups only the Tickets a filter leaves, and a filtered-out parent becomes context', () => {
    const board = fixtureBoard([ticket({ id: 'a', labels: [] }), child('a1', 'a', { labels: [backend] })])
    const [todo] = buildColumns(board, withFilter({ labelIds: ['l-be'] }), [], [], labels)
    expect(names(todo.rows)).toEqual(['[A Todo]', '  a1'])
    expect(todo.total).toBe(2)
  })

  it('keeps a parent that was put on top above newer updates, until something more recent happens', () => {
    const old = '2026-10-19T08:00:00Z'
    const column = todoColumn([
      ticket({ id: 'a', updatedAt: '2026-10-19T12:00:00Z' }),
      ticket({ id: 'b', updatedAt: old, movedToTopAt: '2026-10-19T13:00:00Z' }),
      child('b1', 'b'),
    ])
    expect(names(column.rows)).toEqual(['b', '  b1', 'a'])
    // A later update of a wins again.
    const later = todoColumn([
      ticket({ id: 'a', updatedAt: '2026-10-19T14:00:00Z' }),
      ticket({ id: 'b', updatedAt: old, movedToTopAt: '2026-10-19T13:00:00Z' }),
    ])
    expect(names(later.rows)).toEqual(['a', 'b'])
  })

  it('positions a drop only against the siblings of the same parent', () => {
    const column = todoColumn([ticket({ id: 'a' }), child('a1', 'a'), child('a2', 'a'), ticket({ id: 'z', updatedAt: '2026-10-18T00:00:00Z' })])
    expect(siblingIds(column, column.tickets.find((item) => item.id === 'a1') as never)).toEqual(['a1', 'a2'])
    expect(siblingIds(column, column.tickets.find((item) => item.id === 'a') as never)).toEqual([])
    expect(siblingIds(column, column.tickets.find((item) => item.id === 'z') as never)).toEqual([])
    // A child arriving from another State is placed among the siblings already here.
    expect(siblingIds(column, child('a3', 'a'))).toEqual(['a1', 'a2'])
  })
})

describe('layouts follow the place', () => {
  it('offers a Board only in a Project', () => {
    expect(layoutsFor('PROJECT')).toEqual(['BOARD', 'LIST', 'TABLE'])
    expect(layoutsFor('TEAM')).toEqual(['LIST', 'TABLE'])
    expect(layoutsFor('WORKSPACE')).toEqual(['LIST', 'TABLE'])
    expect(defaultLayout('PROJECT')).toBe('BOARD')
    expect(defaultLayout('TEAM')).toBe('LIST')
  })

  it('offers only groups a layout can show', () => {
    expect(groupsFor('PROJECT', 'BOARD')).toEqual(['STATUS', 'ASSIGNEE', 'PRIORITY', 'SPRINT'])
    expect(groupsFor('PROJECT', 'LIST')).toContain('CATEGORY')
    expect(groupsFor('PROJECT', 'LIST')).toContain('MILESTONE')
    // A Team mixes Projects: no State group, but the category and the Project.
    expect(groupsFor('TEAM', 'LIST')).toEqual(['CATEGORY', 'PROJECT', 'MILESTONE', 'ASSIGNEE', 'PRIORITY', 'SPRINT', 'NONE'])
    expect(defaultDefinition('TEAM').group).toBe('CATEGORY')
    expect(defaultDefinition('PROJECT').group).toBe('STATUS')
  })

  it('moves to a group the Board can show when the layout changes', () => {
    const byMilestone = { ...defaultDefinition(), group: 'MILESTONE' as const }
    expect(fitLayout(byMilestone, 'PROJECT', 'BOARD').group).toBe('STATUS')
    expect(fitLayout(byMilestone, 'PROJECT', 'TABLE').group).toBe('MILESTONE')
  })

  it('writes the layout to the URL only when it differs from the View', () => {
    expect(paramsWithLayout(new URLSearchParams('v=a'), 'TABLE', 'BOARD').get('layout')).toBe('table')
    expect(paramsWithLayout(new URLSearchParams('layout=table'), 'BOARD', 'BOARD').has('layout')).toBe(false)
    expect(layoutFromParams(new URLSearchParams('layout=table'), 'PROJECT')).toBe('TABLE')
    // A Board asked for in a Team is not an option there.
    expect(layoutFromParams(new URLSearchParams('layout=board'), 'TEAM')).toBeNull()
  })
})

describe('sections of a List or Table', () => {
  const labels = { unassigned: 'none', noPriority: 'no priority', backlog: 'backlog', priority: (value: string) => value, category: (value: string) => value, noMilestone: 'no milestone' }
  const work = (partial: Partial<WorkTicket>): WorkTicket => ({
    ...ticket({ id: 'x' }), projectName: 'Alpha', channelName: null, stateName: 'Todo', stateCategory: 'NOT_STARTED', ...partial,
  })
  const tickets = [
    work({ id: 'a', key: 'A-1', projectId: 'pa', projectName: 'Alpha', stateName: 'Todo', stateCategory: 'NOT_STARTED' }),
    work({ id: 'b', key: 'B-1', projectId: 'pb', projectName: 'Beta', statusId: 'sb', stateName: 'Review', stateCategory: 'IN_PROGRESS', milestones: [{ id: 'm1', name: 'Launch', state: 'OPEN' }] }),
    work({ id: 'c', key: 'B-2', projectId: 'pb', projectName: 'Beta', statusId: 'sb2', stateName: 'Shipped', stateCategory: 'DONE', milestones: [{ id: 'm1', name: 'Launch', state: 'OPEN' }] }),
  ]

  it('groups Projects with different State names by category, each Ticket keeping its own State', () => {
    const sections = buildSections(tickets, { ...defaultDefinition('TEAM'), group: 'CATEGORY' }, { labels })
    expect(sections.map((section) => section.title)).toEqual(['NOT_STARTED', 'IN_PROGRESS', 'DONE'])
    expect(sections[1]?.tickets[0]?.stateName).toBe('Review')
    expect(sections[2]?.tickets[0]?.stateName).toBe('Shipped')
  })

  it('groups by Project, by Milestone and not at all', () => {
    expect(buildSections(tickets, { ...defaultDefinition('TEAM'), group: 'PROJECT' }, { labels }).map((section) => `${section.title}:${section.tickets.length}`)).toEqual(['Alpha:1', 'Beta:2'])
    const byMilestone = buildSections(tickets, { ...defaultDefinition('TEAM'), group: 'MILESTONE' }, { labels })
    expect(byMilestone.map((section) => `${section.title}:${section.tickets.length}`)).toEqual(['Launch:2', 'no milestone:1'])
    expect(buildSections(tickets, { ...defaultDefinition('TEAM'), group: 'NONE' }, { labels })).toHaveLength(1)
  })

  it('filters by category and project before grouping, with the same words in every Project', () => {
    const definition = { ...defaultDefinition('TEAM'), filter: { ...emptyFilter(), categories: ['DONE' as const], projectIds: ['pb'] } }
    const sections = buildSections(tickets, definition, { labels })
    expect(sections.flatMap((section) => section.tickets.map((item) => item.key))).toEqual(['B-2'])
  })

  it('hides Subtasks when the display says so and nests them under their parent otherwise', () => {
    const parent = work({ id: 'p1', key: 'P-1', hierarchyKind: 'TASK' })
    const child = work({ id: 'c1', key: 'P-2', hierarchyKind: 'SUBTASK', parentId: 'p1', parent: { id: 'p1', key: 'P-1', title: 'Fix login', hierarchyKind: 'TASK' } })
    const nested = buildSections([parent, child], { ...defaultDefinition(), group: 'NONE' }, { labels })
    expect(nested[0]?.rows.map((row) => (row.kind === 'ticket' ? `${row.ticket.key}${row.nested ? ' (child)' : ''}` : 'context'))).toEqual(['P-1', 'P-2 (child)'])
    const hidden = buildSections([parent, child], { ...defaultDefinition(), group: 'NONE', display: { subtasks: false, compact: false, columns: [] } }, { labels })
    expect(hidden[0]?.tickets.map((item) => item.key)).toEqual(['P-1'])
  })

  it('always shows the Project column in a scope that mixes Projects', () => {
    expect(tableColumns(defaultDefinition('TEAM'), true)).toContain('PROJECT')
    expect(tableColumns(defaultDefinition(), false)).not.toContain('PROJECT')
    expect(tableColumns({ ...defaultDefinition(), display: { subtasks: true, compact: false, columns: ['KEY', 'TITLE', 'CHANNEL'] } }, false)).toEqual(['KEY', 'TITLE', 'CHANNEL'])
  })
})

describe('estimate in a View', () => {
  const est = (value: string, unit: 'POINTS' | 'HOURS' = 'POINTS') => ({ value, unit })
  const three = ticket({ id: 'e3', key: 'P-3', estimate: est('3') })
  const five = ticket({ id: 'e5', key: 'P-5', estimate: est('5.5') })
  const zero = ticket({ id: 'e0', key: 'P-0', estimate: est('0') })
  const none = ticket({ id: 'en', key: 'P-N' })
  const all = [three, five, zero, none]
  const ids = (list: Array<{ id: string }>) => list.map((item) => item.id)

  it('filters by having one, having none, and by comparing', () => {
    const pick = (estimate: ViewDefinition['filter']['estimate']) => ids(all.filter((item) => matches(item, { ...emptyFilter(), estimate })))
    expect(pick({ op: 'SET' })).toEqual(['e3', 'e5', 'e0'])
    expect(pick({ op: 'UNSET' })).toEqual(['en'])
    expect(pick({ op: 'GTE', value: '5.5' })).toEqual(['e5'])
    expect(pick({ op: 'LTE', value: '3' })).toEqual(['e3', 'e0'])
    expect(pick({ op: 'GTE', value: '0' })).toEqual(['e3', 'e5', 'e0'])
    expect(pick(null)).toEqual(['e3', 'e5', 'e0', 'en'])
    expect(pick({ op: 'GTE', value: 'x' })).toEqual([])
    // An older saved View has no estimate criterion at all.
    expect(matches(none, { ...emptyFilter(), estimate: undefined })).toBe(true)
  })

  it('counts as one criterion and travels in the address', () => {
    expect(filterCount({ ...emptyFilter(), estimate: { op: 'SET' } })).toBe(1)
    const draft = withFilter({ estimate: { op: 'GTE', value: '5.50' } })
    const params = paramsWithDraft(new URLSearchParams(), draft, defaultDefinition())
    expect(params.get('est')).toBe('GTE:5.5')
    expect(definitionFromParams(params)?.filter.estimate).toEqual({ op: 'GTE', value: '5.5' })
    expect(definitionFromParams(paramsWithDraft(new URLSearchParams(), withFilter({ estimate: { op: 'UNSET' } }), defaultDefinition()))?.filter.estimate).toEqual({ op: 'UNSET' })
    expect(definitionFromParams(new URLSearchParams('d=1&est=GTE:abc'))?.filter.estimate).toBeNull()
    expect(definitionFromParams(new URLSearchParams('d=1&est=EQ:1'))?.filter.estimate).toBeNull()
    expect(sameDefinition(withFilter({ estimate: { op: 'GTE', value: '5.50' } }), withFilter({ estimate: { op: 'GTE', value: '5.5' } }))).toBe(true)
    expect(sameDefinition(withFilter({ estimate: { op: 'SET' } }), withFilter({ estimate: null }))).toBe(false)
    expect(sameDefinition(withFilter({ estimate: null }), withFilter({ estimate: undefined }))).toBe(true)
  })

  it('sorts by estimate, the unestimated last in either direction, ties in the shared order', () => {
    const another = ticket({ id: 'e3b', key: 'P-3b', estimate: est('3.00') })
    expect(ids(sortTickets([five, none, three, zero, another], { field: 'ESTIMATE', dir: 'ASC' }))).toEqual(['e0', 'e3', 'e3b', 'e5', 'en'])
    expect(ids(sortTickets([five, none, three, zero, another], { field: 'ESTIMATE', dir: 'DESC' }))).toEqual(['e5', 'e3', 'e3b', 'e0', 'en'])
  })

  it('is a column a Table can show and sort', () => {
    expect(tableColumns({ ...defaultDefinition(), display: { subtasks: true, compact: false, columns: ['KEY', 'TITLE', 'ESTIMATE'] } }, false)).toEqual(['KEY', 'TITLE', 'ESTIMATE'])
    expect(definitionFromParams(new URLSearchParams('d=1&cols=KEY,ESTIMATE&sort=ESTIMATE:DESC'))?.sort.field).toBe('ESTIMATE')
  })
})
