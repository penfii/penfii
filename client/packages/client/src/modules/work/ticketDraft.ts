import type { HierarchyKind } from '@penfii/contracts'

/**
 * The unsent "create ticket" form, kept in this browser so closing the panel (Esc, back, a reload) does not lose
 * it. It never goes to the server and never holds files: an upload is deleted when its form goes away, so only the
 * fact that there were attachments is kept (`hadAttachments`).
 *
 * One draft per person, Workspace, Project and origin (a chat message, a parent Ticket, or the Project itself).
 * The sign-out boundary clears every `penfii.*` key, so a draft never reaches the next person on this browser.
 */
const PREFIX = 'penfii.ticketDraft.v1'

/** A draft untouched for this long is dropped when the form is opened. */
export const DRAFT_TTL_MS = 14 * 24 * 60 * 60 * 1000

export type TicketDraft = {
  title: string
  description: string
  kind: HierarchyKind
  parentId: string | null
  channelId: string | null
  assigneeId: string | null
  priority: string | null
  dueOn: string | null
  typeId: string | null
  labelIds: string[]
  estimate: string
  hadAttachments: boolean
  /** Epoch milliseconds of the last edit. */
  updatedAt: number
}

export type DraftOrigin = { messageId?: string; parentId?: string }

export function draftOrigin(origin: DraftOrigin): string {
  if (origin.messageId) return `chat:${origin.messageId}`
  if (origin.parentId) return `parent:${origin.parentId}`
  return 'project'
}

export function ticketDraftKey(userId: string, workspaceId: string, projectId: string, origin: DraftOrigin): string {
  return `${PREFIX}:${userId}:${workspaceId}:${projectId}:${draftOrigin(origin)}`
}

function storage(): Storage | null {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

const KINDS: readonly string[] = ['EPIC', 'TASK', 'SUBTASK']
const text = (value: unknown): string => (typeof value === 'string' ? value : '')
const nullable = (value: unknown): string | null => (typeof value === 'string' && value !== '' ? value : null)

function parse(raw: string): TicketDraft | null {
  try {
    const data = JSON.parse(raw) as Record<string, unknown>
    if (data === null || typeof data !== 'object' || Array.isArray(data) || typeof data.updatedAt !== 'number') return null
    return {
      title: text(data.title),
      description: text(data.description),
      kind: KINDS.includes(text(data.kind)) ? (data.kind as HierarchyKind) : 'TASK',
      parentId: nullable(data.parentId),
      channelId: nullable(data.channelId),
      assigneeId: nullable(data.assigneeId),
      priority: nullable(data.priority),
      dueOn: nullable(data.dueOn),
      typeId: nullable(data.typeId),
      labelIds: Array.isArray(data.labelIds) ? data.labelIds.filter((id): id is string => typeof id === 'string') : [],
      estimate: text(data.estimate),
      hadAttachments: data.hadAttachments === true,
      updatedAt: data.updatedAt,
    }
  } catch {
    return null
  }
}

/** The kept draft, or null when there is none, it is unreadable or it is older than 14 days (then it is removed). */
export function readTicketDraft(key: string, now = Date.now()): TicketDraft | null {
  const store = storage()
  if (!store) return null
  try {
    const raw = store.getItem(key)
    if (!raw) return null
    const draft = parse(raw)
    if (!draft || now - draft.updatedAt > DRAFT_TTL_MS || draft.updatedAt > now + DRAFT_TTL_MS) {
      store.removeItem(key)
      return null
    }
    return draft
  } catch {
    return null
  }
}

/** Returns whether storage took the write. */
export function writeTicketDraft(key: string, draft: TicketDraft): boolean {
  const store = storage()
  if (!store) return false
  try {
    store.setItem(key, JSON.stringify(draft))
    return true
  } catch {
    return false
  }
}

export function clearTicketDraft(key: string): void {
  try {
    storage()?.removeItem(key)
  } catch {
    // A private window can refuse storage. Leaving the key is the same as having none.
  }
}

/** What the form may be restored with: ids that still exist where the form is opened. */
export type DraftValid = {
  labelIds: ReadonlySet<string>
  memberIds: ReadonlySet<string>
  typeIds: ReadonlySet<string>
  ticketIds: ReadonlySet<string>
  channelIds: ReadonlySet<string>
}

/** The draft without values that no longer exist (a deleted label, a removed assignee, a gone parent or channel). */
export function restorable(draft: TicketDraft, valid: DraftValid): { draft: TicketDraft; dropped: boolean } {
  let dropped = false
  const keep = (id: string | null, ids: ReadonlySet<string>): string | null => {
    if (id === null || ids.has(id)) return id
    dropped = true
    return null
  }
  const labelIds = draft.labelIds.filter((id) => {
    if (valid.labelIds.has(id)) return true
    dropped = true
    return false
  })
  return {
    dropped,
    draft: {
      ...draft,
      parentId: keep(draft.parentId, valid.ticketIds),
      channelId: keep(draft.channelId, valid.channelIds),
      assigneeId: keep(draft.assigneeId, valid.memberIds),
      typeId: keep(draft.typeId, valid.typeIds),
      labelIds,
    },
  }
}
