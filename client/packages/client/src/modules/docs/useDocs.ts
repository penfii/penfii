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
  // A starred page is named in Saved too: its title and place come from the same server read.
  const refresh = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) }),
    queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) }),
  ])
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
  }
}

export function ruleCode(error: unknown): string | null {
  return isApiError(error) && error.status === 422 ? error.code : null
}
