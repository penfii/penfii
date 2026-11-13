import { useState } from 'react'
import { QueryError } from '../../ui/QueryError'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { ProjectWithoutAdmin } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'
import type { RecoveryStatus } from './recoveryStatus'

/**
 * Projects nobody can manage (no active admin, directly or through a Team or Group) and the one thing
 * a Workspace owner or admin may do about it: name an admin. Nothing is promoted on its own, and the
 * confirmation says that the person named gets the Project's content. The section is absent while
 * every Project has an admin.
 */
export function ProjectsWithoutAdminSection({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const projects = useQuery(queries.adminProjectsWithoutAdmin(workspaceId))
  const members = useQuery(queries.adminMembers(workspaceId))
  // The outcome is kept here, not in the row: naming the admin makes the row disappear.
  const [status, setStatus] = useState<RecoveryStatus | null>(null)
  // A failed read must not look like "nothing to recover".
  if (projects.isError) return <QueryError query={projects} compact />
  if (!projects.data?.length && !status) return null
  // Only people who can act: an active member whose account is not disabled.
  const candidates = (members.data ?? []).filter((member) => member.state === 'ACTIVE' && !member.accountDisabled)
  return (
    <div className="card settings-card" aria-labelledby="admin-no-admin-title">
      <h3 id="admin-no-admin-title">{t('admin.noAdmin.title')}</h3>
      {projects.data?.length ? <p className="field-hint">{t('admin.noAdmin.lead')}</p> : null}
      {status ? (
        <p className={status.tone === 'error' ? 'form-error' : 'hint'} role={status.tone === 'error' ? 'alert' : 'status'}>
          {t(status.key)}
        </p>
      ) : null}
      <ul className="plain-list">
        {(projects.data ?? []).map((project) => (
          <ProjectRow key={project.id} workspaceId={workspaceId} project={project} candidates={candidates} report={setStatus} />
        ))}
      </ul>
    </div>
  )
}

function ProjectRow({
  workspaceId,
  project,
  candidates,
  report,
}: {
  workspaceId: string
  project: ProjectWithoutAdmin
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
      ? t('admin.noAdmin.confirmSelf', { project: project.name })
      : t('admin.noAdmin.confirm', { person: person.displayName, project: project.name })
    if (!(await confirm({ title: t('admin.noAdmin.assign'), body, confirmLabel: t('admin.noAdmin.assign'), tone: 'primary' }))) return
    report(null)
    let result: Awaited<ReturnType<typeof api.recoverProjectAdmin>> | null = null
    const ok = await perform(
      async () => {
        try {
          result = await api.recoverProjectAdmin(project.id, person.userId)
        } catch (caught) {
          // Somebody named an admin first: the list is out of date, not the request wrong. Say so,
          // and read the list again so the row that no longer applies goes away.
          if (isApiError(caught) && caught.code === 'project_has_admin') {
            report({ tone: 'error', key: 'admin.noAdmin.hasAdmin' })
            void queryClient.invalidateQueries({ queryKey: queryKeys.adminProjectsWithoutAdmin(workspaceId) })
          }
          throw caught
        }
      },
      async () => {
        await queryClient.invalidateQueries({ queryKey: queryKeys.adminProjectsWithoutAdmin(workspaceId) })
        await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
      },
    )
    if (ok) report({ tone: 'ok', key: (result as { newAccess: boolean } | null)?.newAccess ? 'admin.noAdmin.doneNewAccess' : 'admin.noAdmin.done' })
  }

  const former = project.formerAdmins.map((admin) => admin.displayName).join(', ')
  return (
    <li className="admin-no-admin-row">
      <strong>{project.name}</strong> <small className="hint">{project.ticketPrefix}</small>
      <p className="hint">{former ? t('admin.noAdmin.former', { names: former }) : t('admin.noAdmin.formerNone')}</p>
      <div className="row-actions">
        <Select aria-label={`${t('admin.noAdmin.choose')}: ${project.name}`} value={chosen} disabled={busy} onChange={(event) => setChosen(event.target.value)}>
          <option value="">{t('admin.noAdmin.pick')}</option>
          {candidates.map((person) => (
            <option key={person.userId} value={person.userId}>{`${person.displayName} (${person.email})`}</option>
          ))}
        </Select>
        <Button disabled={busy || !chosen} aria-label={`${t('admin.noAdmin.assign')}: ${project.name}`} onClick={() => void assign()}>
          {t('admin.noAdmin.assign')}
        </Button>
      </div>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {dialog}
      {confirmDialog}
    </li>
  )
}
