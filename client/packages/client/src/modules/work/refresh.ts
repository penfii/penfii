import type { QueryClient } from '@tanstack/react-query'
import { queryKeys } from '../../queryKeys'

/** Marks everything that shows this Ticket as stale after it changed. */
export function refreshTicket(queryClient: QueryClient, ticketId: string, projectId: string): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }),
    // Every loaded ticket: a change can move a parent's child progress too.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticket }),
    queryClient.invalidateQueries({ queryKey: queryKeys.ticketActivity(ticketId) }),
    // A comment added, edited or deleted by someone else arrives through the same feed.
    queryClient.invalidateQueries({ queryKey: queryKeys.ticketComments(ticketId) }),
    // Files on the Ticket body added or removed by someone else arrive the same way.
    queryClient.invalidateQueries({ queryKey: queryKeys.ticketAttachments(ticketId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.ticketHistory(ticketId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketRevisions(ticketId) }),
    // A child's change also moves the parent's progress and the parent's child list.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketChildren }),
    // A dependency, Sprint or Milestone change shows on both ends and in the planning lists.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.ticketDependencies }),
    queryClient.invalidateQueries({ queryKey: queryKeys.projectPlanning(projectId) }),
    // The Timeline draws schedules, assignees and dependencies from its own list.
    queryClient.invalidateQueries({ queryKey: queryKeys.projectSchedule(projectId) }),
    // Team and Workspace scopes show the same Tickets, and a Sprint's list is built from them.
    queryClient.invalidateQueries({ queryKey: queryKeys.all.scopeDataset }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintResults }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.favorites }),
    queryClient.invalidateQueries({ queryKey: queryKeys.all.later }),
  ])
}
