import type {
  BoardColumn,
  EstimateFilter,
  HierarchyKind,
  ProjectBoard,
  ProjectMember,
  ScopedTicket,
  Sprint,
  TableColumn,
  Ticket,
  TicketRef,
  ViewDefinition,
  ViewFilter,
  ViewGroup,
  ViewLayout,
  ViewSortField,
  ViewTargetKind,
  WorkflowState,
} from '@penfii/contracts'
import { estimateHundredths, estimateSortValue, isEstimateText, normalizeEstimateText } from './planValues'

/*
 * A View is a definition (filter, group, sort, display) plus a layout, never a copy of Tickets.
 * Board, List and Table show the same Ticket dataset; these functions turn that dataset and a
 * definition into what a layout draws. Ordering of the shared rank comes from the server; nothing
 * here invents a position.
 */

/** A Ticket with the places it lives in. Project scope builds these from the Board, wider scopes get them from the server. */
export type WorkTicket = ScopedTicket

export const SORT_FIELDS: ViewSortField[] = ['RANK', 'PRIORITY', 'DUE_ON', 'CREATED', 'UPDATED', 'TITLE', 'ESTIMATE']
export const CATEGORIES: WorkflowState['category'][] = ['NOT_STARTED', 'IN_PROGRESS', 'DONE', 'CANCELED']
export const TABLE_COLUMNS: TableColumn[] = ['KEY', 'TITLE', 'STATE', 'ASSIGNEE', 'PRIORITY', 'LABELS', 'PROJECT', 'CHANNEL', 'SPRINT', 'MILESTONE', 'PARENT', 'DUE_ON', 'ESTIMATE']
/** What a Table shows when the View names no columns. */
export const DEFAULT_TABLE_COLUMNS: TableColumn[] = ['KEY', 'TITLE', 'STATE', 'ASSIGNEE', 'PRIORITY', 'LABELS', 'SPRINT', 'DUE_ON']

const BOARD_GROUPS: ViewGroup[] = ['STATUS', 'ASSIGNEE', 'PRIORITY', 'SPRINT']
const PROJECT_GROUPS: ViewGroup[] = ['STATUS', 'CATEGORY', 'MILESTONE', 'ASSIGNEE', 'PRIORITY', 'SPRINT', 'NONE']
const WIDE_GROUPS: ViewGroup[] = ['CATEGORY', 'PROJECT', 'MILESTONE', 'ASSIGNEE', 'PRIORITY', 'SPRINT', 'NONE']

const PRIORITY_ORDER = ['URGENT', 'HIGH', 'MEDIUM', 'LOW']
const KINDS: HierarchyKind[] = ['EPIC', 'TASK', 'SUBTASK']

/** The layouts a place offers. A Board needs one Project's States, so only a Project has one. */
export function layoutsFor(target: ViewTargetKind): ViewLayout[] {
  return target === 'PROJECT' ? ['BOARD', 'LIST', 'TABLE'] : ['LIST', 'TABLE']
}

export function defaultLayout(target: ViewTargetKind): ViewLayout {
  return target === 'PROJECT' ? 'BOARD' : 'LIST'
}

/** The groups that make sense for this place and layout. A Board's columns must be able to take a drop. */
export function groupsFor(target: ViewTargetKind, layout: ViewLayout): ViewGroup[] {
  if (target !== 'PROJECT') return WIDE_GROUPS
  return layout === 'BOARD' ? BOARD_GROUPS : PROJECT_GROUPS
}

export function emptyFilter(): ViewFilter {
  return {
    labelIds: [],
    assigneeIds: [],
    unassigned: false,
    priorities: [],
    typeIds: [],
    kinds: [],
    sprintIds: [],
    noSprint: false,
    milestoneIds: [],
    blocked: null,
    text: null,
    categories: [],
    channelIds: [],
    projectIds: [],
    estimate: null,
  }
}

export function defaultDisplay(): ViewDefinition['display'] {
  return { subtasks: true, compact: false, columns: [] }
}

export function defaultDefinition(target: ViewTargetKind = 'PROJECT'): ViewDefinition {
  return { filter: emptyFilter(), group: target === 'PROJECT' ? 'STATUS' : 'CATEGORY', sort: { field: 'RANK', dir: 'ASC' }, display: defaultDisplay() }
}

