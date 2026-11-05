import type { ChildSummary, HierarchyKind, ProjectBoard, Ticket, TicketField } from '@penfii/contracts'

export const PRIORITIES = ['LOW', 'MEDIUM', 'HIGH', 'URGENT'] as const
export type Priority = (typeof PRIORITIES)[number]

export function isPriority(value: string | null): value is Priority {
  return PRIORITIES.some((item) => item === value)
}

/** The kind a child of this kind has. Subtasks have no children. */
export function childKind(kind: HierarchyKind): HierarchyKind | null {
  if (kind === 'EPIC') return 'TASK'
  if (kind === 'TASK') return 'SUBTASK'
  return null
}

/** The kind a parent of this kind must have. Epics have no parent. */
export function parentKind(kind: HierarchyKind): HierarchyKind | null {
  if (kind === 'SUBTASK') return 'TASK'
  if (kind === 'TASK') return 'EPIC'
  return null
}

const TEXT_BLOCKS = new Set(['paragraph', 'heading', 'codeBlock'])

/** Text of a rich text document, one line per text block. */
export function descriptionText(value: unknown): string {
  const lines: string[] = []
  const walk = (node: unknown, line: string[]) => {
    if (!node || typeof node !== 'object') return
    const item = node as { type?: unknown; text?: unknown; content?: unknown }
    if (typeof item.text === 'string') line.push(item.text)
    const children = Array.isArray(item.content) ? item.content : []
    if (typeof item.type === 'string' && TEXT_BLOCKS.has(item.type)) {
      const inner: string[] = []
      for (const child of children) walk(child, inner)
      lines.push(inner.join(''))
      return
    }
    for (const child of children) walk(child, line)
  }
  walk(value, [])
  return lines.join('\n').replace(/\s+$/, '')
}

/** A document with one paragraph per line, so line breaks survive a save. */
export function textToDescription(text: string): { type: 'doc'; content: unknown[] } {
  const trimmed = text.replace(/\s+$/, '')
  if (!trimmed) return { type: 'doc', content: [] }
  return {
    type: 'doc',
    content: trimmed.split('\n').map((line) => (line ? { type: 'paragraph', content: [{ type: 'text', text: line }] } : { type: 'paragraph' })),
  }
}

export function progressPercent(summary: ChildSummary): number {
  const counted = summary.total - summary.canceled
  if (counted <= 0) return 0
  return Math.round((summary.done / counted) * 100)
}

/**
 * Tickets a ticket of `kind` may sit under. A Task goes under an Epic; a Subtask goes under a
 * Task in the same channel. `selfId` leaves out the ticket being edited.
 */
export function parentCandidates(kind: HierarchyKind, channelId: string | null, selfId: string | null, board: ProjectBoard): Ticket[] {
  const wanted = parentKind(kind)
  if (!wanted) return []
  return board.columns
    .flatMap((column) => column.tickets)
    .filter((item) => item.hierarchyKind === wanted && item.id !== selfId)
    .filter((item) => kind !== 'SUBTASK' || item.channelId === channelId)
}

export function eligibleParents(ticket: Ticket, board: ProjectBoard): Ticket[] {
  return parentCandidates(ticket.hierarchyKind, ticket.channelId, ticket.id, board)
}

export function allTickets(board: ProjectBoard): Ticket[] {
  return board.columns.flatMap((column) => column.tickets)
}

export function isOverdue(dueOn: string | null, categoryDone: boolean, today = new Date()): boolean {
  if (!dueOn || categoryDone) return false
  const key = `${String(today.getFullYear())}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`
  return dueOn < key
}

export function formatDue(dueOn: string, locale: string): string {
  const [year, month, day] = dueOn.split('-').map(Number)
  if (!year || !month || !day) return dueOn
  return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(new Date(year, month - 1, day))
}

const FIELD_ORDER: TicketField[] = ['title', 'description', 'status', 'type', 'priority', 'assignee', 'dueOn', 'labels', 'parent', 'estimate', 'schedule']

export function sortFields(fields: TicketField[]): TicketField[] {
  return [...fields].sort((a, b) => FIELD_ORDER.indexOf(a) - FIELD_ORDER.indexOf(b))
}

export function errorCode(cause: unknown): string {
  return cause instanceof Error && 'code' in cause && typeof cause.code === 'string' ? cause.code : ''
}

export function errorDetails(cause: unknown): Record<string, unknown> {
  if (cause instanceof Error && 'details' in cause && cause.details && typeof cause.details === 'object') {
    return cause.details as Record<string, unknown>
  }
  return {}
}
