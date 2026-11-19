import { Link, useParams, useSearchParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { Department, Team } from '@penfii/contracts'
import { Badge, EmptyState } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { isWorkspaceManager } from '../modules/spaces/capabilities'
import { departmentPath, distinctPeople } from '../modules/spaces/orgTree'
import { activeTeams, isArchivedTeam } from '../modules/spaces/teamState'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { queries } from '../queries'
import { OpenCreate } from '../shell/WorkspaceShell'

export type TeamScope = 'all' | 'mine' | 'unplaced'

export function teamScope(value: string | null): TeamScope {
  return value === 'mine' || value === 'unplaced' ? value : 'all'
}

/** The path of department names above a Team, or an empty list for a Team in no department (or in one that is gone). */
export function teamPath(team: Team, departments: Department[]): string[] {
  return departmentPath(departments, team.departmentId ?? null)
}

/** The Teams the text matches by name or by the names of the departments above them. */
export function filterTeams(teams: Team[], departments: Department[], mine: ReadonlySet<string>, scope: TeamScope, query: string): Team[] {
  const needle = query.trim().toLowerCase()
  return teams
    .filter((team) => {
      const path = teamPath(team, departments)
      if (scope === 'mine' && !mine.has(team.id)) return false
      if (scope === 'unplaced' && path.length > 0) return false
      if (needle === '') return true
      return team.name.toLowerCase().includes(needle) || path.some((name) => name.toLowerCase().includes(needle))
    })
    .sort((a, b) => a.name.localeCompare(b.name))
}

/**
 * Every Team the person may see, as its own place in the global Teams: search by name or department, narrow to My teams
 * or Teams with no department, and open the Team. Same-named Teams differ by their department path. Search and scope
 * live in the address (replace), so back and refresh return to them and nothing is shared with anyone else.
 */
export function AllTeams() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const session = useSession()
  const role = useWorkspaceRole(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const departments = useQuery(queries.departments(workspaceId))
  const people = useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
  const query = params.get('q') ?? ''
  const scope = teamScope(params.get('filter'))
  const showArchived = params.get('archived') === '1'
  const everyone = people.data ?? []
  const allTeams = space.data?.teams ?? []
  const hasArchived = allTeams.some(isArchivedTeam)
  const base = showArchived ? allTeams : activeTeams(allTeams)
  const mine = new Set(everyone.find((person) => person.userId === session.data?.id)?.teams.map((team) => team.id) ?? [])
  const tree = departments.data ?? []
  const shown = filterTeams(base, tree, mine, scope, query)
  const update = (patch: { q?: string; filter?: TeamScope; archived?: boolean }) =>
    setParams((current) => {
      const next = new URLSearchParams(current)
      const q = patch.q ?? query
      const filter = patch.filter ?? scope
      const archived = patch.archived ?? showArchived
      if (q) next.set('q', q)
      else next.delete('q')
      if (filter === 'all') next.delete('filter')
      else next.set('filter', filter)
      if (archived) next.set('archived', '1')
      else next.delete('archived')
      return next
    }, { replace: true })
  const loading = space.isPending || departments.isPending
  return (
    <div className="page-scroll teams-page">
      <header className="page-intro project-index-head">
        <div>
          <h1>{t('teams.all')}</h1>
          <p className="hint">{t('teams.all.lead')}</p>
        </div>
        {isWorkspaceManager(role) ? <OpenCreate kind="team" label={`+ ${t('create.team')}`} /> : null}
      </header>
      <div className="project-browse-tools">
        <label className="field org-search">
          <span>{t('teams.all.search')}</span>
          <input type="search" value={query} onChange={(event) => update({ q: event.target.value })} />
        </label>
        <div className="view-layouts" role="group" aria-label={t('teams.all.filter')}>
          {(['all', 'mine', 'unplaced'] as const).map((item) => (
            <button key={item} type="button" className={item === scope ? 'view-layout on' : 'view-layout'} aria-pressed={item === scope} onClick={() => update({ filter: item })}>
              {t(`teams.all.filter.${item}`)}
            </button>
          ))}
        </div>
        {hasArchived ? (
          <label className="switch-line">
            <input type="checkbox" checked={showArchived} onChange={(event) => update({ archived: event.target.checked })} />
            <span>{t('teams.showArchived')}</span>
          </label>
        ) : null}
      </div>
      {loading ? <p className="hint" role="status">{t('loading')}</p> : null}
      {space.isError || departments.isError ? <p className="form-error" role="alert">{t('error.generic')}</p> : null}
      {!loading ? <p className="hint" role="status">{t('teams.all.count', { shown: String(shown.length), total: String(base.length) })}</p> : null}
      {!loading && base.length === 0 ? (
        <EmptyState title={t('teams.all.empty')} body={t('teams.none')} />
      ) : !loading && shown.length === 0 ? (
        scope === 'mine' && query.trim() === '' ? (
          <EmptyState title={t('teams.all.noMine')} body={t('teams.all.noMineBody')}>
            <button type="button" className="btn btn-primary" onClick={() => update({ filter: 'all' })}>{t('teams.all.filter.all')}</button>
          </EmptyState>
        ) : (
          <EmptyState title={t('teams.all.noMatch')} body={t('teams.all.noMatchBody')}>
            <button type="button" className="btn btn-secondary" onClick={() => setParams({}, { replace: true })}>{t('teams.all.clear')}</button>
          </EmptyState>
        )
      ) : (
        <ul className="all-teams">
          {shown.map((team) => {
            const path = teamPath(team, tree)
            const count = distinctPeople(everyone, new Set([team.id]))
            return (
              <li key={team.id} className="all-team-row">
                <div>
                  <Link className="all-team-name" to={`/w/${workspaceId}/directory/teams/${team.id}`}>{team.name}</Link>
                  {mine.has(team.id) ? <> <Badge tone="accent">{t('teams.mine')}</Badge></> : null}
                  {isArchivedTeam(team) ? <> <Badge>{t('teams.archivedTag')}</Badge></> : null}
                  <small className="hint all-team-path">{path.length > 0 ? path.join(' › ') : t('teams.unplacedTeam')}</small>
                </div>
                <small className="hint">{t('teams.all.members', { count: String(count) })}</small>
                <Link className="all-team-org" to={`/w/${workspaceId}/directory?focus=team:${team.id}`} aria-label={t('teams.viewInOrgFor', { name: team.name })}>{t('teams.viewInOrg')}</Link>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