/** The same definition, adjusted so the layout can show it (for example a Board cannot group by Milestone). */
export function fitLayout(definition: ViewDefinition, target: ViewTargetKind, layout: ViewLayout): ViewDefinition {
  const groups = groupsFor(target, layout)
  return groups.includes(definition.group) ? definition : { ...definition, group: groups[0] ?? 'STATUS' }
}

const sorted = (items: string[]) => [...items].sort()

/** The estimate criterion in one normal form, so equal Views compare equal. */
function normalizeEstimateFilter(filter: ViewFilter['estimate']): EstimateFilter | null {
  if (!filter) return null
  if (filter.op === 'SET' || filter.op === 'UNSET') return { op: filter.op }
  const value = filter.value?.trim() ?? ''
  return isEstimateText(value) ? { op: filter.op, value: normalizeEstimateText(value) } : null
}

/** `SET`, `UNSET`, `GTE:5` or `LTE:0.5` in the address; anything else is no criterion. */
function estimateFromParam(param: string | null): EstimateFilter | null {
  if (!param) return null
  const [op, value] = param.split(':')
  if (op === 'SET' || op === 'UNSET') return { op }
  return (op === 'GTE' || op === 'LTE') && value !== undefined ? normalizeEstimateFilter({ op, value }) : null
}

function estimateToParam(filter: EstimateFilter): string {
  return filter.op === 'GTE' || filter.op === 'LTE' ? `${filter.op}:${filter.value ?? ''}` : filter.op
}

/** Has one, has none, or compares. A Ticket without an estimate never satisfies a comparison. */
function matchesEstimate(ticket: Ticket, filter: ViewFilter['estimate']): boolean {
  if (!filter) return true
  const estimate = ticket.estimate ?? null
  if (filter.op === 'SET') return estimate !== null
  if (filter.op === 'UNSET') return estimate === null
  if (estimate === null || !filter.value || !isEstimateText(filter.value)) return false
  const actual = estimateHundredths(estimate.value)
  const wanted = estimateHundredths(filter.value)
  return filter.op === 'GTE' ? actual >= wanted : actual <= wanted
}

/** The same definition with lists in a fixed order, so two equal views compare equal. */
export function normalizeDefinition(definition: ViewDefinition): ViewDefinition {
  const filter = definition.filter
  const text = filter.text?.trim() ?? ''
  const display = definition.display ?? defaultDisplay()
  return {
    filter: {
      labelIds: sorted(filter.labelIds),
      assigneeIds: sorted(filter.assigneeIds),
      unassigned: filter.unassigned,
      priorities: sorted(filter.priorities),
      typeIds: sorted(filter.typeIds),
      kinds: sorted(filter.kinds) as HierarchyKind[],
      sprintIds: sorted(filter.sprintIds),
      noSprint: filter.noSprint,
      milestoneIds: sorted(filter.milestoneIds),
      blocked: filter.blocked,
      text: text === '' ? null : text,
      categories: sorted(filter.categories ?? []) as WorkflowState['category'][],
      channelIds: sorted(filter.channelIds ?? []),
      projectIds: sorted(filter.projectIds ?? []),
      estimate: normalizeEstimateFilter(filter.estimate),
    },
    group: definition.group,
    sort: { field: definition.sort.field, dir: definition.sort.dir },
    display: { subtasks: display.subtasks, compact: display.compact, columns: [...display.columns] },
  }
}

export function sameDefinition(a: ViewDefinition, b: ViewDefinition): boolean {
  return JSON.stringify(normalizeDefinition(a)) === JSON.stringify(normalizeDefinition(b))
}

/** How many criteria narrow the Tickets. Group, sort and display do not count. */
export function filterCount(filter: ViewFilter): number {
  return (
    (filter.labelIds.length > 0 ? 1 : 0) +
    (filter.assigneeIds.length > 0 || filter.unassigned ? 1 : 0) +
    (filter.priorities.length > 0 ? 1 : 0) +
    (filter.typeIds.length > 0 ? 1 : 0) +
    (filter.kinds.length > 0 ? 1 : 0) +
    (filter.sprintIds.length > 0 || filter.noSprint ? 1 : 0) +
    (filter.milestoneIds.length > 0 ? 1 : 0) +
    (filter.blocked !== null ? 1 : 0) +
    ((filter.categories ?? []).length > 0 ? 1 : 0) +
    ((filter.channelIds ?? []).length > 0 ? 1 : 0) +
    ((filter.projectIds ?? []).length > 0 ? 1 : 0) +
    (filter.estimate ? 1 : 0) +
    (filter.text && filter.text.trim() !== '' ? 1 : 0)
  )
}

