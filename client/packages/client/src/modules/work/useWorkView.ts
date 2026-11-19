import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { ProjectView, TableColumn, ViewDefinition, ViewLayout, ViewTargetKind } from '@penfii/contracts'
import { api } from '../../api'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { errorCode } from './ticketModel'
import type { ViewSaveError } from './ViewBar'
import { defaultDefinition, definitionFromParams, fitLayout, layoutFromParams, layoutOf, layoutsFor, paramsWithDraft, sameDefinition, startsCompact } from './viewModel'
import { hasPrefs, prefsKey, readPrefs, writePrefs, type DisplayPrefs } from './displayPrefs'

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
  const base: ViewDefinition = active?.definition ?? defaultDefinition(kind)
  // The person's own display of this View (layout, density, columns) is kept on this device and is never part of the View.
  const session = useSession()
  const userId = session.data?.id ?? 'anonymous'
  const storeKey = prefsKey(userId, key, active?.id ?? null)
  const [prefsTick, setPrefsTick] = useState(0)
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const prefs: DisplayPrefs = useMemo(() => readPrefs(storeKey), [storeKey, prefsTick])
  const savePrefs = useCallback(
    (next: DisplayPrefs) => {
      writePrefs(storeKey, next)
      setPrefsTick((tick) => tick + 1)
    },
    [storeKey],
  )
  // Priority: the person's own choice, then what the saved View records, then the product's default.
  const baseLayout: ViewLayout = layoutOf(active?.layout, kind)
  const paramText = params.toString()
  const [draft, setLocalDraft] = useState<ViewDefinition | null>(() => definitionFromParams(params, defaultDefinition(kind).group))
  const urlLayout = layoutFromParams(params, kind)
  const layout: ViewLayout = urlLayout ?? (prefs.layout && layoutsFor(kind).includes(prefs.layout) ? prefs.layout : baseLayout)
  const askedLayout = params.get('layout')?.toUpperCase()
  const comesFromList = askedLayout === 'LIST' || (askedLayout === undefined && startsCompact(active ? active.layout : null, kind))
  const density: 'basic' | 'compact' = prefs.density ?? (comesFromList || base.display.compact ? 'compact' : 'basic')
  const columns: TableColumn[] = prefs.columns ?? base.display.columns
  const fitted = fitLayout(draft ?? base, kind, layout)
  const definition: ViewDefinition = { ...fitted, display: { subtasks: fitted.display.subtasks, compact: density === 'compact', columns } }
  // Only what the View shows can be an unsaved change. Layout, density and columns are the person's own.
  const dirty = draft !== null && !sameDefinition(draft, base)
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
      // A change of density or columns is the person's own display, not a change to the View.
      const display = next.display
      if (display.compact !== (density === 'compact') || display.columns.join() !== columns.join()) {
        savePrefs({ ...prefs, density: display.compact ? 'compact' : 'basic', columns: display.columns })
      }
      setLocalDraft(sameDefinition(next, base) ? null : next)
      setParams((current) => paramsWithDraft(current, next, base), { replace: true })
    },
    [base, columns, density, prefs, savePrefs, setParams],
  )

  const setLayout = useCallback(
    (next: ViewLayout) => {
      if (!layoutsFor(kind).includes(next)) return
      // Switching to a layout that cannot show the grouping moves to one it can.
      const adjusted = fitLayout(draft ?? base, kind, next)
      setLocalDraft(sameDefinition(adjusted, base) ? null : adjusted)
      savePrefs({ ...prefs, layout: next })
      setParams((current) => {
        const params = paramsWithDraft(current, adjusted, base)
        params.delete('layout')
        return params
      }, { replace: true })
    },
    [base, draft, kind, prefs, savePrefs, setParams],
  )

  /** Drops the unsaved filter, grouping and sorting, and the person's own display for this View. */
  const reset = useCallback(() => {
    setLocalDraft(null)
    savePrefs({})
    setParams((current) => {
      const params = paramsWithDraft(current, base, base)
      params.delete('layout')
      return params
    }, { replace: true })
  }, [base, savePrefs, setParams])

  /** Only the display goes back (to the View's own saved display, or the product's default); the filter stays. */
  const resetDisplay = useCallback(() => savePrefs({}), [savePrefs])

  const select = useCallback(
    (id: string | null) => {
      writeLastView(key, id)
      setLocalDraft(null)
      setParams((current) => {
        const next = paramsWithDraft(current, base, base)
        next.delete('layout')
        if (id) next.set('v', id)
        else next.delete('v')
        return next
      })
    },
    [base, key, setParams],
  )

  const reload = useCallback(() => queryClient.invalidateQueries({ queryKey: viewsQueryKey(target) }), [queryClient, target])

  async function saveChanges(): Promise<ViewSaveError> {
    if (!active) return 'generic'
    try {
      // What is saved is what the View shows. The person's own layout, density and columns are not written into it:
      // the View keeps the display it already has (including a List layout it was saved with).
      const keep = active.definition.display
      const shared: ViewDefinition = { ...fitLayout(draft ?? base, kind, layoutOf(active.layout, kind)), display: { subtasks: definition.display.subtasks, compact: keep.compact, columns: keep.columns } }
      const saved = await api.updateProjectView(active.id, { expectedVersion: active.version, layout: active.layout ?? baseLayout, definition: shared })
      await reload()
      setLocalDraft(null)
      setParams((current) => {
        const params = paramsWithDraft(current, saved.definition, saved.definition)
        params.delete('layout')
        return params
      }, { replace: true })
      return null
    } catch (cause) {
      await reload()
      return viewError(cause)
    }
  }

  async function saveAs(name: string, scope: 'PERSONAL' | 'PROJECT'): Promise<ViewSaveError> {
    // A new View starts without a saved density or columns; the person's own display carries over on this device only.
    const newViewDefinition: ViewDefinition = { ...definition, display: { subtasks: definition.display.subtasks, compact: false, columns: [] } }
    try {
      const created =
        target.kind === 'PROJECT'
          ? await api.createProjectView(target.projectId, { name, scope, layout, definition: newViewDefinition }, newIdempotencyKey())
          : await api.createWideView(
              target.workspaceId,
              { targetKind: target.kind, teamId: target.kind === 'TEAM' ? target.teamId : undefined, name, layout, definition: newViewDefinition },
              newIdempotencyKey(),
            )
      await reload()
      writePrefs(prefsKey(userId, key, created.id), prefs)
      writeLastView(key, created.id)
      setLocalDraft(null)
      setParams((current) => {
        const next = paramsWithDraft(current, created.definition, created.definition)
        next.delete('layout')
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

  return { views: viewList, loaded: views.isSuccess, active, layout, density, definition, dirty, hasOwnDisplay: hasPrefs(prefs), resetDisplay, setDefinition, setLayout, reset, select, saveChanges, saveAs, rename, remove }
}
