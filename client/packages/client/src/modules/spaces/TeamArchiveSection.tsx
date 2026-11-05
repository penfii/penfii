import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Team } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { useAdminAction } from '../admin/useAdminAction'
import { refreshTeamData } from './teamState'

/** Archive for a Workspace owner/admin; a Team manager sees the action disabled with the reason. */
export function TeamArchiveSection({ workspaceId, team, canArchive, channelCount, memberCount }: {
  workspaceId: string
  team: Team
  canArchive: boolean
  channelCount: number
  memberCount: number
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const { perform, busy, error, dialog } = useAdminAction()
  const projects = useQuery({ ...queries.teamProjects(workspaceId, team.id), enabled: open })
  const invitations = useQuery({ ...queries.adminInvitations(workspaceId), enabled: open && canArchive })
  const withRole = (projects.data ?? []).filter((project) => project.teamRole !== null)
  const linkedOnly = (projects.data ?? []).filter((project) => project.teamRole === null)
  const pending = (invitations.data ?? []).filter((invitation) => invitation.status === 'PENDING' && invitation.teamId === team.id).length
  return (
    <section className="card settings-card team-archive" aria-labelledby="team-archive-title">
      <h2 id="team-archive-title">{t('teams.archive.title')}</h2>
      <p className="hint">{canArchive ? t('teams.archive.who') : t('teams.archive.disabled')}</p>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      <Button type="button" variant="danger" disabled={!canArchive || busy} onClick={() => setOpen(true)}>{t('teams.archive.button')}</Button>
      <Dialog open={open} title={t('teams.archive.confirmTitle', { name: team.name })} onClose={() => setOpen(false)}>
        <p>{t('teams.archive.confirmLead')}</p>
        <ul className="plain-list">
          <li>{t('teams.archive.members', { count: String(memberCount) })}</li>
          <li>{t('teams.archive.channels', { count: String(channelCount) })}</li>
          <li>
            {t('teams.archive.projects', { count: String(withRole.length + linkedOnly.length) })}
            {projects.data ? ` (${[...withRole.map((project) => `${project.name} ${project.teamRole ? t(`permissions.role.${project.teamRole}`) : ''}`), ...linkedOnly.map((project) => `${project.name} ${t('teams.projects.linkedOnly')}`)].join(', ')})` : ''}
          </li>
          <li>{t('teams.archive.invites', { count: String(pending) })}</li>
        </ul>
        <div className="callout">
          <strong>{t('teams.archive.projectsWarnTitle')}</strong>
          <p className="hint">{t('teams.archive.projectsWarn')}</p>
        </div>
        <div className="dialog-actions">
          <Button type="button" variant="secondary" onClick={() => setOpen(false)}>{t('message.cancel')}</Button>
          <Button
            type="button"
            variant="danger"
            disabled={busy}
            onClick={() => void perform(() => api.archiveTeam(workspaceId, team.id), async () => { setOpen(false); await refreshTeamData(queryClient, workspaceId) })}
          >
            {t('teams.archive.confirm')}
          </Button>
        </div>
      </Dialog>
      {dialog}
    </section>
  )
}

/** The restore action of an archived Team's banner. */
export function RestoreTeamButton({ workspaceId, team }: { workspaceId: string; team: Team }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { perform, busy, error, dialog } = useAdminAction()
  return (
    <>
      <Button type="button" disabled={busy} onClick={() => void perform(() => api.restoreTeam(workspaceId, team.id), () => refreshTeamData(queryClient, workspaceId))}>
        {t('teams.restore')}
      </Button>
      {error ? <span className="form-error" role="alert">{error}</span> : null}
      {dialog}
    </>
  )
}
