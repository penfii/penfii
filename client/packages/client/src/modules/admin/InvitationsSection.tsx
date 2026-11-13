import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { InvitationCreated, InvitationSummary } from '@penfii/contracts'
import { api } from '../../api'
import { Badge, Button, useConfirm } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { OpenCreate } from '../../shell/WorkspaceShell'
import { InviteResult } from './InviteResult'
import { useAdminAction } from './useAdminAction'
import '../spaces/teamLifecycle.css'

export function InvitationsSection({ workspaceId, viewerRole }: { workspaceId: string; viewerRole: 'OWNER' | 'ADMIN' }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const invitations = useQuery(queries.adminInvitations(workspaceId))
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  // The link of the invitation that was just sent again: shown once, it is not stored anywhere.
  const [reissued, setReissued] = useState<InvitationCreated | null>(null)

  async function sendAgain(id: string, email: string) {
    if (!(await confirm({ title: t('admin.invitations.reissue'), body: t('admin.invitations.reissueConfirm', { email }), confirmLabel: t('admin.invitations.reissue') }))) return
    setReissued(null)
    const result: { created?: InvitationCreated } = {}
    void perform(async () => { result.created = await api.reissueInvitation(workspaceId, id) }, async () => {
      setReissued(result.created ?? null)
      await queryClient.invalidateQueries({ queryKey: queryKeys.adminInvitations(workspaceId) })
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
    })
  }

  async function revoke(id: string, email: string) {
    if (!(await confirm({ title: t('admin.invitations.revoke'), body: t('admin.invitations.revokeConfirm', { email }), confirmLabel: t('admin.invitations.revoke') }))) return
    void perform(() => api.revokeInvitation(workspaceId, id), async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.adminInvitations(workspaceId) })
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
    })
  }

  return (
    <div className="card settings-card" aria-labelledby="admin-invitations-title">
      <h3 id="admin-invitations-title">{t('admin.invitations.title')}</h3>
      <p className="field-hint">{t('admin.invitations.lead')}</p>
      <OpenCreate kind="invite" label={t('create.invite')} />
      {reissued ? <InviteResult invitation={reissued} /> : null}
      {invitations.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {invitations.isError ? <p className="form-error" role="alert">{t('admin.loadError')}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {invitations.data && invitations.data.length === 0 ? <p className="hint">{t('admin.invitations.empty')}</p> : null}
      {invitations.data && invitations.data.length > 0 ? (
        <div className="admin-table-wrap"><table className="record-table permissions-table">
          <thead>
            <tr>
              <th>{t('admin.invitations.email')}</th>
              <th>{t('admin.invitations.role')}</th>
              <th>{t('admin.invitations.team')}</th>
              <th>{t('admin.invitations.status')}</th>
              <th>{t('admin.invitations.expires')}</th>
              <th>{t('admin.invitations.mail')}</th>
              <th>{t('admin.invitations.by')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {invitations.data.map((invitation) => {
              // An admin revokes invitations for plain members only; the server enforces it as well.
              const canRevoke = invitation.status === 'PENDING' && (viewerRole === 'OWNER' || invitation.role === 'MEMBER')
              // An open invitation, expired or not, is sent again with a new link. An expired one that a newer
              // invitation took the place of is closed (it has a `revokedAt`) and cannot be.
              const open = invitation.status === 'PENDING' || (invitation.status === 'EXPIRED' && invitation.revokedAt === null)
              const canReissue = open && (viewerRole === 'OWNER' || invitation.role === 'MEMBER')
              return (
                <tr key={invitation.id}>
                  <td data-label={t('admin.invitations.email')}>{invitation.email}</td>
                  <td className="cell-nowrap" data-label={t('admin.invitations.role')}>{t(`admin.role.${invitation.role as 'OWNER' | 'ADMIN' | 'MEMBER'}`)}</td>
                  <td className="cell-invitation-team" data-label={t('admin.invitations.team')}><InvitationTeam invitation={invitation} /></td>
                  <td data-label={t('admin.invitations.status')}><Badge tone={invitation.status === 'PENDING' ? 'accent' : 'neutral'}>{t(`admin.invitations.status.${invitation.status}`)}</Badge></td>
                  <td data-label={t('admin.invitations.expires')}>{formatStamp(invitation.expiresAt, locale)}</td>
                  <td data-label={t('admin.invitations.mail')}>
                    {invitation.emailStatus ? (
                      <Badge tone={invitation.emailStatus === 'FAILED' ? 'accent' : 'neutral'}>{t(`admin.invitations.mail.${invitation.emailStatus}`)}</Badge>
                    ) : null}
                    {invitation.emailStatus === 'FAILED' && invitation.emailError ? <small className="hint invitation-team-note">{invitation.emailError}</small> : null}
                  </td>
                  <td data-label={t('admin.invitations.by')}>{invitation.createdByName ?? ''}</td>
                  <td>
                    {canReissue ? (
                      <Button variant="secondary" disabled={busy} aria-label={`${t('admin.invitations.reissue')} ${invitation.email}`} onClick={() => void sendAgain(invitation.id, invitation.email)}>
                        {t('admin.invitations.reissue')}
                      </Button>
                    ) : null}
                    {canRevoke ? (
                      <Button variant="secondary" disabled={busy} aria-label={`${t('admin.invitations.revoke')} ${invitation.email}`} onClick={() => void revoke(invitation.id, invitation.email)}>
                        {t('admin.invitations.revoke')}
                      </Button>
                    ) : null}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table></div>
      ) : null}
      {dialog}
      {confirmDialog}
    </div>
  )
}

/** The Team step of an invitation: which Team, and what accepting it now would do when that Team is archived or gone. */
function InvitationTeam({ invitation }: { invitation: InvitationSummary }) {
  const { t } = useI18n()
  if (!invitation.teamStatus) return <span className="hint">{t('admin.invitations.noTeam')}</span>
  const name = invitation.teamName ?? ''
  if (invitation.teamStatus === 'ACTIVE') return <span>{name}</span>
  const pending = invitation.status === 'PENDING'
  const archived = invitation.teamStatus === 'ARCHIVED'
  return (
    <span>
      <span>{t(archived ? 'admin.invitations.teamArchived' : 'admin.invitations.teamGone', { name })}</span>
      {pending ? <small className="hint invitation-team-note">{t(archived ? 'admin.invitations.teamArchivedNote' : 'admin.invitations.teamGoneNote', { name })}</small> : null}
    </span>
  )
}
