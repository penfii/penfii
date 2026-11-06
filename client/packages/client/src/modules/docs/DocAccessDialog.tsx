import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { DocAccessRole, DocPage, DocParticipant, DocVisibility } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog, Select } from '../../design/ui'
import { ShareScopePicker, type ShareScope } from './ShareScopePicker'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { useScopeLabel } from './useScopeLabel'
import { CloseIcon } from '../../design/icons'

type Row = { userId: string; name: string; role: DocAccessRole }

/** What the dialog needs from a Doc page: its participants, visibility and refresh. */
export type AccessTarget = {
  title: string
  visibility: DocVisibility
  scopeProjectId: string | null
  scopeTeamId: string | null
  /** Where it was made: offered first when "a Project" is picked, so the usual choice is one click. */
  createdInProjectId?: string | null
  canManage: boolean
  queryKey: readonly unknown[]
  listParticipants: () => Promise<DocParticipant[]>
  setParticipants: (participants: Array<{ userId: string; role: DocAccessRole }>) => Promise<unknown>
  setVisibility: (visibility: DocVisibility, scope: { scopeProjectId: string | null; scopeTeamId: string | null }) => Promise<unknown>
  refresh: () => Promise<unknown>
}

/**
 * Who can read and edit a page . WORKSPACE: every member. PROJECT: the Project's people (only
 * those also in the chosen Team, when one is chosen). RESTRICTED: the people invited by name and nobody else,
 * not even Workspace admins. People outside the scope can be invited by name in every case. A change takes
 * effect at once, including for people who have it open.
 */
