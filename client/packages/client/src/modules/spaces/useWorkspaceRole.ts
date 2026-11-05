import { useQuery } from '@tanstack/react-query'
import { queries } from '../../queries'

/** The signed-in person's role in this Workspace (OWNER, ADMIN or MEMBER) once the Workspace list has loaded. */
export function useWorkspaceRole(workspaceId: string): string | undefined {
  const workspaces = useQuery(queries.workspaces())
  return workspaces.data?.find((item) => item.id === workspaceId)?.role
}
