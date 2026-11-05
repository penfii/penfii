import { useQuery } from '@tanstack/react-query'
import { queries } from '../../queries'
import type { ProjectContext } from './projectContext'

/** The Board and option lists (labels, members, types) of a Project, shared by every panel. */
export function useProjectContext(projectId: string) {
  const enabled = projectId.length > 0
  const board = useQuery({ ...queries.projectBoard(projectId), enabled })
  const labels = useQuery({ ...queries.projectLabels(projectId), enabled })
  const members = useQuery({ ...queries.projectMembers(projectId), enabled })
  const types = useQuery({ ...queries.projectTypes(projectId), enabled })
  const planning = useQuery({ ...queries.projectPlanning(projectId), enabled })
  const context: ProjectContext | undefined = board.data
    ? { board: board.data, labels: labels.data ?? [], members: members.data ?? [], types: types.data ?? [], planning: planning.data }
    : undefined
  return { board, context }
}