// ---- The unsaved draft lives in the URL, so a refresh and a shared link keep it. -------------

const DRAFT_KEYS = ['d', 'q', 'label', 'assignee', 'unassigned', 'priority', 'type', 'kind', 'sprint', 'nosprint', 'milestone', 'blocked', 'group', 'sort', 'cat', 'channel', 'project', 'est', 'subtasks', 'compact', 'cols']

const list = (value: string | null) => (value ? value.split(',').filter((item) => item !== '') : [])

/**
 * The draft in the URL, or null when the URL carries none. `d=1` is how the page writes it; a link
 * written by hand with only `sort=` or `est=` is read the same way, so the address and the screen
 * never disagree.
 */
export function definitionFromParams(params: URLSearchParams, fallbackGroup: ViewGroup = 'STATUS'): ViewDefinition | null {
  if (params.get('d') !== '1' && !DRAFT_KEYS.some((key) => key !== 'd' && params.has(key))) return null
  const group = params.get('group')
  const [field, dir] = (params.get('sort') ?? '').split(':')
  const blocked = params.get('blocked')
  const allGroups: ViewGroup[] = ['NONE', 'STATUS', 'CATEGORY', 'PROJECT', 'MILESTONE', 'ASSIGNEE', 'PRIORITY', 'SPRINT']
  return {
    filter: {
      labelIds: list(params.get('label')),
      assigneeIds: list(params.get('assignee')),
      unassigned: params.get('unassigned') === '1',
      priorities: list(params.get('priority')).filter((item) => PRIORITY_ORDER.includes(item)),
      typeIds: list(params.get('type')),
      kinds: list(params.get('kind')).filter((item): item is HierarchyKind => KINDS.some((kind) => kind === item)),
      sprintIds: list(params.get('sprint')),
      noSprint: params.get('nosprint') === '1',
      milestoneIds: list(params.get('milestone')),
      blocked: blocked === '1' ? true : blocked === '0' ? false : null,
      text: params.get('q')?.trim() || null,
      categories: list(params.get('cat')).filter((item): item is WorkflowState['category'] => CATEGORIES.some((category) => category === item)),
      channelIds: list(params.get('channel')),
      projectIds: list(params.get('project')),
      estimate: estimateFromParam(params.get('est')),
    },
    group: allGroups.find((item) => item === group) ?? fallbackGroup,
    sort: {
      field: SORT_FIELDS.find((item) => item === field) ?? 'RANK',
      dir: dir === 'DESC' ? 'DESC' : 'ASC',
    },
    display: {
      subtasks: params.get('subtasks') !== '0',
      compact: params.get('compact') === '1',
      columns: list(params.get('cols')).filter((item): item is TableColumn => TABLE_COLUMNS.some((column) => column === item)),
    },
  }
}

/** Writes `draft` into the params, or clears the draft when it is the same as `base`. */
export function paramsWithDraft(params: URLSearchParams, draft: ViewDefinition, base: ViewDefinition): URLSearchParams {
  const next = new URLSearchParams(params)
  for (const key of DRAFT_KEYS) next.delete(key)
  if (sameDefinition(draft, base)) return next
  const { filter, group, sort, display } = normalizeDefinition(draft)
  next.set('d', '1')
  if (filter.text) next.set('q', filter.text)
  if (filter.labelIds.length) next.set('label', filter.labelIds.join(','))
  if (filter.assigneeIds.length) next.set('assignee', filter.assigneeIds.join(','))
  if (filter.unassigned) next.set('unassigned', '1')
  if (filter.priorities.length) next.set('priority', filter.priorities.join(','))
  if (filter.typeIds.length) next.set('type', filter.typeIds.join(','))
  if (filter.kinds.length) next.set('kind', filter.kinds.join(','))
  if (filter.sprintIds.length) next.set('sprint', filter.sprintIds.join(','))
  if (filter.noSprint) next.set('nosprint', '1')
  if (filter.milestoneIds.length) next.set('milestone', filter.milestoneIds.join(','))
  if (filter.blocked !== null) next.set('blocked', filter.blocked ? '1' : '0')
  if (filter.categories.length) next.set('cat', filter.categories.join(','))
  if (filter.channelIds.length) next.set('channel', filter.channelIds.join(','))
  if (filter.projectIds.length) next.set('project', filter.projectIds.join(','))
  if (filter.estimate) next.set('est', estimateToParam(filter.estimate))
  // The group is always written so a draft does not depend on the default of the place it is read in.
  next.set('group', group)
  if (sort.field !== 'RANK' || sort.dir !== 'ASC') next.set('sort', `${sort.field}:${sort.dir}`)
  if (!display.subtasks) next.set('subtasks', '0')
  if (display.compact) next.set('compact', '1')
  if (display.columns.length) next.set('cols', display.columns.join(','))
  return next
}

