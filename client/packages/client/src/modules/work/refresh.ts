import type { QueryClient } from '@tanstack/react-query'
import type { Ticket } from '@penfii/contracts'
import { queryKeys } from '../../queryKeys'

/** What the feed says kind of change a Ticket had. Comments and files change what a Ticket's tabs show, not the Ticket. */
const COMMENT_KINDS = new Set(['ticket.commented', 'ticket.comment_edited', 'ticket.comment_deleted'])
const FILE_KINDS = new Set(['ticket.file_attached', 'ticket.file_detached'])

export type TicketChange = { ticketId: string; kind?: string; revisionNo?: number }

/**
 * Revisions of Tickets this tab changed itself, as the server answered them. The feed announces the same change a moment
 * later; that echo carries nothing the answer did not, so it is not read again.
 */
const ownRevisions = new Map<string, number>()
const OWN_REMEMBERED = 200

export function noteOwnChange(ticketId: string, revisionNo: number): void {
  ownRevisions.delete(ticketId)
  ownRevisions.set(ticketId, revisionNo)
  if (ownRevisions.size > OWN_REMEMBERED) {
    const oldest = ownRevisions.keys().next().value
    if (oldest !== undefined) ownRevisions.delete(oldest)
  }
}

export function isOwnEcho(change: TicketChange): boolean {
  const own = ownRevisions.get(change.ticketId)
  const kind = change.kind ?? ''
  if (own === undefined || change.revisionNo === undefined || COMMENT_KINDS.has(kind) || FILE_KINDS.has(kind)) return false
  return change.revisionNo <= own
}

/**
 * While a card is being moved the Board on screen shows where it is going. A read of the Board that was started before the
 * server has answered would put it back; so Board reads are held back until the move is over, then done once.
 */
const heldBoards = new Map<string, { holds: number; stale: boolean; queryClient: QueryClient }>()

/** The release reads the Board once if a read was held back, unless the caller is about to read it anyway (`flush: false`). */
export function holdBoardRefresh(queryClient: QueryClient, projectId: string): (flush?: boolean) => void {
  const entry = heldBoards.get(projectId) ?? { holds: 0, stale: false, queryClient }
  entry.holds += 1
  heldBoards.set(projectId, entry)
  let released = false
  return (flush = true) => {
    if (released) return
    released = true
    entry.holds -= 1
    if (entry.holds > 0) return
    heldBoards.delete(projectId)
    if (flush && entry.stale) void entry.queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
  }
}

/** Whether a Board read is to be held back now (and remembered for later). */
function boardHeld(projectId: string): boolean {
  const entry = heldBoards.get(projectId)
  if (!entry) return false
  entry.stale = true
  return true
}

function ticketOf(queryClient: QueryClient, ticketId: string): Ticket | undefined {
  return queryClient.getQueryData<Ticket>(queryKeys.ticket(ticketId))
}

/** The queries about one Ticket, by what changed. Another Ticket is touched only where it shows this one: its parent. */
function ticketQueries(queryClient: QueryClient, change: TicketChange): Array<readonly unknown[]> {
  const { ticketId } = change
  const kind = change.kind ?? ''
  if (COMMENT_KINDS.has(kind)) return [queryKeys.ticketComments(ticketId), queryKeys.ticketActivity(ticketId)]
  if (FILE_KINDS.has(kind)) return [queryKeys.ticketAttachments(ticketId), queryKeys.ticketActivity(ticketId)]
  const parentId = ticketOf(queryClient, ticketId)?.parentId
  const own: Array<readonly unknown[]> = [
    queryKeys.ticket(ticketId),
    queryKeys.ticketChildren(ticketId),
    queryKeys.ticketActivity(ticketId),
    queryKeys.ticketHistory(ticketId),
    queryKeys.all.ticketRevisions(ticketId),
    // A dependency shows on both ends, and the other end is not known here.
    queryKeys.all.ticketDependencies,
  ]
  // A child's change also moves the parent's progress and the parent's child list.
  if (parentId) own.push(queryKeys.ticket(parentId), queryKeys.ticketChildren(parentId))
  // Called without a kind (a change this tab made): everything about the Ticket may differ.
  if (!change.kind) own.push(queryKeys.ticketComments(ticketId), queryKeys.ticketAttachments(ticketId))
  return own
}

/** The queries of the Project and of the places that list Tickets, which a change of a Ticket field may move. */
function listQueries(projectId: string): Array<readonly unknown[]> {
  return [
    queryKeys.projectBoard(projectId),
    queryKeys.projectPlanning(projectId),
    // The Timeline draws schedules, assignees and dependencies from its own list.
    queryKeys.projectSchedule(projectId),
    // Team and Workspace scopes show the same Tickets, and a Sprint's list is built from them.
    queryKeys.all.scopeDataset,
    queryKeys.all.sprintResults,
    queryKeys.all.favorites,
    queryKeys.all.later,
  ]
}

/**
 * Marks stale what shows these changed Tickets, each query once however many Tickets changed (a bulk change of a hundred
 * is one read of the Board, not a hundred). Comment and file changes touch only the Ticket's own tabs.
 */
export function refreshTicketChanges(queryClient: QueryClient, projectId: string, changes: readonly TicketChange[]): Promise<unknown> {
  const keys = new Map<string, readonly unknown[]>()
  const add = (key: readonly unknown[]) => keys.set(JSON.stringify(key), key)
  for (const change of changes) {
    for (const key of ticketQueries(queryClient, change)) add(key)
    const kind = change.kind ?? ''
    if (!COMMENT_KINDS.has(kind) && !FILE_KINDS.has(kind)) for (const key of listQueries(projectId)) add(key)
  }
  const board = JSON.stringify(queryKeys.projectBoard(projectId))
  if (keys.has(board) && boardHeld(projectId)) keys.delete(board)
  return Promise.all([...keys.values()].map((queryKey) => queryClient.invalidateQueries({ queryKey })))
}

/** Marks everything that shows this Ticket as stale after this tab changed it. */
export function refreshTicket(queryClient: QueryClient, ticketId: string, projectId: string): Promise<unknown> {
  return refreshTicketChanges(queryClient, projectId, [{ ticketId }])
}

/**
 * Everything a Project's screens show may be out of date (the live feed could not say what changed, or the person
 * was away): every query of the Project and of its Tickets is marked stale, and the ones on screen are read again.
 * This is the answer to `project.resync_required`; a change that names its Ticket uses `refreshTicket`.
 */
export function refreshProject(queryClient: QueryClient, projectId: string): Promise<unknown> {
  return Promise.all([
    boardHeld(projectId) ? undefined : queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectPlanning(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectSchedule(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectCalendar(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectWorkflow(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectStatusSummary(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectVelocity(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectLabels(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectMembers(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectTypes(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectArchive(projectId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectViews(projectId) }),
    // Tickets are keyed by their own id, not by Project: every loaded one, with what hangs off it.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticket }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketChildren }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketDependencies }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketActivity }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketComments }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketAttachments }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketHistory }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketRevision }),
    // Sprint lists and reports, and the Team and Workspace scopes that show the same Tickets.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintResults }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintReport }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintPreview }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintArchiveSchedule }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.scopeDataset }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.favorites }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.later }),
  ])
}
