import { useState } from 'react'
import { newIdempotencyKey } from '@penfii/api-client'
import type { BulkItemResult, BulkResult, ProjectBoard } from '@penfii/contracts'
import { api } from '../../api'

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
      const result = await api.bulkTickets(workspaceId, batch, { type: action }, newIdempotencyKey())
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
export async function restoreFromTrash(ids: string[]): Promise<{ ok: number; failed: number }> {
  let ok = 0
  let next = 0
  // At most RESTORE_CONCURRENCY requests at a time: three hundred Tickets must not be three hundred requests at once.
  const worker = async () => {
    while (next < ids.length) {
      const id = ids[next]
      next += 1
      if (id === undefined) return
      try {
        await api.restoreTicketFromTrash(id)
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
