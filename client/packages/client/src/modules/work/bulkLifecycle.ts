import { useState } from 'react'
import { newIdempotencyKey } from '@penfii/api-client'
import type { BulkChange, BulkItemResult, BulkResult, ProjectBoard, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { openChildCount } from './ticketModel'

/** The server takes at most this many Tickets in one bulk request; the split is invisible to the person. */
export const BULK_REQUEST_LIMIT = 100

/** How many Tickets an Undo puts back at the same time. */
export const RESTORE_CONCURRENCY = 5

export type LifecycleAction = 'ARCHIVE' | 'TRASH'

/**
 * The chosen Tickets that are still Completed or Canceled on this Board. A card chosen earlier that has since
 * moved on (or left the Board) is dropped, so an Archive or Trash never reaches work that is not finished: the
 * server takes any State, only the Board offers this choice.
 */
export function finishedSelection(chosen: Set<string>, board: ProjectBoard): string[] {
  const finishedStates = new Set(board.states.filter((state) => state.category === 'DONE' || state.category === 'CANCELED').map((state) => state.id))
  const finished = new Set(board.columns.flatMap((column) => column.tickets).filter((ticket) => finishedStates.has(ticket.statusId)).map((ticket) => ticket.id))
  return [...chosen].filter((id) => finished.has(id))
}

/** A Ticket with Tasks or Subtasks that are neither Completed nor Canceled cannot be archived yet. */
export function hasOpenChildren(ticket: Pick<Ticket, 'childSummary'>): boolean {
  return openChildCount(ticket.childSummary) > 0
}

export type ArchivePlan = {
  /** What an Archive would send: finished, with nothing unfinished below it, in a Project the person manages. */
  ids: string[]
  /** Chosen but not Completed or Canceled (only a Table can choose these). */
  unfinished: number
  /** Finished but with unfinished children. */
  openChildren: number
  /** Finished in a Project where the person is not an administrator. */
  noPermission: number
}

/**
 * How many of the chosen Tickets an Archive would really take, and why the others stay out. This is guidance for the
 * person; the server still decides for every Ticket (permission, State, unfinished children) when the Archive runs.
 */
export function archivePlan(
  tickets: Array<Pick<Ticket, 'id' | 'childSummary' | 'projectId'> & { finished: boolean }>,
  canManage: (projectId: string) => boolean,
): ArchivePlan {
  const plan: ArchivePlan = { ids: [], unfinished: 0, openChildren: 0, noPermission: 0 }
  for (const ticket of tickets) {
    if (!ticket.finished) plan.unfinished += 1
    else if (!canManage(ticket.projectId)) plan.noPermission += 1
    else if (hasOpenChildren(ticket)) plan.openChildren += 1
    else plan.ids.push(ticket.id)
  }
  return plan
}

export function chunk<T>(items: T[], size: number): T[][] {
  const out: T[][] = []
  for (let at = 0; at < items.length; at += size) out.push(items.slice(at, at + size))
  return out
}

/**
 * Archive or trash the chosen Tickets as one operation. They are sent in requests of at most
 * BULK_REQUEST_LIMIT; if a request fails, the Tickets of it and of every later one are reported as not
 * processed. The result covers every chosen Ticket, so a partial run is never summed into a success.
 */
export async function runLifecycle(
  workspaceId: string,
  action: LifecycleAction,
  ids: string[],
  progress: (done: number) => void = () => undefined,
): Promise<BulkResult> {
  const items: BulkItemResult[] = []
  const batches = chunk(ids, BULK_REQUEST_LIMIT)
  for (const [index, batch] of batches.entries()) {
    try {
      // An Archive from these screens is only for finished work: the server checks that again as it processes each Ticket,
      // so one that was reopened after it was chosen is refused instead of archived.
      const change: BulkChange = action === 'ARCHIVE' ? { type: 'ARCHIVE', onlyFinished: true } : { type: 'TRASH' }
      const result = await api.bulkTickets(workspaceId, batch, change, newIdempotencyKey())
      items.push(...result.items)
      progress(items.length)
    } catch {
      for (const later of batches.slice(index)) {
        for (const ticketId of later) items.push({ ticketId, key: null, ok: false, code: 'not_processed', message: null })
      }
      break
    }
  }
  const succeeded = items.filter((item) => item.ok).length
  return { total: items.length, succeeded, failed: items.length - succeeded, items }
}

/** Puts trashed Tickets back (the Undo of a bulk trash). Each one is its own request; the count of those that did not come back is reported. */
export function restoreFromTrash(ids: string[]): Promise<{ ok: number; failed: number }> {
  return putBack(ids, (id) => api.restoreTicketFromTrash(id))
}

/** Takes archived Tickets out of the archive (the Undo of a bulk archive). */
export function restoreArchived(ids: string[]): Promise<{ ok: number; failed: number }> {
  return putBack(ids, (id) => api.unarchiveTicket(id))
}

async function putBack(ids: string[], restore: (id: string) => Promise<unknown>): Promise<{ ok: number; failed: number }> {
  let ok = 0
  let next = 0
  // At most RESTORE_CONCURRENCY requests at a time: three hundred Tickets must not be three hundred requests at once.
  const worker = async () => {
    while (next < ids.length) {
      const id = ids[next]
      next += 1
      if (id === undefined) return
      try {
        await restore(id)
        ok += 1
      } catch {
        // Counted as failed below.
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(RESTORE_CONCURRENCY, ids.length) }, worker))
  return { ok, failed: ids.length - ok }
}

export function useBulkLifecycle(workspaceId: string, onSettled: (result: BulkResult) => void) {
  const [running, setRunning] = useState<{ done: number; total: number } | null>(null)
  const [result, setResult] = useState<BulkResult | null>(null)
  const [last, setLast] = useState<LifecycleAction | null>(null)

  async function run(action: LifecycleAction, ids: string[]): Promise<BulkResult | undefined> {
    if (running || ids.length === 0) return undefined
    setLast(action)
    setResult(null)
    setRunning({ done: 0, total: ids.length })
    const outcome = await runLifecycle(workspaceId, action, ids, (done) => setRunning({ done, total: ids.length }))
    setRunning(null)
    setResult(outcome)
    onSettled(outcome)
    return outcome
  }

  return { run, running, result, last, clear: () => setResult(null) }
}
