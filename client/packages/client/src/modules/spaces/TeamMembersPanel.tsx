import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { TeamMember, TeamRole, WorkspacePerson } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from '../admin/useAdminAction'
import { PersonButton } from '../people/PersonButton'
import { PersonPicker } from './PersonPicker'
import { refreshTeamData } from './teamState'

export const TEAM_ROLES: TeamRole[] = ['MANAGER', 'MEMBER']

/** A person the roster names but the people list does not (yet): enough to show a card for. */
function rosterPerson(member: TeamMember): WorkspacePerson {
  return { userId: member.userId, displayName: member.displayName, email: member.email, hasAvatar: false, statusEmoji: null, statusText: null, jobTitle: null, timeZone: null, role: 'MEMBER', state: 'ACTIVE', statusClearAt: null, teams: [] }
}

/**
 * A Team's members: everyone in the Workspace can read the roster. Whoever runs the Team (a Workspace owner or admin, or its
 * manager) can also change roles, remove people and add them. The Workspace role is shown to owners and admins only.
 */
export function TeamMembersPanel({
  workspaceId,
  teamId,
  archived = false,
  canManage,
  showWorkspaceRole = false,
}: {
  workspaceId: string
  teamId: string
  archived?: boolean
  canManage: boolean
  showWorkspaceRole?: boolean
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const members = useQuery(queries.adminTeamMembers(workspaceId, teamId))
  const people = useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
  const [search, setSearch] = useState('')
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [notice, setNotice] = useState<string | null>(null)
  const [report, setReport] = useState<Array<{ name: string; text: string }>>([])
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminTeamMembers(workspaceId, teamId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.adminMembers(workspaceId) })
    await refreshTeamData(queryClient, workspaceId)
  }
  const editable = canManage && !archived
  const inTeam = new Set(members.data?.map((member) => member.userId))
  // Removing several people: the last-manager rule is checked on the whole set before anything runs.
  async function removePicked() {
    const rows = members.data?.filter((member) => picked.has(member.userId)) ?? []
    if (rows.length === 0) return
    const managers = members.data?.filter((member) => member.role === 'MANAGER') ?? []
    if (managers.length > 0 && managers.every((manager) => picked.has(manager.userId))) {
      setNotice(t('batch.lastManager'))
      return
    }
    setNotice(null)
    if (!(await confirm({ title: t('batch.removeTitle', { count: String(rows.length) }), body: t('batch.removeBody'), confirmLabel: t('batch.remove') }))) return
    const lines: Array<{ name: string; text: string }> = []
    for (const member of rows) {
      try {
        await api.removeTeamMember(workspaceId, teamId, member.userId)
        lines.push({ name: member.displayName, text: t('batch.removed') })
      } catch {
        lines.push({ name: member.displayName, text: t('batch.failed', { reason: t('batch.reason.generic') }) })
      }
    }
    setReport(lines)
    setPicked(new Set())
    await refresh()
  }
  if (members.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (members.isError) return <p className="form-error" role="alert">{t('error.generic')}</p>
  const needle = search.trim().toLowerCase()
  const shown = members.data.filter((member) => needle === '' || member.displayName.toLowerCase().includes(needle) || member.email.toLowerCase().includes(needle))
  return (
    <section className="team-members" aria-labelledby={`team-members-${teamId}`}>
      <h4 id={`team-members-${teamId}`}>{t('admin.teams.membersTitle')}</h4>
      <p className="hint">{t('admin.teams.accessHint')}</p>
      <p className="hint">{t('teams.rosterHint')}</p>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {archived ? <p className="hint">{t('teams.archived.noMembers')}</p> : null}
      <label className="field team-roster-search">
        <span>{t('teamPage.search')}</span>
        <input value={search} onChange={(event) => setSearch(event.target.value)} />
      </label>
      {editable && picked.size > 0 ? (
        <div className="batch-bar" role="group" aria-label={t('batch.remove')}>
          <span>{t('batch.selected', { count: String(picked.size) })}</span>
          <Button type="button" variant="secondary" onClick={() => setPicked(new Set())}>{t('batch.clear')}</Button>
          <Button type="button" variant="danger" disabled={busy} onClick={() => void removePicked()}>{t('batch.remove')}</Button>
        </div>
      ) : null}
      {notice ? <p className="form-error" role="alert">{notice}</p> : null}
      {report.length > 0 ? (
        <ul className="plain-list" role="status">{report.map((line) => <li key={line.name}>{line.name}: {line.text}</li>)}</ul>
      ) : null}
      {members.data.length === 0 ? <p className="hint">{t('admin.teams.empty')}</p> : null}
      {members.data.length > 0 && shown.length === 0 ? <p className="hint">{t('teamPage.noMatch')}</p> : null}
      {shown.length > 0 ? (
        <div className="admin-table-wrap"><table className="record-table permissions-table team-roster">
          <thead>
            <tr>
              {editable ? <th /> : null}
              <th>{t('admin.members.name')}</th>
              <th>{t('teamPage.col.teamRole')}</th>
              <th>{t('teamPage.col.other')}</th>
              {showWorkspaceRole ? <th>{t('teamPage.col.workspaceRole')}</th> : null}
              {editable ? <th /> : null}
            </tr>
          </thead>
          <tbody>
            {shown.map((member) => {
              const person = people.data?.find((item) => item.userId === member.userId)
              const others = (person?.teams ?? []).filter((team) => team.id !== teamId)
              // Until the people list arrives, the roster alone cannot say which other Teams or Workspace role this person has.
              const known = people.isSuccess
              return (
                <tr key={member.userId}>
                  {editable ? (
                    <td>
                      <input
                        type="checkbox"
                        aria-label={`${t('batch.select')} ${member.displayName}`}
                        checked={picked.has(member.userId)}
                        onChange={(event) => setPicked((current) => {
                          const next = new Set(current)
                          if (event.target.checked) next.add(member.userId)
                          else next.delete(member.userId)
                          return next
                        })}
                      />
                    </td>
                  ) : null}
                  <td data-label={t('admin.members.name')}>
                    {known ? <PersonButton workspaceId={workspaceId} person={person ?? rosterPerson(member)} /> : member.displayName}
                  </td>
                  <td className="cell-nowrap" data-label={t('teamPage.col.teamRole')}>
                    {editable ? (
                      <Select
                        aria-label={`${member.displayName}: ${t('admin.teams.role')}`}
                        value={member.role}
                        disabled={busy}
                        onChange={(event) => {
                          const role = event.target.value as TeamRole
                          void perform(() => api.addTeamMember(workspaceId, teamId, member.userId, role), refresh)
                        }}
                      >
                        {TEAM_ROLES.map((role) => <option key={role} value={role}>{t(`team.role.${role}`)}</option>)}
                      </Select>
                    ) : t(`team.role.${member.role as TeamRole}`)}
                  </td>
                  <td data-label={t('teamPage.col.other')}>{known ? (others.length > 0 ? others.map((team) => team.name).join(', ') : t('teamPage.none')) : t('loading')}</td>
                  {showWorkspaceRole ? <td className="cell-nowrap" data-label={t('teamPage.col.workspaceRole')}>{known ? (person ? t(`admin.role.${person.role as 'OWNER' | 'ADMIN' | 'MEMBER'}`) : t('teamPage.none')) : t('loading')}</td> : null}
                  {editable ? (
                    <td>
                      <Button variant="ghost" disabled={busy} aria-label={`${t('admin.teams.remove')} ${member.displayName}`} onClick={() => void perform(() => api.removeTeamMember(workspaceId, teamId, member.userId), refresh)}>
                        {t('admin.teams.remove')}
                      </Button>
                    </td>
                  ) : null}
                </tr>
              )
            })}
          </tbody>
        </table></div>
      ) : null}
      {editable ? (
        <PersonPicker
          workspaceId={workspaceId}
          taken={inTeam}
          label={t('teamPage.addMembers')}
          preferUnaffiliated
          onPick={(person) => void perform(() => api.addTeamMember(workspaceId, teamId, person.id, 'MEMBER'), refresh)}
        />
      ) : null}
      {dialog}
      {confirmDialog}
    </section>
  )
}