export function AccessDialog({
  open,
  target,
  workspaceId,
  onClose,
}: {
  open: boolean
  target: AccessTarget
  workspaceId: string
  onClose: () => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const participants = useQuery({ queryKey: target.queryKey, queryFn: target.listParticipants, enabled: open })
  const initial: ShareScope = { visibility: target.visibility, projectId: target.scopeProjectId, teamId: target.scopeTeamId }
  const [scope, setScope] = useState<ShareScope>(initial)
  const [rows, setRows] = useState<Row[]>([])
  const [text, setText] = useState('')
  const [found, setFound] = useState<Array<{ id: string; name: string }>>([])
  const [error, setError] = useState<string | null>(null)
  const saving = useMutation({
    mutationFn: async () => {
      const send = rows.map((r) => ({ userId: r.userId, role: r.role }))
      // Widening first, narrowing last: the person never locks themselves out half way, and a narrower scope
      // always has its manager before it is applied.
      const rank = (v: DocVisibility) => (v === 'WORKSPACE' ? 2 : v === 'PROJECT' ? 1 : 0)
      const apply = () => target.setVisibility(scope.visibility, { scopeProjectId: scope.projectId, scopeTeamId: scope.teamId })
      const changed = scope.visibility !== target.visibility || scope.projectId !== target.scopeProjectId || scope.teamId !== target.scopeTeamId
      const widening = rank(scope.visibility) > rank(target.visibility)
      if (changed && widening) await apply()
      await target.setParticipants(send)
      if (changed && !widening) await apply()
    },
    onMutate: () => setError(null),
    onSuccess: async () => {
      await target.refresh()
      await queryClient.invalidateQueries({ queryKey: target.queryKey })
      onClose()
    },
    onError: (failure) => {
      const code = isApiError(failure) ? failure.code : ''
      setError(code === 'doc_manager_required' ? t('docs.access.managerRequired') : t('docs.access.error'))
    },
  })
  const busy = saving.isPending
  const manage = target.canManage
  const nowLabel = useScopeLabel(workspaceId, initial)
  const nextLabel = useScopeLabel(workspaceId, scope)
  const scopeChanged = scope.visibility !== initial.visibility || scope.projectId !== initial.projectId || scope.teamId !== initial.teamId

  useEffect(() => {
    if (open) {
      setScope({ visibility: target.visibility, projectId: target.scopeProjectId, teamId: target.scopeTeamId })
      setError(null)
    }
  }, [open, target.visibility, target.scopeProjectId, target.scopeTeamId])
  useEffect(() => {
    if (participants.data) setRows(participants.data.map((p) => ({ userId: p.userId, name: p.name, role: p.role })))
  }, [participants.data])

  async function search(query: string) {
    setText(query)
    if (!query.trim()) {
      setFound([])
      return
    }
    const result = await api.searchDirectory(workspaceId, { q: query, types: 'person', limit: 6 })
    setFound(
      result.results
        .flatMap((hit) => (hit.type === 'person' && hit.person ? [{ id: hit.person.id, name: hit.person.displayName }] : []))
        .filter((p) => !rows.some((r) => r.userId === p.id)),
    )
  }

  return (
    <Dialog open={open} title={target.title} onClose={onClose} presentation="full">
      <ShareScopePicker workspaceId={workspaceId} name="doc-visibility" value={scope} onChange={setScope} disabled={!manage || busy} suggestedProjectId={target.createdInProjectId ?? null} />
      {scopeChanged ? <p className="share-scope-change" role="status">{t('docs.access.change', { from: nowLabel, to: nextLabel })}</p> : null}
      <fieldset disabled={!manage || busy} className="doc-access">
        <h3>{t('docs.access.people')}</h3>
        <p className="hint">{t('docs.access.peopleHint')}</p>
        {participants.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
        <ul className="doc-access-list">
          {rows.map((row) => (
            <li key={row.userId}>
              <span>{row.name}</span>
              <Select
                aria-label={`${row.name}: ${t('docs.access.role')}`}
                value={row.role}
                onChange={(event) => setRows((current) => current.map((r) => (r.userId === row.userId ? { ...r, role: event.target.value as DocAccessRole } : r)))}
              >
                <option value="MANAGER">{t('docs.access.manager')}</option>
                <option value="EDITOR">{t('docs.access.editor')}</option>
                <option value="VIEWER">{t('docs.access.viewer')}</option>
              </Select>
              <button type="button" aria-label={`${t('docs.delete')}: ${row.name}`} onClick={() => setRows((current) => current.filter((r) => r.userId !== row.userId))}><CloseIcon /></button>
            </li>
          ))}
        </ul>
        {manage ? (
          <>
            <input type="search" aria-label={t('docs.access.add')} placeholder={t('docs.access.add')} value={text} onChange={(event) => void search(event.target.value)} />
            <ul className="doc-access-list">
              {found.map((person) => (
                <li key={person.id}>
                  <button
                    type="button"
                    onClick={() => {
                      setRows((current) => [...current, { userId: person.id, name: person.name, role: 'EDITOR' }])
                      setFound([])
                      setText('')
                    }}
                  >
                    + {person.name}
                  </button>
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p className="hint">{t('docs.access.readOnly')}</p>
        )}
      </fieldset>
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      <div className="dialog-actions">
        <Button variant="ghost" onClick={onClose}>{t('docs.cancel')}</Button>
        {manage ? <Button onClick={() => saving.mutate()} disabled={busy}>{t('docs.access.save')}</Button> : null}
      </div>
    </Dialog>
  )
}

export function DocAccessDialog({ open, page, workspaceId, onClose }: { open: boolean; page: DocPage; workspaceId: string; onClose: () => void }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  return (
    <AccessDialog
      open={open}
      workspaceId={workspaceId}
      onClose={onClose}
      target={{
        title: t('docs.access.title'),
        visibility: page.visibility,
        scopeProjectId: page.scopeProjectId,
        scopeTeamId: page.scopeTeamId,
        createdInProjectId: page.projectId,
        canManage: page.rights.manage,
        queryKey: queryKeys.docParticipants(page.id),
        listParticipants: () => api.getDocParticipants(page.id),
        setParticipants: (list) => api.setDocParticipants(page.id, list),
        setVisibility: (v, scope) => api.setDocVisibility(page.id, v, scope),
        refresh: () =>
          Promise.all([
            queryClient.invalidateQueries({ queryKey: queryKeys.doc(page.id) }),
            queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) }),
          ]),
      }}
    />
  )
}
