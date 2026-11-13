import { useEffect, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { WorkspaceMember } from '@penfii/contracts'
import { api } from '../../api'
import { ActionMenu, type ActionItem } from '../../design/ActionMenu'
import { notify } from '../../design/notices'
import { Badge, Button, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'

const ROLES = ['OWNER', 'ADMIN', 'MEMBER'] as const
type Role = (typeof ROLES)[number]
/** Members per page. The list is read whole and paged here; the server is not paged. */
export const MEMBERS_PER_PAGE = 25
const SHOWN_TEAMS = 2

export function MembersSection({ workspaceId, viewerRole }: { workspaceId: string; viewerRole: 'OWNER' | 'ADMIN' }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const me = useSession()
  const members = useQuery(queries.adminMembers(workspaceId))
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const isOwner = viewerRole === 'OWNER'
  const [query, setQuery] = useState('')
  const [page, setPage] = useState(0)
  /** What became of the last role change, kept in that person's row until the next change: the row, not a passing message, is where the result lives. */
  const [rowResult, setRowResult] = useState<{ userId: string; failed: boolean; retry: () => void } | null>(null)
  useEffect(() => { setPage(0) }, [query])
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminMembers(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
  }
  // Removing a member or disabling an account can leave a Project without an admin and a private channel without a manager.
  const refreshAll = async () => {
    await refresh()
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminProjectsWithoutAdmin(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminChannelsWithoutManager(workspaceId) })
  }

  /**
   * The Projects and private channels that would be left without their last admin or manager, as a sentence for the confirmation. It
   * only informs: if it cannot be asked for, the action is still offered.
   */
  async function impactText(member: WorkspaceMember): Promise<string> {
    try {
      const { projects, channels } = await api.getOffboardingImpact(workspaceId, member.userId)
      if (!projects.length && !channels.length) return ''
      const parts: string[] = []
      if (projects.length) {
        const list = projects.map((project) => `${project.name} (${t(`admin.members.impact.${project.kind}`)})`).join('; ')
        parts.push(`${t('admin.members.impact.intro')} ${list}.`)
      }
      if (channels.length) {
        parts.push(`${t('admin.members.impact.channelsIntro')} ${channels.map((channel) => channel.name).join('; ')}.`)
      }
      return ` ${parts.join(' ')} ${t('admin.members.impact.hint')}`
    } catch {
      return ''
    }
  }

  /** Says what happened to a person's access. It stays until dismissed: it is a permission change, not a passing result. */
  function done(message: string) {
    notify({ message, sticky: true })
  }
  function failed(member: WorkspaceMember) {
    notify({ kind: 'error', message: t('admin.members.failed', { name: member.displayName }) })
  }

  async function remove(member: WorkspaceMember) {
    setRowResult(null)
    const warning = await impactText(member)
    if (!(await confirm({ title: t('admin.members.remove'), body: t('admin.members.removeConfirm', { name: member.displayName }) + warning, confirmLabel: t('admin.members.remove') }))) return
    if (await perform(() => api.removeMember(workspaceId, member.userId), refreshAll)) done(t('admin.members.removeDone', { name: member.displayName }))
    else failed(member)
  }

  async function setDisabled(member: WorkspaceMember, disabled: boolean) {
    setRowResult(null)
    if (disabled) {
      const warning = await impactText(member)
      if (!(await confirm({ title: t('admin.members.disable'), body: t('admin.members.disableConfirm', { name: member.displayName }) + warning, confirmLabel: t('admin.members.disable') }))) return
    }
    if (await perform(() => api.setAccountDisabled(workspaceId, member.userId, disabled), refreshAll)) done(t(disabled ? 'admin.members.disableDone' : 'admin.members.enableDone', { name: member.displayName }))
    else failed(member)
  }

  async function changeRole(member: WorkspaceMember, role: Role) {
    if (role === member.role) return
    // Every role change is confirmed; handing over or taking back ownership says more.
    const sentence = t('admin.members.roleConfirm', { name: member.displayName, from: t(`admin.role.${member.role}`), to: t(`admin.role.${role}`) })
    const ownerNote = role === 'OWNER' || member.role === 'OWNER' ? ` ${t('admin.members.ownerConfirm', { name: member.displayName })}` : ''
    if (!(await confirm({ title: t('admin.members.role'), body: sentence + ownerNote, confirmLabel: t('confirm'), tone: 'primary' }))) return
    setRowResult(null)
    if (await perform(() => api.changeMemberRole(workspaceId, member.userId, role), refresh)) {
      setRowResult({ userId: member.userId, failed: false, retry: () => undefined })
      // Undo changes the role back, and asks again like any role change does.
      notify({ message: t('admin.members.roleDone', { name: member.displayName, role: t(`admin.role.${role}`) }), action: { label: t('notice.undo'), run: () => void changeRole({ ...member, role }, member.role) } })
    } else {
      setRowResult({ userId: member.userId, failed: true, retry: () => void changeRole(member, role) })
      failed(member)
    }
  }

  const needle = query.trim().toLowerCase()
  const matching = (members.data ?? []).filter((member) => !needle || [member.displayName, member.email, ...member.teams.map((team) => team.name)].some((text) => text.toLowerCase().includes(needle)))
  const pages = Math.max(1, Math.ceil(matching.length / MEMBERS_PER_PAGE))
  const current = Math.min(page, pages - 1)
  const shown = matching.slice(current * MEMBERS_PER_PAGE, (current + 1) * MEMBERS_PER_PAGE)

  return (
    <div className="card settings-card" aria-labelledby="admin-members-title">
      <h3 id="admin-members-title">{t('admin.members.title')}</h3>
      <p className="field-hint">{t('admin.members.lead')}</p>
      {members.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {members.isError ? <p className="form-error" role="alert">{t('admin.loadError')}</p> : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {members.data ? (
        <>
          <input type="search" className="admin-member-search" aria-label={t('admin.members.search')} placeholder={t('admin.members.search')} value={query} onChange={(event) => setQuery(event.target.value)} />
          {matching.length === 0 ? <p className="hint" role="status">{t('admin.members.noMatch')}</p> : (
            <div className="admin-table-wrap"><table className="record-table permissions-table members-table">
              <thead>
                <tr>
                  <th>{t('admin.members.name')}</th>
                  <th>{t('admin.members.role')}</th>
                  <th className="admin-col-teams">{t('admin.members.teams')}</th>
                  <th>{t('admin.members.state')}</th>
                  <th><span className="visually-hidden">{t('admin.members.actions')}</span></th>
                </tr>
              </thead>
              <tbody>
                {shown.map((member) => {
                  const self = member.userId === me.data?.id
                  const active = member.state === 'ACTIVE'
                  // An admin manages plain members only; nobody changes themself here.
                  const canRemove = active && !self && (isOwner || member.role === 'MEMBER')
                  const items: ActionItem[] = []
                  if (isOwner && active && !self) {
                    for (const role of ROLES) if (role !== member.role) items.push({ key: `role-${role}`, label: t('admin.members.makeRole', { role: t(`admin.role.${role}`) }), disabled: busy, onSelect: () => void changeRole(member, role) })
                    items.push({ key: 'disable', label: member.accountDisabled ? t('admin.members.enable') : t('admin.members.disable'), disabled: busy, onSelect: () => void setDisabled(member, !member.accountDisabled) })
                  } else if (isOwner && !self) {
                    items.push({ key: 'disable', label: member.accountDisabled ? t('admin.members.enable') : t('admin.members.disable'), disabled: busy, onSelect: () => void setDisabled(member, !member.accountDisabled) })
                  }
                  if (canRemove) items.push({ key: 'remove', label: t('admin.members.remove'), danger: true, disabled: busy, onSelect: () => void remove(member) })
                  const teamNames = member.teams.map((team) => team.name)
                  return (
                    <tr key={member.userId}>
                      <td data-label={t('admin.members.name')}>
                        {member.displayName} {self ? <small>{t('admin.members.you')}</small> : null}
                        <br />
                        <small className="hint">{member.email}</small>
                      </td>
                      <td data-label={t('admin.members.role')}>
                        {t(`admin.role.${member.role}`)}
                        {rowResult?.userId === member.userId ? (
                          rowResult.failed ? (
                            <small className="field-error" role="alert"> {t('admin.members.rowFailed')} · <button type="button" className="link-button" disabled={busy} onClick={rowResult.retry}>{t('admin.members.retry')}</button></small>
                          ) : (
                            <small className="hint" role="status"> {t('admin.members.rowSaved')}</small>
                          )
                        ) : null}
                      </td>
                      <td className="admin-col-teams" data-label={t('admin.members.teams')} title={teamNames.join(', ')}>
                        {teamNames.slice(0, SHOWN_TEAMS).join(', ')}
                        {teamNames.length > SHOWN_TEAMS ? <small className="hint"> {t('admin.members.moreTeams', { count: String(teamNames.length - SHOWN_TEAMS) })}</small> : null}
                      </td>
                      <td data-label={t('admin.members.state')}>
                        <Badge tone={active ? 'accent' : 'neutral'}>{active ? t('admin.members.active') : t('admin.members.removed')}</Badge>{' '}
                        {member.accountDisabled ? <Badge tone="preview">{t('admin.members.disabled')}</Badge> : null}
                      </td>
                      <td className="admin-col-actions">
                        <ActionMenu label={t('admin.members.actionsFor', { name: `${member.displayName} (${member.email})` })} items={items} />
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table></div>
          )}
          {matching.length > MEMBERS_PER_PAGE ? (
            <nav className="admin-pager" aria-label={t('admin.members.pages')}>
              <Button variant="secondary" disabled={current === 0} onClick={() => setPage(current - 1)}>{t('admin.members.prev')}</Button>
              <span role="status">{t('admin.members.count', { from: String(current * MEMBERS_PER_PAGE + 1), to: String(Math.min(matching.length, (current + 1) * MEMBERS_PER_PAGE)), total: String(matching.length) })}</span>
              <Button variant="secondary" disabled={current >= pages - 1} onClick={() => setPage(current + 1)}>{t('admin.members.next')}</Button>
            </nav>
          ) : null}
        </>
      ) : null}
      {dialog}
      {confirmDialog}
    </div>
  )
}