/** The layout asked for in the URL, if it is one the place offers. */
export function layoutFromParams(params: URLSearchParams, target: ViewTargetKind): ViewLayout | null {
  const value = params.get('layout')?.toUpperCase()
  return layoutsFor(target).find((layout) => layout === value) ?? null
}

/** Writes the layout into the params, or clears it when it is the layout the base View already has. */
export function paramsWithLayout(params: URLSearchParams, layout: ViewLayout, base: ViewLayout): URLSearchParams {
  const next = new URLSearchParams(params)
  if (layout === base) next.delete('layout')
  else next.set('layout', layout.toLowerCase())
  return next
}

// ---- Evaluation -----------------------------------------------------------------------------

/** Category of a Ticket's State: carried by Tickets of a wider scope, otherwise looked up from the Project's States. */
export type CategoryOf = (ticket: Ticket) => WorkflowState['category'] | undefined

export function matches(ticket: Ticket, filter: ViewFilter, categoryOf?: CategoryOf): boolean {
  const categories = filter.categories ?? []
  if (categories.length > 0) {
    const category = (ticket as Partial<WorkTicket>).stateCategory ?? categoryOf?.(ticket)
    if (!category || !categories.includes(category)) return false
  }
  const channels = filter.channelIds ?? []
  if (channels.length > 0 && !(ticket.channelId && channels.includes(ticket.channelId))) return false
  const projects = filter.projectIds ?? []
  if (projects.length > 0 && !projects.includes(ticket.projectId)) return false
  if (filter.labelIds.length > 0 && !ticket.labels.some((label) => filter.labelIds.includes(label.id))) return false
  if (filter.assigneeIds.length > 0 || filter.unassigned) {
    const byPerson = ticket.assigneeId !== null && filter.assigneeIds.includes(ticket.assigneeId)
    const byNone = filter.unassigned && ticket.assigneeId === null
    if (!byPerson && !byNone) return false
  }
  if (filter.priorities.length > 0 && !(ticket.priority && filter.priorities.includes(ticket.priority))) return false
  if (filter.typeIds.length > 0 && !(ticket.ticketType && filter.typeIds.includes(ticket.ticketType.id))) return false
  if (filter.kinds.length > 0 && !filter.kinds.includes(ticket.hierarchyKind)) return false
  if (filter.sprintIds.length > 0 || filter.noSprint) {
    const inSprint = ticket.sprint !== null && filter.sprintIds.includes(ticket.sprint.id)
    const inNone = filter.noSprint && ticket.sprint === null
    if (!inSprint && !inNone) return false
  }
  if (filter.milestoneIds.length > 0 && !ticket.milestones.some((item) => filter.milestoneIds.includes(item.id))) return false
  if (filter.blocked !== null && (ticket.dependencies.openBlockers > 0) !== filter.blocked) return false
  if (!matchesEstimate(ticket, filter.estimate)) return false
  const text = filter.text?.trim().toLowerCase()
  if (text && !ticket.title.toLowerCase().includes(text) && !ticket.key.toLowerCase().includes(text)) return false
  return true
}

function priorityRank(priority: string | null): number {
  const index = priority ? PRIORITY_ORDER.indexOf(priority) : -1
  return index === -1 ? PRIORITY_ORDER.length : index
}

