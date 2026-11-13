import { Link, NavLink, useLocation } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { Department, Team } from '@penfii/contracts'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { isWorkspaceManager } from '../modules/spaces/capabilities'
import { activeTeams } from '../modules/spaces/teamState'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { queries } from '../queries'
import { OpenCreate } from '../shell/WorkspaceShell'

/**
 * The Teams section of the side list. It lives apart from the Teams pages on purpose: the shell shows it
 * on every screen, while the pages (org chart, member directory, Team detail) load only when opened.
 */
export function TeamsNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const session = useSession()
  const role = useWorkspaceRole(workspaceId)
  const { pathname, search } = useLocation()
  // The members page has two entries that differ only by the query, so the link must look at it too.
  const onMembers = pathname.endsWith('/members')
  const onlyNone = new URLSearchParams(search).get('team') === 'none'
  const space = useQuery(queries.space(workspaceId))
  const people = useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
  const me = (people.data ?? []).find((person) => person.userId === session.data?.id)
  const mine = new Set(me?.teams.map((team) => team.id) ?? [])
  const departments = useQuery(queries.departments(workspaceId))
  const unaffiliated = (people.data ?? []).filter((person) => person.state === 'ACTIVE' && person.teams.length === 0).length
  const teams = activeTeams(space.data?.teams ?? [])
  const tree = departments.data ?? []
  const roots = tree.filter((department) => department.parentId === null)
  const unplaced = teams.filter((team) => !team.departmentId || !tree.some((department) => department.id === team.departmentId))
  return (
    <>
      {isWorkspaceManager(role) ? <OpenCreate kind="team" label={`+ ${t('create.team')}`} /> : null}
      <ul>
        <li><NavLink end className="tree-link" to={`/w/${workspaceId}/directory`}>{t('teams.org')}</NavLink></li>
        <li><Link className="tree-link" aria-current={onMembers && !onlyNone ? 'page' : undefined} to={`/w/${workspaceId}/directory/members`}>{t('teams.members')}</Link></li>
        <li><Link className="tree-link" aria-current={onMembers && onlyNone ? 'page' : undefined} to={`/w/${workspaceId}/directory/members?team=none`}>{t('teams.unaffiliated')}{unaffiliated ? ` ${unaffiliated}` : ''}</Link></li>
      </ul>
      {roots.map((department) => (
        <DepartmentLinks key={department.id} workspaceId={workspaceId} department={department} departments={tree} teams={teams} mine={mine} />
      ))}
      {unplaced.length > 0 ? (
        <section className="tree-block">
          <h3>{t('teams.unplaced')}</h3>
          <ul>
            {unplaced.map((team) => (
              <li key={team.id}><NavLink className="tree-link" to={`/w/${workspaceId}/directory/teams/${team.id}`}>{team.name}{mine.has(team.id) ? ` · ${t('teams.mine')}` : ''}</NavLink></li>
            ))}
          </ul>
        </section>
      ) : null}
    </>
  )
}

function DepartmentLinks({
  workspaceId,
  department,
  departments,
  teams,
  mine,
}: {
  workspaceId: string
  department: Department
  departments: Department[]
  teams: Team[]
  mine: Set<string>
}) {
  const { t } = useI18n()
  const here = teams.filter((team) => team.departmentId === department.id)
  const children = departments.filter((item) => item.parentId === department.id)
  return (
    <section className="tree-block">
      <h3>{department.name}</h3>
      <ul>
        {here.map((team) => (
          <li key={team.id}><NavLink className="tree-link" to={`/w/${workspaceId}/directory/teams/${team.id}`}>{team.name}{mine.has(team.id) ? ` · ${t('teams.mine')}` : ''}</NavLink></li>
        ))}
      </ul>
      {children.map((child) => (
        <DepartmentLinks key={child.id} workspaceId={workspaceId} department={child} departments={departments} teams={teams} mine={mine} />
      ))}
    </section>
  )
}
