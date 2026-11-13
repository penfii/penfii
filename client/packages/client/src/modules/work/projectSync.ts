import type { QueryClient } from '@tanstack/react-query'
import { isOwnEcho, refreshProject, refreshTicketChanges, type TicketChange } from './refresh'
import { queryKeys } from '../../queryKeys'

/**
 * How a client keeps one Project's screens in step with the server.
 *
 * The server numbers every recorded change of a Project (`changeSeq`, no gaps, in commit order). The client remembers
 * the last number it holds (`lastSeq`) and sends it when it connects:
 *  - the server replays what came after it (as ordinary `project.ticket.change` frames), then sends `project.ready`
 *    with the head of the feed (`headSeq`); the client then holds everything up to the head;
 *  - when the server cannot say what happened (a Sprint, Milestone or workflow change, a position it does not know,
 *    too far behind, a socket that lagged) it sends `project.resync_required`, and the client reads everything of the
 *    Project again (`refreshProject`).
 * A change record exists only for Ticket changes; the changes it cannot carry are announced as `resync_required`.
 * A frame whose number is more than one past the last held (`lastSeq`) means something between was not delivered: the
 * Project is read as a whole again. (A hole can also come from a Ticket this person may not see, which the server
 * leaves out; that costs one extra read, never a wrong screen.)
 */
export type SyncState = {
  /** The last position of the Project's feed this client holds (0: none yet). */
  lastSeq: number
  /** `project.resync_required` came: the next `project.ready` names the position to start from again. */
  rebaseline: boolean
  /** A connection reached `project.ready` before. */
  connectedBefore: boolean
  /** The position the current connection asked to continue from. */
  askedAfter: number
  /** Ticket changes that arrived within the last moment and are read together. */
  pending: Map<string, TicketChange>
  flushTimer: number
}

/** Frames of a burst (a bulk change, a replay) are read together once this long has passed without another. */
export const BATCH_MS = 150

export function newSyncState(): SyncState {
  return { lastSeq: 0, rebaseline: false, connectedBefore: false, askedAfter: 0, pending: new Map(), flushTimer: 0 }
}

/** What to send as soon as the socket opens. */
export function helloFor(state: SyncState): { v: 1; type: 'hello'; afterChangeSeq: number } {
  state.askedAfter = state.lastSeq
  return { v: 1, type: 'hello', afterChangeSeq: state.lastSeq }
}

/** Reads what the changes collected so far touch, each query once. */
export function flushChanges(queryClient: QueryClient, projectId: string, state: SyncState): void {
  window.clearTimeout(state.flushTimer)
  state.flushTimer = 0
  if (state.pending.size === 0) return
  const changes = [...state.pending.values()]
  state.pending.clear()
  void refreshTicketChanges(queryClient, projectId, changes)
}

/** Forgets what was collected and not yet read (the Project is read as a whole instead, or nobody looks any more). */
export function dropChanges(state: SyncState): void {
  window.clearTimeout(state.flushTimer)
  state.flushTimer = 0
  state.pending.clear()
}

type Frame = {
  type?: unknown
  projectId?: unknown
  ticketId?: unknown
  changeSeq?: unknown
  headSeq?: unknown
  changeKind?: unknown
  ticketRevisionNo?: unknown
}

/** Applies one frame of a Project socket to the cache. Frames of another Project and malformed frames do nothing. */
export function handleProjectFrame(queryClient: QueryClient, projectId: string, state: SyncState, data: unknown): void {
  if (typeof data !== 'object' || data === null) return
  const frame = data as Frame
  if (frame.projectId !== projectId) return
  const seq = typeof frame.changeSeq === 'number' ? frame.changeSeq : null
  if (frame.type === 'project.ticket.change' && typeof frame.ticketId === 'string') {
    // Held already (a replay and a live frame can both carry it).
    if (seq !== null && seq <= state.lastSeq) return
    // A hole is only meaningful once there is a position to compare with (before `project.ready` the first number is the start).
    const gap = seq !== null && state.connectedBefore && state.lastSeq > 0 && seq > state.lastSeq + 1
    if (seq !== null) state.lastSeq = seq
    if (gap) {
      dropChanges(state)
      void refreshProject(queryClient, projectId)
      return
    }
    const change: TicketChange = {
      ticketId: frame.ticketId,
      kind: typeof frame.changeKind === 'string' ? frame.changeKind : undefined,
      revisionNo: typeof frame.ticketRevisionNo === 'number' ? frame.ticketRevisionNo : undefined,
    }
    if (isOwnEcho(change)) return
    // A later change of the same Ticket says at least what an earlier one did, except that comment and file changes
    // are about other tabs: those are kept apart.
    state.pending.set(`${change.ticketId}:${change.kind ?? ''}`, change)
    if (!state.flushTimer) state.flushTimer = window.setTimeout(() => flushChanges(queryClient, projectId, state), BATCH_MS)
  } else if (frame.type === 'project.resync_required') {
    state.rebaseline = true
    dropChanges(state)
    void refreshProject(queryClient, projectId)
  } else if (frame.type === 'project.ready') {
    const head = typeof frame.headSeq === 'number' ? frame.headSeq : null
    if (!state.connectedBefore) {
      // The page read its data before the socket opened, and a Sprint, Milestone or workflow change in between has no
      // record to replay: read what the Project's screens show once more, so nothing in between is missed.
      for (const key of [queryKeys.projectBoard(projectId), queryKeys.projectPlanning(projectId), queryKeys.projectSchedule(projectId), queryKeys.projectCalendar(projectId), queryKeys.projectWorkflow(projectId)]) {
        void queryClient.invalidateQueries({ queryKey: key })
      }
    } else if (state.askedAfter === 0 && !state.rebaseline) {
      // Reconnected without a position to continue from: nothing was replayed, so read the Project again.
      void refreshProject(queryClient, projectId)
    } else if (head !== null && head < state.lastSeq && !state.rebaseline) {
      // The server's feed is behind what this client holds (a restored database): it cannot be trusted.
      void refreshProject(queryClient, projectId)
      state.rebaseline = true
    }
    if (head !== null) state.lastSeq = state.rebaseline ? head : Math.max(state.lastSeq, head)
    state.rebaseline = false
    state.connectedBefore = true
  }
}