/** Orders Tickets by the sort. Missing values go last in either direction; ties keep the shared order. */
export function sortTickets(tickets: Ticket[], sort: ViewDefinition['sort']): Ticket[] {
  if (sort.field === 'RANK') return sort.dir === 'ASC' ? tickets : [...tickets].reverse()
  const sign = sort.dir === 'ASC' ? 1 : -1
  const key = (ticket: Ticket): number | string | null => {
    switch (sort.field) {
      case 'PRIORITY':
        return ticket.priority ? priorityRank(ticket.priority) : null
      case 'DUE_ON':
        return ticket.dueOn
      case 'CREATED':
        return ticket.createdAt
      case 'UPDATED':
        return ticket.updatedAt
      case 'TITLE':
        return ticket.title
      case 'ESTIMATE':
        return estimateSortValue(ticket)
      case 'RANK':
      default:
        return null
    }
  }
  const indexed = tickets.map((ticket, index) => ({ ticket, index, value: key(ticket) }))
  indexed.sort((a, b) => {
    if (a.value === null && b.value === null) return a.index - b.index
    if (a.value === null) return 1
    if (b.value === null) return -1
    const order =
      typeof a.value === 'number' && typeof b.value === 'number'
        ? a.value - b.value
        : String(a.value).localeCompare(String(b.value), undefined, { numeric: true })
    return order === 0 ? a.index - b.index : order * sign
  })
  return indexed.map((item) => item.ticket)
}

/**
 * One line of a column. A `ticket` row is a real Ticket (a card); `nested` ones sit under their
 * parent. A `context` row names a parent that is not in this column so its children make sense;
 * it is not a Ticket, cannot be dragged and is never counted.
 */
export type ColumnRow =
  | { kind: 'ticket'; ticket: Ticket; nested: boolean }
  | { kind: 'context'; id: string; parent: TicketRef; stateName: string | null }

export type ColumnModel = {
  id: string
  name: string
  /** The real Tickets of the column, in the order they are shown. Context rows are not here. */
  tickets: Ticket[]
  rows: ColumnRow[]
  /** Whether `rows` groups children under their parent (the default View) or is a flat list. */
  hierarchical: boolean
  /** The Workflow State the column shows; null when the column groups by another field. */
  stateId: string | null
  stateIds: string[]
  /** For Sprint columns: the Sprint, or null for the backlog. */
  sprintId?: string | null
  category?: WorkflowState['category']
  wipLimit: number | null
  /** Tickets in the State before the View filter. WIP warns on this, not on what the filter leaves. */
  total: number
}

export type GroupLabels = { unassigned: string; noPriority: string; backlog: string; priority: (value: string) => string }

const flatRows = (tickets: Ticket[]): ColumnRow[] => tickets.map((ticket) => ({ kind: 'ticket', ticket, nested: false }))

/**
 * Display order of one State column: parent groups, newest parent update first; inside a group
 * the children of this State in the shared order. A child's own update time never moves its
 * group. A parent that is not in the column becomes a context row. Nothing here is stored.
 * `tickets` come in the shared order; ties between groups keep that order.
 */
export function hierarchyRows(tickets: Ticket[], everyTicket: Map<string, Ticket>, stateNames: Map<string, string>): ColumnRow[] {
  // A ticket that has children here anchors its own group; it is not nested under its own parent.
  const anchors = new Set(tickets.filter((ticket) => ticket.parent !== null && ticket.parentId !== null).map((ticket) => ticket.parentId as string))
  type Group = { key: string; first: number; children: Ticket[]; head: Ticket | null; ref: TicketRef | null }
  const groups = new Map<string, Group>()
  tickets.forEach((ticket, index) => {
    const nested = ticket.parent !== null && ticket.parentId !== null && !anchors.has(ticket.id)
    const key = nested ? (ticket.parentId as string) : ticket.id
    let group = groups.get(key)
    if (!group) {
      group = { key, first: index, children: [], head: null, ref: null }
      groups.set(key, group)
    }
    group.first = Math.min(group.first, index)
    if (nested) {
      group.children.push(ticket)
      group.ref = ticket.parent
    } else {
      group.head = ticket
    }
  })
  // A parent counts as touched when it was last updated or last put on top by hand.
  const touched = (ticket: Ticket): number => Math.max(Date.parse(ticket.updatedAt), ticket.movedToTopAt ? Date.parse(ticket.movedToTopAt) : Number.NEGATIVE_INFINITY)
  const stamp = (group: Group): number => {
    const owner = group.head ?? everyTicket.get(group.key)
    return owner ? touched(owner) : Number.NEGATIVE_INFINITY
  }
  const ordered = [...groups.values()].sort((a, b) => {
    const diff = stamp(b) - stamp(a)
    return Number.isNaN(diff) || diff === 0 ? a.first - b.first : diff
  })
  const rows: ColumnRow[] = []
  for (const group of ordered) {
    if (group.head) rows.push({ kind: 'ticket', ticket: group.head, nested: false })
    else if (group.ref) {
      const parent = everyTicket.get(group.key)
      rows.push({ kind: 'context', id: group.key, parent: group.ref, stateName: parent ? (stateNames.get(parent.statusId) ?? null) : null })
    }
    for (const child of group.children) rows.push({ kind: 'ticket', ticket: child, nested: true })
  }
  return rows
}

