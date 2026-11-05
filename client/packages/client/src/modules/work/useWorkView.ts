import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { ProjectView, ViewDefinition, ViewLayout, ViewTargetKind } from '@penfii/contracts'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { errorCode } from './ticketModel'
import type { ViewSaveError } from './ViewBar'
import { defaultDefinition, defaultLayout, definitionFromParams, fitLayout, layoutFromParams, layoutsFor, paramsWithDraft, paramsWithLayout, sameDefinition } from './viewModel'

/** The place a person is looking from. It decides the Views offered and the layouts available. */
export type WorkTarget =
  | { kind: 'PROJECT'; workspaceId: string; projectId: string }
  | { kind: 'TEAM'; workspaceId: string; teamId: string }
  | { kind: 'WORKSPACE'; workspaceId: string }

export function targetKey(target: WorkTarget): string {
  return target.kind === 'PROJECT' ? target.projectId : target.kind === 'TEAM' ? `team:${target.teamId}` : `workspace:${target.workspaceId}`
}

const LAST_VIEW_KEY = 'penfii.work.view.'

function readLastView(key: string): string | null {
  try {
    return localStorage.getItem(LAST_VIEW_KEY + key)
  } catch {
    return null
  }
}

function writeLastView(key: string, id: string | null) {
  try {
    if (id) localStorage.setItem(LAST_VIEW_KEY + key, id)
    else localStorage.removeItem(LAST_VIEW_KEY + key)
  } catch {
    // The choice still applies for this visit.
  }
}

function viewError(cause: unknown): ViewSaveError {
  const code = errorCode(cause)
  if (code === 'view_name_taken') return 'nameTaken'
  if (code === 'version_conflict') return 'conflict'
  if (code === 'forbidden') return 'forbidden'
  return 'generic'
}

export function viewsQueryKey(target: WorkTarget): readonly unknown[] {
  return target.kind === 'PROJECT' ? queryKeys.projectViews(target.projectId) : queryKeys.wideViews(target.kind, targetKey(target))
}

/**
 * The state of a View in one place: the saved Views, which one is open, and the unsaved draft.
 * The draft lives in the URL (definition and layout), so a refresh and a shared link keep it, and
 * browsing never changes a saved or shared View. The place decides everything else: a Project
 * offers Board, List and Table; a Team or the Workspace offer List and Table.
 */
