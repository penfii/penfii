import { activeTeams } from '../spaces/teamState'
import { GearIcon } from '../../design/icons'
import { Link } from 'react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import type { PermissionMember, ProjectPermissions, ProjectRole } from '@penfii/contracts'
import { api } from '../../api'
import { Button, EmptyState, Select } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { TrashSection } from '../admin/TrashSection'
import { PersonPicker } from '../spaces/PersonPicker'
import { AddFromTeam } from './AddFromTeam'
import { CalendarSection, EstimateUnitSection } from '../work/ProjectPlanSettings'

/** A link to the Project's settings, shown only to someone who administers the Project. */
export function SettingsLink({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t } = useI18n()
  const board = useQuery(queries.projectBoard(projectId))
  if (!board.data?.canManageWorkflow) return null
  return (
    <Link className="btn icon-outline" to={`/w/${workspaceId}/projects/${projectId}/settings`} title={t('projectSettings.open')}>
      <GearIcon />
      <span className="visually-hidden">{t('projectSettings.open')}</span>
    </Link>
  )
}

/** The Project's settings page. For now it holds Permissions; Workflow stays on the Work screen. */
export function ProjectSettings({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t } = useI18n()
  const board = useQuery(queries.projectBoard(projectId))
  if (board.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (!board.data?.canManageWorkflow) return <EmptyState title={t('projectSettings.title')} body={t('projectSettings.adminOnly')} />
  return (
    <div className="project-settings">
      <h2>{t('projectSettings.title')}</h2>
      <p className="hint">{t('projectSettings.workflowHint')} <Link to={`/w/${workspaceId}/projects/${projectId}/work`}>{t('projects.openBoard')}</Link></p>
      <EstimateUnitSection projectId={projectId} unit={board.data.estimateUnit ?? 'NONE'} />
      <CalendarSection projectId={projectId} />
      <Permissions workspaceId={workspaceId} projectId={projectId} />
      <TrashSection workspaceId={workspaceId} projectId={projectId} />
    </div>
  )
}

function describe(value: { canWaiveDependencies: boolean; canManagePins: boolean }, t: ReturnType<typeof useI18n>['t']): string {
  const parts = [
    value.canWaiveDependencies ? t('permissions.cap.dependency_waiver') : null,
    value.canManagePins ? t('permissions.cap.pin_management') : null,
  ].filter((part): part is string => part !== null)
  return parts.length > 0 ? parts.join(', ') : t('permissions.none')
}

type Designation = { canWaiveDependencies: boolean; canManagePins: boolean }

const PROJECT_ROLES: ProjectRole[] = ['ADMIN', 'EDITOR', 'VIEWER']

function Permissions({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const permissions = useQuery(queries.projectPermissions(projectId))
  const space = useQuery(queries.space(workspaceId))
  const [error, setError] = useState('')
  const [newRole, setNewRole] = useState<ProjectRole>('VIEWER')
  const [teamToLink, setTeamToLink] = useState('')
  const changed = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectPermissions(projectId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
  }
  // Grants, Team links and Team roles: a failure (for example the last admin) shows one message and the list reloads.
  const access = useMutation({
    mutationFn: (change: () => Promise<void>) => change(),
    onMutate: () => setError(''),
    onError: () => setError(t('permissions.accessError')),
    onSettled: changed,
  })
  const designation = useMutation({
    mutationFn: ({ member, next }: { member: PermissionMember; next: Designation }) => api.setDesignation(projectId, member.userId, next),
    onMutate: () => setError(''),
    onError: () => setError(t('permissions.error')),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.projectPermissions(projectId) })
      await queryClient.invalidateQueries({ queryKey: queryKeys.projectPins(projectId) })
    },
  })
  // What the person just chose shows at once; the server's answer replaces it when it arrives.
  const chosen = designation.isPending ? designation.variables : undefined
  if (permissions.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (permissions.isError || !permissions.data) return <p className="board-error" role="alert">{t('permissions.loadError')}</p>
  const data: ProjectPermissions = permissions.data

  return (
    <section className="permissions" aria-labelledby="permissions-title">
      <h3 id="permissions-title">{t('permissions.title')}</h3>
      <p>{t('permissions.lead')}</p>
      <p className="hint">{t('permissions.directOnly')}</p>
      <table className="record-table permissions-table">
        <thead>
          <tr>
            <th>{t('permissions.member')}</th>
            <th>{t('permissions.role')}</th>
            {data.capabilities.map((capability) => (
              <th key={capability} title={t(`permissions.cap.${capability}.help`)}>{t(`permissions.cap.${capability}`)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.members.map((member) => (
            <tr key={member.userId}>
              <td>
                {member.displayName}{' '}
                <Button variant="ghost" disabled={access.isPending} aria-label={`${t('permissions.remove')} ${member.displayName}`} onClick={() => access.mutate(() => api.removeProjectGrant(projectId, member.userId))}>
                  {t('permissions.remove')}
                </Button>
              </td>
              <td>
                <Select
                  aria-label={`${member.displayName}: ${t('permissions.role')}`}
                  value={member.role}
                  disabled={access.isPending}
                  onChange={(event) => {
                    // Read now: the controlled value is put back before the request runs.
                    const role = event.target.value as ProjectRole
                    access.mutate(() => api.setProjectGrant(projectId, member.userId, role))
                  }}
                >
                  {PROJECT_ROLES.map((role) => <option key={role} value={role}>{t(`permissions.role.${role}`)}</option>)}
                </Select>
              </td>
              {data.capabilities.map((capability) => {
                const key = capability === 'dependency_waiver' ? 'canWaiveDependencies' : 'canManagePins'
                const current = { canWaiveDependencies: member.canWaiveDependencies, canManagePins: member.canManagePins }
                const mine = chosen?.member.userId === member.userId
                return (
                  <td key={capability}>
                    {member.designatable ? (
                      <label>
                        <input
                          type="checkbox"
                          checked={mine ? chosen.next[key] : current[key]}
                          disabled={mine}
                          aria-label={`${member.displayName}: ${t(`permissions.cap.${capability}`)}`}
                          onChange={(event) => designation.mutate({ member, next: { ...current, [key]: event.target.checked } })}
                        />
                      </label>
                    ) : (
                      <small className="hint">{member.role === 'ADMIN' ? '✓' : t('permissions.notDesignatable')}</small>
                    )}
                  </td>
                )
              })}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="row-actions">
        <Select aria-label={t('permissions.newRole')} value={newRole} onChange={(event) => setNewRole(event.target.value as ProjectRole)}>
          {PROJECT_ROLES.map((role) => <option key={role} value={role}>{t(`permissions.role.${role}`)}</option>)}
        </Select>
        <PersonPicker
          workspaceId={workspaceId}
          taken={new Set(data.members.map((member) => member.userId))}
          label={t('permissions.addPerson')}
          onPick={(person) => access.mutate(() => api.setProjectGrant(projectId, person.id, newRole))}
        />
      </div>
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      <AddFromTeam projectId={projectId} workspaceTeams={activeTeams(space.data?.teams ?? [])} data={data} busy={access.isPending} run={(change) => access.mutate(change)} />
      <h4>{t('permissions.teams')}</h4>
      <p className="hint">{t('permissions.teamsHint')}</p>
      {data.teams.length === 0 ? <p className="hint">{t('permissions.teamsNone')}</p> : null}
      <ul className="plain-list">
        {data.teams.map((team) => (
          <li key={team.teamId} className="row-actions">
            <span>{team.name}{team.archivedAt ? ` (${t('teams.archivedTag')})` : ''} · {t('permissions.members', { count: String(team.memberCount) })}</span>
            <Select
              aria-label={`${team.name}: ${t('permissions.teamRole')}`}
              value={team.role ?? ''}
              disabled={access.isPending}
              onChange={(event) => {
                const role = event.target.value === '' ? null : (event.target.value as ProjectRole)
                access.mutate(() => api.setProjectTeamRole(projectId, team.teamId, role))
              }}
            >
              <option value="">{t('permissions.teamLinkedOnly')}</option>
              {PROJECT_ROLES.map((role) => <option key={role} value={role}>{t(`permissions.role.${role}`)}</option>)}
            </Select>
            <Button variant="ghost" disabled={access.isPending} aria-label={`${t('permissions.unlink')} ${team.name}`} onClick={() => access.mutate(() => api.unlinkProjectTeam(projectId, team.teamId))}>
              {t('permissions.unlink')}
            </Button>
          </li>
        ))}
      </ul>
      <div className="row-actions">
        <Select aria-label={t('permissions.linkTeam')} value={teamToLink} onChange={(event) => setTeamToLink(event.target.value)}>
          <option value="">—</option>
          {activeTeams(space.data?.teams ?? []).filter((team) => !data.teams.some((linked) => linked.teamId === team.id)).map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
        </Select>
        <Button variant="secondary" disabled={access.isPending || teamToLink === ''} onClick={() => { access.mutate(() => api.linkProjectTeam(projectId, teamToLink)); setTeamToLink('') }}>
          {t('permissions.link')}
        </Button>
      </div>
      {data.viaGroups.length > 0 ? (
        <>
          <h4>{t('permissions.viaGroups')}</h4>
          <ul>{data.viaGroups.map((group) => <li key={group.groupId}>{group.name} · {group.role} · {t('permissions.members', { count: String(group.memberCount) })}</li>)}</ul>
        </>
      ) : null}
      <h4>{t('permissions.recent')}</h4>
      {data.recentChanges.length === 0 ? (
        <p className="hint">{t('permissions.recentNone')}</p>
      ) : (
        <ul className="permission-log">
          {data.recentChanges.map((change, index) => (
            <li key={`${change.at}:${index}`}>
              <small className="hint">{formatStamp(change.at, locale)}</small>{' '}
              {t('permissions.changeLine', {
                actor: change.actorName ?? '—',
                user: change.userName ?? '—',
                before: describe(change.before, t),
                after: describe(change.after, t),
              })}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