/** Whether the View shows the default hierarchy-aware order: grouped by State, ordered by the shared rank. */
export function isHierarchical(definition: ViewDefinition): boolean {
  return definition.group === 'STATUS' && definition.sort.field === 'RANK' && definition.sort.dir === 'ASC'
}

/**
 * The cards a drop or a keyboard step is positioned against: in a flat column all of them; in a
 * grouped one only the other children of the same parent. A parent or a lone card has none, so
 * moving it never changes the shared order.
 */
export function siblingIds(column: Pick<ColumnModel, 'hierarchical' | 'tickets' | 'rows'>, ticket: Ticket): string[] {
  if (!column.hierarchical) return column.tickets.map((item) => item.id)
  const own = column.rows.find((row) => row.kind === 'ticket' && row.ticket.id === ticket.id)
  if (own?.kind === 'ticket' && !own.nested) return []
  if (ticket.parentId === null || ticket.parent === null) return []
  const ids: string[] = []
  for (const row of column.rows) if (row.kind === 'ticket' && row.nested && row.ticket.parentId === ticket.parentId) ids.push(row.ticket.id)
  return ids
}

function stateColumn(column: BoardColumn, tickets: Ticket[], rows: ColumnRow[] | null): ColumnModel {
  return {
    id: column.id,
    name: column.name,
    tickets: rows ? rows.flatMap((row) => (row.kind === 'ticket' ? [row.ticket] : [])) : tickets,
    rows: rows ?? flatRows(tickets),
    hierarchical: rows !== null,
    stateId: column.stateId,
    stateIds: column.stateIds,
    category: column.category,
    wipLimit: column.wipLimit,
    total: column.tickets.length,
  }
}

/** The columns a View shows, in order. */
export function buildColumns(board: ProjectBoard, definition: ViewDefinition, sprints: Sprint[], members: ProjectMember[], labels: GroupLabels): ColumnModel[] {
  const categories = new Map(board.states.map((state) => [state.id, state.category]))
  const categoryOf: CategoryOf = (ticket) => categories.get(ticket.statusId)
  const shown = (ticket: Ticket) => definition.display.subtasks || ticket.hierarchyKind !== 'SUBTASK'
  const visible = (tickets: Ticket[]) => sortTickets(tickets.filter((ticket) => shown(ticket) && matches(ticket, definition.filter, categoryOf)), definition.sort)
  if (definition.group === 'STATUS') {
    if (!isHierarchical(definition)) return board.columns.map((column) => stateColumn(column, visible(column.tickets), null))
    const every = new Map(board.columns.flatMap((column) => column.tickets).map((ticket) => [ticket.id, ticket]))
    const names = new Map(board.states.map((state) => [state.id, state.name]))
    return board.columns.map((column) => {
      const own = visible(column.tickets)
      return stateColumn(column, own, hierarchyRows(own, every, names))
    })
  }
  const all = visible(board.columns.flatMap((column) => column.tickets))
  const make = (id: string, name: string, tickets: Ticket[], extra: Partial<ColumnModel> = {}): ColumnModel => ({
    id,
    name,
    tickets,
    rows: flatRows(tickets),
    hierarchical: false,
    stateId: null,
    stateIds: [],
    wipLimit: null,
    total: tickets.length,
    ...extra,
  })
  if (definition.group === 'PRIORITY') {
    return [
      ...PRIORITY_ORDER.map((priority) => make(`priority:${priority}`, labels.priority(priority), all.filter((ticket) => ticket.priority === priority))),
      make('priority:none', labels.noPriority, all.filter((ticket) => !ticket.priority || !PRIORITY_ORDER.includes(ticket.priority))),
    ]
  }
  if (definition.group === 'ASSIGNEE') {
    const named = new Map<string, string>(members.map((member) => [member.userId, member.displayName]))
    for (const ticket of all) if (ticket.assigneeId && !named.has(ticket.assigneeId)) named.set(ticket.assigneeId, ticket.assigneeName ?? ticket.assigneeId)
    const people = [...named.entries()]
      .map(([id, name]) => make(`assignee:${id}`, name, all.filter((ticket) => ticket.assigneeId === id)))
      .filter((column) => column.tickets.length > 0)
      .sort((a, b) => a.name.localeCompare(b.name))
    return [...people, make('assignee:none', labels.unassigned, all.filter((ticket) => ticket.assigneeId === null))]
  }
  // SPRINT: the backlog first, then every Sprint that can still take work, then ended ones with Tickets.
  const open = sprints.filter((sprint) => sprint.state === 'PLANNED' || sprint.state === 'ACTIVE')
  const known = new Set(open.map((sprint) => sprint.id))
  const ended = sprints.filter((sprint) => !known.has(sprint.id) && all.some((ticket) => ticket.sprint?.id === sprint.id))
  return [
    make('sprint:backlog', labels.backlog, all.filter((ticket) => ticket.sprint === null), { sprintId: null }),
    ...[...open, ...ended].map((sprint) => make(`sprint:${sprint.id}`, sprint.name, all.filter((ticket) => ticket.sprint?.id === sprint.id), { sprintId: sprint.id })),
  ]
}

