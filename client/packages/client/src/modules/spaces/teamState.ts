import type { QueryClient } from '@tanstack/react-query'
import type { Team } from '@penfii/contracts'
import { queryKeys } from '../../queryKeys'

/** A Team is archived while `archivedAt` is set. Archived Teams are readable but hidden from default lists and pickers. */
export function isArchivedTeam(team: Pick<Team, 'archivedAt'> | undefined | null): boolean {
  return Boolean(team?.archivedAt)
}

export function activeTeams<T extends Pick<Team, 'archivedAt'>>(teams: T[]): T[] {
  return teams.filter((team) => !isArchivedTeam(team))
}

/** Everything a change of Team membership or state touches: the Teams, the people list, invitations and the audit log. */
export async function refreshTeamData(queryClient: QueryClient, workspaceId: string) {
  await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
  await queryClient.invalidateQueries({ queryKey: queryKeys.people(workspaceId) })
  await queryClient.invalidateQueries({ queryKey: queryKeys.adminInvitations(workspaceId) })
  await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
}
