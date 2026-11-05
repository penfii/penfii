import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { WorkspaceMember } from '@penfii/contracts'
import { api } from '../../api'
import { Badge, Button, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'

const ROLES = ['OWNER', 'ADMIN', 'MEMBER'] as const

export function MembersSection({ workspaceId, viewerRole }: { workspaceId: string; viewerRole: 'OWNER' | 'ADMIN' }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const me = useSession()
  const members = useQuery(queries.adminMembers(workspaceId))
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const isOwner = viewerRole === 'OWNER'
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminMembers(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
  }

  async function remove(member: WorkspaceMember) {
    if (!(await confirm({ title: t('admin.members.remove'), body: t('admin.members.removeConfirm', { name: member.displayName }), confirmLabel: t('admin.members.remove') }))) return
    void perform(() => api.removeMember(workspaceId, member.userId), refresh)
  }

  async function setDisabled(member: WorkspaceMember, disabled: boolean) {
    if (disabled && !(await confirm({ title: t('admin.members.disable'), body: t('admin.members.disableConfirm', { name: member.displayName }), confirmLabel: t('admin.members.disable') }))) return
    void perform(() => api.setAccountDisabled(workspaceId, member.userId, disabled), refresh)
  }

  async function changeRole(member: WorkspaceMember, role: (typeof ROLES)[number]) {
    if (role === member.role) return
    // Handing over or taking back ownership is the one role change that deserves a second look.
    if ((role === 'OWNER' || member.role === 'OWNER') && !(await confirm({ title: t('admin.members.role'), body: t('admin.members.ownerConfirm', { name: member.displayName }), confirmLabel: t('confirm'), tone: 'primary' }))) return
    void perform(() => api.changeMemberRole(workspaceId, member.userId, role), refresh)
  }

  return (
    <div className="card settings-card" aria-labelledby="admin-members-title">
      <h3 id="admin-members-title">{t('admin.members.title')}</h3>
      <p className="field-hint">{t('admin.members.lead')}</p>
      {members.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {members.isError ? <p className="form-error" role="alert">{t('admin.loadError')}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {members.data ? (
        <div className="admin-table-wrap"><table className="record-table permissions-table">
          <thead>
            <tr>
              <th>{t('admin.members.name')}</th>
              <th>{t('admin.members.role')}</th>
              <th className="admin-col-teams">{t('admin.members.teams')}</th>
              <th>{t('admin.members.state')}</th>
              <th>{t('admin.members.actions')}</th>
            </tr>
          </thead>
          <tbody>
            {members.data.map((member) => {
              const self = member.userId === me.data?.id
              const active = member.state === 'ACTIVE'
              // An admin manages plain members only; nobody changes themself here.
              const canRemove = active && !self && (isOwner || member.role === 'MEMBER')
              return (
                <tr key={member.userId}>
                  <td data-label={t('admin.members.name')}>
                    {member.displayName} {self ? <small>{t('admin.members.you')}</small> : null}
                    <br />
                    <small className="hint">{member.email}</small>
                  </td>
                  <td data-label={t('admin.members.role')}>
                    {isOwner && active && !self ? (
                      <Select
                        aria-label={t('admin.members.roleOf', { name: `${member.displayName} (${member.email})` })}
                        value={member.role}
                        disabled={busy}
                        onChange={(event) => void changeRole(member, event.target.value as (typeof ROLES)[number])}
                      >
                        {ROLES.map((role) => <option key={role} value={role}>{t(`admin.role.${role}`)}</option>)}
                      </Select>
                    ) : (
                      t(`admin.role.${member.role}`)
                    )}
                  </td>
                  <td className="admin-col-teams" data-label={t('admin.members.teams')}>{member.teams.map((team) => team.name).join(', ')}</td>
                  <td data-label={t('admin.members.state')}>
                    <Badge tone={active ? 'accent' : 'neutral'}>{active ? t('admin.members.active') : t('admin.members.removed')}</Badge>{' '}
                    {member.accountDisabled ? <Badge tone="preview">{t('admin.members.disabled')}</Badge> : null}
                  </td>
                  <td data-label={t('admin.members.actions')}>
                    <div className="row-actions">
                      {canRemove ? (
                        <Button variant="secondary" disabled={busy} aria-label={`${t('admin.members.remove')} ${member.displayName} (${member.email})`} onClick={() => void remove(member)}>
                          {t('admin.members.remove')}
                        </Button>
                      ) : null}
                      {isOwner && !self ? (
                        <Button
                          variant="ghost"
                          disabled={busy}
                          aria-label={`${member.accountDisabled ? t('admin.members.enable') : t('admin.members.disable')} ${member.displayName} (${member.email})`}
                          onClick={() => void setDisabled(member, !member.accountDisabled)}
                        >
                          {member.accountDisabled ? t('admin.members.enable') : t('admin.members.disable')}
                        </Button>
                      ) : null}
                    </div>
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