/** Whether cards in this View may be put in a new place in the shared order by dragging. */
export function canReorder(definition: ViewDefinition): boolean {
  return (definition.group === 'STATUS' || definition.group === 'SPRINT') && definition.sort.field === 'RANK' && definition.sort.dir === 'ASC'
}

// ---- List and Table: the same dataset as flat, grouped sections --------------------------------

/** A Project's Board as work Tickets, so List and Table read the same rows a Board draws. */
export function workTicketsFromBoard(board: ProjectBoard, projectName: string, channelNames: Map<string, string>): WorkTicket[] {
  const states = new Map(board.states.map((state) => [state.id, state]))
  return board.columns
    .flatMap((column) => column.tickets)
    .map((ticket) => ({
      ...ticket,
      projectName,
      channelName: ticket.channelId ? (channelNames.get(ticket.channelId) ?? null) : null,
      stateName: states.get(ticket.statusId)?.name ?? '',
      stateCategory: states.get(ticket.statusId)?.category ?? 'NOT_STARTED',
    }))
}

export type Section = {
  id: string
  title: string
  /** Where the section comes from (a Project name for a Milestone or Sprint of a wider scope). */
  hint?: string
  /** The real Tickets of the section in the order shown. Context rows are not here. */
  tickets: WorkTicket[]
  rows: ColumnRow[]
  hierarchical: boolean
}

export type SectionLabels = GroupLabels & { category: (value: WorkflowState['category']) => string; noMilestone: string }

/** Whether a List nests children under their parent: only in the shared order, with Subtasks shown. */
export function nestsChildren(definition: ViewDefinition): boolean {
  return definition.sort.field === 'RANK' && definition.sort.dir === 'ASC' && definition.display.subtasks
}

/**
 * Filters, sorts and groups the dataset for a List or Table. A Ticket sits in one section, except
 * that grouping by Milestone lists a Ticket under each Milestone it belongs to.
 */
