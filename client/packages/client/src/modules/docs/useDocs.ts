import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { DocVisibility } from '@penfii/contracts'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'

export function useDocTree(workspaceId: string, projectId?: string | null) {
  return useQuery(queries.docTree(workspaceId, projectId))
}

/** Commands that change the tree; each one refreshes every tree on screen. */
export function useDocTreeCommands(workspaceId: string) {
  const queryClient = useQueryClient()
  const refresh = () => queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) })
  return {
    create: useMutation({
      mutationFn: (input: { title: string; parentId?: string | null; projectId?: string | null; visibility?: DocVisibility; scopeProjectId?: string | null; scopeTeamId?: string | null; templateId?: string }) =>
        api.createDoc(workspaceId, { visibility: 'WORKSPACE', ...input }),
      onSuccess: refresh,
    }),
    rename: useMutation({
      mutationFn: ({ id, title }: { id: string; title: string }) => api.updateDoc(id, { title }),
      onSuccess: async (page) => {
        queryClient.setQueryData(queryKeys.doc(page.id), page)
        await refresh()
      },
    }),
    move: useMutation({
      mutationFn: ({ id, parentId, beforeId }: { id: string; parentId: string | null; beforeId?: string | null }) =>
        api.moveDoc(id, { parentId, beforeId }),
      onSuccess: refresh,
    }),
    remove: useMutation({
      mutationFn: (id: string) => api.deleteDoc(id),
      onSuccess: refresh,
    }),
  }
}

export function ruleCode(error: unknown): string | null {
  return isApiError(error) && error.status === 422 ? error.code : null
}
