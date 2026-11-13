import { useCallback, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { EstimateInput, ScheduleInput, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { noteOwnChange, refreshTicket } from './refresh'
import { errorCode, errorDetails } from './ticketModel'

export type TicketPatch = {
  title?: string
  description?: unknown
  assigneeId?: string | null
  priority?: string | null
  dueOn?: string | null
  labelIds?: string[]
  parentId?: string | null
  typeId?: string | null
  /** `null` clears the estimate. */
  estimate?: EstimateInput | null
  /** `null` clears the schedule; a schedule replaces the whole schedule. */
  schedule?: ScheduleInput | null
}

export type TicketConflict = { patch: TicketPatch; latest: Ticket; again: boolean }

export type EditFailure = 'save' | 'invalidValue' | 'unfinished' | 'notAllowed' | 'move' | 'moveConflict' | 'blocked' | 'blockedCanceled'

/**
 * Edits one Ticket against the revision the person read. A stale revision never overwrites:
 * the attempted change is kept together with the latest ticket so both can be compared.
 */
export function useTicketEditor(ticket: Ticket | undefined) {
  const queryClient = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<EditFailure | null>(null)
  const [conflict, setConflict] = useState<TicketConflict | null>(null)

  const store = useCallback(
    (updated: Ticket) => {
      queryClient.setQueryData(queryKeys.ticket(updated.id), updated)
      // The answer is what this tab changed; the feed's echo of it need not be read again. The lists are read in the
      // background: the controls are free as soon as the server has answered.
      noteOwnChange(updated.id, updated.revisionNo)
      void refreshTicket(queryClient, updated.id, updated.projectId)
    },
    [queryClient],
  )

  const loadLatest = useCallback(
    async (current: Ticket, patch: TicketPatch, again: boolean) => {
      const latest = await api.getTicket(current.id)
      queryClient.setQueryData(queryKeys.ticket(current.id), latest)
      void refreshTicket(queryClient, current.id, current.projectId)
      setConflict({ patch, latest, again })
    },
    [queryClient],
  )

  const send = useCallback(
    async (current: Ticket, patch: TicketPatch, again: boolean): Promise<boolean> => {
      setBusy(true)
      setFailure(null)
      try {
        const updated = await api.updateTicket(current.id, { ...patch, expectedRevision: current.revisionNo }, newIdempotencyKey())
        setConflict(null)
        store(updated)
        return true
      } catch (cause) {
        const code = errorCode(cause)
        if (code === 'version_conflict') {
          try {
            await loadLatest(current, patch, again)
          } catch {
            setFailure('save')
          }
        } else if (code === 'estimate_not_allowed' || code === 'estimate_unit_mismatch' || code === 'schedule_invalid') {
          // The value was understood and the data does not allow it; the person fixes the value.
          setFailure('invalidValue')
        } else {
          setFailure('save')
        }
        return false
      } finally {
        setBusy(false)
      }
    },
    [loadLatest, store],
  )

  const save = useCallback((patch: TicketPatch) => (ticket ? send(ticket, patch, false) : Promise.resolve(false)), [send, ticket])

  /** Re-applies the kept change on top of the latest ticket the person has now seen. */
  const reapply = useCallback(async () => {
    if (!conflict) return false
    return send(conflict.latest, conflict.patch, true)
  }, [conflict, send])

  const discard = useCallback(() => setConflict(null), [])

  const changeStatus = useCallback(
    async (statusId: string): Promise<boolean> => {
      if (!ticket || statusId === ticket.statusId) return true
      setBusy(true)
      setFailure(null)
      try {
        const updated = await api.changeTicketStatus(ticket.id, { expectedRevision: ticket.revisionNo, statusId }, newIdempotencyKey())
        store(updated)
        return true
      } catch (cause) {
        const code = errorCode(cause)
        setFailure(code === 'unfinished_subtasks' ? 'unfinished' : code === 'dependency_blocked' ? (Number(errorDetails(cause).canceledBlockers ?? 0) > 0 ? 'blockedCanceled' : 'blocked') : code === 'version_conflict' ? 'moveConflict' : code === 'forbidden' ? 'notAllowed' : 'move')
        void refreshTicket(queryClient, ticket.id, ticket.projectId)
        return false
      } finally {
        setBusy(false)
      }
    },
    [queryClient, store, ticket],
  )

  /** Runs a command that returns the changed Ticket (Sprint, Milestones) and shows its result. */
  const perform = useCallback(
    async (action: () => Promise<Ticket>): Promise<boolean> => {
      if (!ticket) return false
      setBusy(true)
      setFailure(null)
      try {
        store(await action())
        return true
      } catch {
        setFailure('save')
        void refreshTicket(queryClient, ticket.id, ticket.projectId)
        return false
      } finally {
        setBusy(false)
      }
    },
    [queryClient, store, ticket],
  )

  return { busy, failure, conflict, save, reapply, discard, changeStatus, perform, clearFailure: () => setFailure(null) }
}
