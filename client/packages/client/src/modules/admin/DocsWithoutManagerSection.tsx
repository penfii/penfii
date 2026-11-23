import { useState } from 'react'
import { QueryError } from '../../ui/QueryError'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { DocWithoutManager } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'
import type { RecoveryStatus } from './recoveryStatus'

/**
 * Docs shared with a Project or with invited people that nobody can manage, and the one thing a Workspace
 * owner or admin may do about it: name a manager. Only title, creator and date are listed, never content,
 * and the confirmation says that the person named gets to read and edit the Doc. Absent while every such
 * Doc has a manager.
 */
export function DocsWithoutManagerSection({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const docs = useQuery(queries.adminDocsWithoutManager(workspaceId))
  const members = useQuery(queries.adminMembers(workspaceId))
  const [status, setStatus] = useState<RecoveryStatus | null>(null)
  if (docs.isError) return <QueryError query={docs} compact />
  if (!docs.data?.length && !status) return null
  const candidates = (members.data ?? []).filter((member) => member.state === 'ACTIVE' && !member.accountDisabled)
  return (
    <div className="card settings-card" aria-labelledby="admin-no-doc-manager-title">
      <h3 id="admin-no-doc-manager-title">{t('admin.noDocManager.title')}</h3>
      {docs.data?.length ? <p className="field-hint">{t('admin.noDocManager.lead')}</p> : null}
      {status ? (
        <p className={status.tone === 'error' ? 'form-error' : 'hint'} role={status.tone === 'error' ? 'alert' : 'status'}>
          {t(status.key)}
        </p>
      ) : null}
      <ul className="plain-list">
        {(docs.data ?? []).map((doc) => (
          <DocRow key={doc.id} workspaceId={workspaceId} doc={doc} candidates={candidates} report={setStatus} />
        ))}
      </ul>
    </div>
  )
}

function DocRow({
  workspaceId,
  doc,
  candidates,
  report,
}: {
  workspaceId: string
  doc: DocWithoutManager
  candidates: { userId: string; displayName: string; email: string }[]
  report: (status: RecoveryStatus | null) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const me = useSession()
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [chosen, setChosen] = useState('')

  async function assign() {
    const person = candidates.find((candidate) => candidate.userId === chosen)
    if (!person) return
    const self = person.userId === me.data?.id
    const body = self
      ? t('admin.noDocManager.confirmSelf', { doc: doc.title })
      : t('admin.noDocManager.confirm', { person: person.displayName, doc: doc.title })
    if (!(await confirm({ title: t('admin.noDocManager.assign'), body, confirmLabel: t('admin.noDocManager.assign'), tone: 'primary' }))) return
    report(null)
    let result: Awaited<ReturnType<typeof api.recoverDocManager>> | null = null
    const ok = await perform(
      async () => {
        try {
          result = await api.recoverDocManager(doc.id, person.userId)
        } catch (caught) {
          // Somebody named a manager first: the list is out of date, not the request wrong.
          if (isApiError(caught) && (caught.code === 'doc_has_manager' || caught.code === 'doc_not_restricted')) {
            report({ tone: 'error', key: 'admin.noDocManager.hasManager' })
            void queryClient.invalidateQueries({ queryKey: queryKeys.adminDocsWithoutManager(workspaceId) })
          }
          throw caught
        }
      },
      async () => {
        await queryClient.invalidateQueries({ queryKey: queryKeys.adminDocsWithoutManager(workspaceId) })
        await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
      },
    )
    if (ok) report({ tone: 'ok', key: (result as { newAccess: boolean } | null)?.newAccess ? 'admin.noDocManager.doneNewAccess' : 'admin.noDocManager.done' })
  }

  return (
    <li className="admin-no-admin-row">
      <strong>{doc.title}</strong>{' '}
      <small className="hint">{t('admin.noDocManager.meta', { creator: doc.createdByName, date: doc.createdAt.slice(0, 10) })}</small>
      <div className="row-actions">
        <Select aria-label={`${t('admin.noDocManager.choose')}: ${doc.title}`} value={chosen} disabled={busy} onChange={(event) => setChosen(event.target.value)}>
          <option value="">{t('admin.noDocManager.pick')}</option>
          {candidates.map((person) => (
            <option key={person.userId} value={person.userId}>{`${person.displayName} (${person.email})`}</option>
          ))}
        </Select>
        <Button disabled={busy || !chosen} aria-label={`${t('admin.noDocManager.assign')}: ${doc.title}`} onClick={() => void assign()}>
          {t('admin.noDocManager.assign')}
        </Button>
      </div>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {dialog}
      {confirmDialog}
    </li>
  )
}