export function buildSections(
  tickets: WorkTicket[],
  definition: ViewDefinition,
  context: { states?: WorkflowState[]; sprints?: Sprint[]; members?: ProjectMember[]; labels: SectionLabels },
): Section[] {
  const visible = sortTickets(
    tickets.filter((ticket) => (definition.display.subtasks || ticket.hierarchyKind !== 'SUBTASK') && matches(ticket, definition.filter)),
    definition.sort,
  ) as WorkTicket[]
  const every = new Map(tickets.map((ticket) => [ticket.id, ticket as Ticket]))
  const stateNames = new Map(tickets.map((ticket) => [ticket.statusId, ticket.stateName]))
  const nest = nestsChildren(definition)
  const make = (id: string, title: string, own: WorkTicket[], hint?: string): Section => {
    const rows = nest ? hierarchyRows(own, every, stateNames) : flatRows(own)
    return { id, title, hint, tickets: nest ? rows.flatMap((row) => (row.kind === 'ticket' ? [row.ticket as WorkTicket] : [])) : own, rows, hierarchical: nest }
  }
  const keep = (sections: Section[]) => sections.filter((section) => section.tickets.length > 0)
  switch (definition.group) {
    case 'NONE':
      return [make('all', '', visible)]
    case 'STATUS': {
      const states = context.states ?? []
      const known = new Set(states.map((state) => state.id))
      return keep([
        ...states.map((state) => make(`state:${state.id}`, state.name, visible.filter((ticket) => ticket.statusId === state.id))),
        make('state:other', '', visible.filter((ticket) => !known.has(ticket.statusId))),
      ])
    }
    case 'CATEGORY':
      return keep(CATEGORIES.map((category) => make(`category:${category}`, context.labels.category(category), visible.filter((ticket) => ticket.stateCategory === category))))
    case 'PROJECT': {
      const names = [...new Set(visible.map((ticket) => ticket.projectName))].sort((a, b) => a.localeCompare(b))
      return names.map((name) => make(`project:${name}`, name, visible.filter((ticket) => ticket.projectName === name)))
    }
    case 'MILESTONE': {
      const milestones = new Map<string, { name: string; projectName: string }>()
      for (const ticket of visible) for (const item of ticket.milestones) milestones.set(item.id, { name: item.name, projectName: ticket.projectName })
      const wide = new Set([...milestones.values()].map((item) => item.projectName)).size > 1
      return keep([
        ...[...milestones.entries()]
          .sort((a, b) => a[1].name.localeCompare(b[1].name))
          .map(([id, item]) => make(`milestone:${id}`, item.name, visible.filter((ticket) => ticket.milestones.some((m) => m.id === id)), wide ? item.projectName : undefined)),
        make('milestone:none', context.labels.noMilestone, visible.filter((ticket) => ticket.milestones.length === 0)),
      ])
    }
    case 'PRIORITY':
      return keep([
        ...PRIORITY_ORDER.map((priority) => make(`priority:${priority}`, context.labels.priority(priority), visible.filter((ticket) => ticket.priority === priority))),
        make('priority:none', context.labels.noPriority, visible.filter((ticket) => !ticket.priority || !PRIORITY_ORDER.includes(ticket.priority))),
      ])
    case 'ASSIGNEE': {
      const named = new Map<string, string>((context.members ?? []).map((member) => [member.userId, member.displayName]))
      for (const ticket of visible) if (ticket.assigneeId && !named.has(ticket.assigneeId)) named.set(ticket.assigneeId, ticket.assigneeName ?? ticket.assigneeId)
      const people = [...named.entries()].sort((a, b) => a[1].localeCompare(b[1])).map(([id, name]) => make(`assignee:${id}`, name, visible.filter((ticket) => ticket.assigneeId === id)))
      return keep([...people, make('assignee:none', context.labels.unassigned, visible.filter((ticket) => ticket.assigneeId === null))])
    }
    case 'SPRINT':
    default: {
      // The backlog first, then each Sprint that has a Ticket here.
      const sprints = new Map<string, { name: string; projectName: string }>()
      for (const ticket of visible) if (ticket.sprint) sprints.set(ticket.sprint.id, { name: ticket.sprint.name, projectName: ticket.projectName })
      const wide = new Set([...sprints.values()].map((item) => item.projectName)).size > 1
      const order = new Map((context.sprints ?? []).map((sprint, index) => [sprint.id, index]))
      return keep([
        make('sprint:backlog', context.labels.backlog, visible.filter((ticket) => ticket.sprint === null)),
        ...[...sprints.entries()]
          .sort((a, b) => (order.get(a[0]) ?? 0) - (order.get(b[0]) ?? 0) || a[1].name.localeCompare(b[1].name))
          .map(([id, item]) => make(`sprint:${id}`, item.name, visible.filter((ticket) => ticket.sprint?.id === id), wide ? item.projectName : undefined)),
      ])
    }
  }
}

/** The Table columns a View shows, in the order of the full set. */
export function tableColumns(definition: ViewDefinition, wide: boolean): TableColumn[] {
  const chosen = definition.display.columns.length > 0 ? definition.display.columns : DEFAULT_TABLE_COLUMNS
  const set = new Set<TableColumn>(chosen)
  // A scope that mixes Projects always shows which Project a Ticket belongs to.
  if (wide) set.add('PROJECT')
  return TABLE_COLUMNS.filter((column) => set.has(column))
}