export function useWorkView(target: WorkTarget, onMissing: () => void) {
  const queryClient = useQueryClient()
  const [params, setParams] = useSearchParams()
  const kind: ViewTargetKind = target.kind
  const key = targetKey(target)
  const views = useQuery({
    queryKey: viewsQueryKey(target),
    queryFn: () =>
      target.kind === 'PROJECT'
        ? api.getProjectViews(target.projectId)
        : target.kind === 'TEAM'
          ? api.getWideViews(target.workspaceId, { kind: 'TEAM', teamId: target.teamId })
          : api.getWideViews(target.workspaceId, { kind: 'WORKSPACE' }),
  })
  const viewList: ProjectView[] = views.data ?? []
  const activeParam = params.get('v')
  const active = viewList.find((view) => view.id === activeParam)
  const baseLayout: ViewLayout = active?.layout && layoutsFor(kind).includes(active.layout) ? active.layout : defaultLayout(kind)
  const base: ViewDefinition = active?.definition ?? defaultDefinition(kind)
  const paramText = params.toString()
  const [draft, setLocalDraft] = useState<ViewDefinition | null>(() => definitionFromParams(params, defaultDefinition(kind).group))
  const layout: ViewLayout = layoutFromParams(params, kind) ?? baseLayout
  const definition: ViewDefinition = fitLayout(draft ?? base, kind, layout)
  const definitionDirty = draft !== null && !sameDefinition(draft, base)
  const dirty = definitionDirty || layout !== baseLayout
  const missing = activeParam !== null && views.isSuccess && !active

  // A change of the URL from elsewhere (back, a link) wins over the local copy.
  useEffect(() => {
    const fromUrl = definitionFromParams(new URLSearchParams(paramText), defaultDefinition(kind).group)
    setLocalDraft((current) => {
      const same = fromUrl ? current !== null && sameDefinition(fromUrl, current) : current === null || sameDefinition(current, base)
      return same ? current : fromUrl
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paramText])

  // An address written by hand (no `d=1`) is rewritten into the form the page itself writes.
  useEffect(() => {
    if (!views.isSuccess && activeParam !== null) return
    const current = new URLSearchParams(paramText)
    if (current.get('d') === '1') return
    const fromUrl = definitionFromParams(current, defaultDefinition(kind).group)
    if (!fromUrl) return
    setParams((existing) => paramsWithDraft(existing, fromUrl, base), { replace: true })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paramText, views.isSuccess])

  // The last saved View comes back when the place is opened without one in the address.
  const restored = useRef(false)
  useEffect(() => {
    if (!views.isSuccess || restored.current) return
    restored.current = true
    if (params.get('v') !== null || params.get('layout') !== null || definitionFromParams(params) !== null) return
    const last = readLastView(key)
    if (last && views.data.some((view) => view.id === last)) {
      setParams((current) => {
        const next = new URLSearchParams(current)
        next.set('v', last)
        return next
      }, { replace: true })
    }
  }, [views.isSuccess, views.data, key, params, setParams])

  // A View deleted elsewhere falls back to all work instead of showing nothing.
  useEffect(() => {
    if (!missing) return
    onMissing()
    writeLastView(key, null)
    setParams((current) => {
      const next = new URLSearchParams(current)
      next.delete('v')
      return next
    }, { replace: true })
  }, [missing, key, setParams, onMissing])

  const setDefinition = useCallback(
    (next: ViewDefinition) => {
      setLocalDraft(sameDefinition(next, base) ? null : next)
      setParams((current) => paramsWithDraft(current, next, base), { replace: true })
    },
    [base, setParams],
  )

  const setLayout = useCallback(
    (next: ViewLayout) => {
      if (!layoutsFor(kind).includes(next)) return
      // Switching to a layout that cannot show the grouping moves to one it can.
      const fitted = fitLayout(draft ?? base, kind, next)
      setLocalDraft(sameDefinition(fitted, base) ? null : fitted)
      setParams((current) => paramsWithLayout(paramsWithDraft(current, fitted, base), next, baseLayout), { replace: true })
    },
    [base, baseLayout, draft, kind, setParams],
  )

  const reset = useCallback(() => {
    setLocalDraft(null)
    setParams((current) => paramsWithLayout(paramsWithDraft(current, base, base), baseLayout, baseLayout), { replace: true })
  }, [base, baseLayout, setParams])

  const select = useCallback(
    (id: string | null) => {
      writeLastView(key, id)
      setLocalDraft(null)
      setParams((current) => {
        const next = paramsWithLayout(paramsWithDraft(current, base, base), baseLayout, baseLayout)
        if (id) next.set('v', id)
        else next.delete('v')
        return next
      })
    },
    [base, baseLayout, key, setParams],
  )

  const reload = useCallback(() => queryClient.invalidateQueries({ queryKey: viewsQueryKey(target) }), [queryClient, target])

  async function saveChanges(): Promise<ViewSaveError> {
    if (!active) return 'generic'
    try {
      const saved = await api.updateProjectView(active.id, { expectedVersion: active.version, layout, definition })
      await reload()
      setLocalDraft(null)
      setParams((current) => paramsWithLayout(paramsWithDraft(current, saved.definition, saved.definition), saved.layout, saved.layout), { replace: true })
      return null
    } catch (cause) {
      await reload()
      return viewError(cause)
    }
  }

  async function saveAs(name: string, scope: 'PERSONAL' | 'PROJECT'): Promise<ViewSaveError> {
    try {
      const created =
        target.kind === 'PROJECT'
          ? await api.createProjectView(target.projectId, { name, scope, layout, definition }, newIdempotencyKey())
          : await api.createWideView(
              target.workspaceId,
              { targetKind: target.kind, teamId: target.kind === 'TEAM' ? target.teamId : undefined, name, layout, definition },
              newIdempotencyKey(),
            )
      await reload()
      writeLastView(key, created.id)
      setLocalDraft(null)
      setParams((current) => {
        const next = paramsWithLayout(paramsWithDraft(current, created.definition, created.definition), created.layout, created.layout)
        next.set('v', created.id)
        return next
      })
      return null
    } catch (cause) {
      return viewError(cause)
    }
  }

  async function rename(name: string): Promise<ViewSaveError> {
    if (!active) return 'generic'
    try {
      await api.updateProjectView(active.id, { expectedVersion: active.version, name })
      await reload()
      return null
    } catch (cause) {
      await reload()
      return viewError(cause)
    }
  }

  async function remove(): Promise<ViewSaveError> {
    if (!active) return 'generic'
    try {
      await api.deleteProjectView(active.id, active.version)
      await reload()
      select(null)
      return null
    } catch (cause) {
      await reload()
      return viewError(cause)
    }
  }

  return { views: viewList, loaded: views.isSuccess, active, layout, definition, dirty, setDefinition, setLayout, reset, select, saveChanges, saveAs, rename, remove }
}
