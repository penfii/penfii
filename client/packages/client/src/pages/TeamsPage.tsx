import { useMemo, useState } from 'react'
import { Link, useLocation, useParams, useSearchParams } from 'react-router'
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import type { Department, Team, WorkspacePerson } from '@penfii/contracts'
import { api } from '../api'
import { Badge, Button, Select, Tabs, useConfirm } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { PersonButton } from '../modules/people/PersonButton'
import { BatchAssignDialog } from '../modules/spaces/BatchAssignDialog'
import { OrgStructureEditor } from '../modules/spaces/OrgStructureEditor'
import { branchCounts, distinctPeople, matchOrg, subtreeIds, type OrgMove } from '../modules/spaces/orgTree'
import { TeamArchiveSection, RestoreTeamButton } from '../modules/spaces/TeamArchiveSection'
import { TeamChannelsTab } from '../modules/spaces/TeamChannelsTab'
import { TeamInfoForm } from '../modules/spaces/TeamInfoForm'
import { TeamMembersPanel } from '../modules/spaces/TeamMembersPanel'
import { TeamPreview } from '../modules/spaces/TeamPreview'
import { TeamProjectsTab } from '../modules/spaces/TeamProjectsTab'
import { activeTeams, isArchivedTeam } from '../modules/spaces/teamState'
import '../modules/spaces/teamLifecycle.css'
import '../modules/spaces/orgChart.css'
import { formatStamp } from '../format'
import { isWorkspaceManager } from '../modules/spaces/capabilities'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { Work } from '../modules/work/Work'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { OpenCreate } from '../shell/WorkspaceShell'

export function TeamsPage() {
  const { teamId } = useParams()
  const { pathname } = useLocation()
  if (pathname.endsWith('/members')) return <MembersDirectory />
  if (teamId) return <TeamDetail teamId={teamId} />
  return <OrgList />
}

function usePeople(workspaceId: string) {
  return useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
}

function OrgList() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const session = useSession()
  const role = useWorkspaceRole(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const departments = useQuery(queries.departments(workspaceId))
  const people = usePeople(workspaceId)
  const { confirm, dialog } = useConfirm()
  const [editing, setEditing] = useState(false)
  const [moves, setMoves] = useState<OrgMove[]>([])
  const [showArchived, setShowArchived] = useState(false)
  const [query, setQuery] = useState('')
  const [scope, setScope] = useState<'all' | 'mine'>('all')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const everyone = people.data ?? []
  const mine = new Set(everyone.find((person) => person.userId === session.data?.id)?.teams.map((team) => team.id) ?? [])
  const allTeams = space.data?.teams ?? []
  const hasArchived = allTeams.some(isArchivedTeam)
  const liveTeams = activeTeams(allTeams)
  const teams = showArchived ? allTeams : liveTeams
  const tree = departments.data ?? []
  const roots = tree.filter((department) => department.parentId === null)
  const filtering = query.trim() !== '' || scope === 'mine'
  const match = matchOrg(query, tree, teams, everyone)
  const shownTeams = teams.filter((team) => match.teamIds.has(team.id) && (scope === 'all' || mine.has(team.id)))
  // With a filter on, only departments that lead to a shown Team stay.
  const departmentShown = (department: Department) => !filtering || shownTeams.some((team) => subtreeIds(tree, department.id).has(team.departmentId ?? ''))
  const unplaced = shownTeams.filter((team) => !team.departmentId || !tree.some((department) => department.id === team.departmentId))
  const needle = query.trim().toLowerCase()
  const unaffiliated = everyone.filter((person) => person.state === 'ACTIVE' && person.teams.length === 0)
  const shownUnaffiliated = unaffiliated.filter((person) => needle === '' || person.displayName.toLowerCase().includes(needle) || person.email.toLowerCase().includes(needle))
  const selected = shownTeams.find((team) => team.id === selectedId) ?? null
  const nothing = filtering && shownTeams.length === 0 && (scope === 'mine' || shownUnaffiliated.length === 0)
  async function toggleEditing() {
    if (editing && moves.length > 0) {
      if (!(await confirm({ title: t('orgEdit.leaveTitle'), body: t('orgEdit.leaveBody', { count: String(moves.length) }), confirmLabel: t('orgEdit.leave') }))) return
      setMoves([])
    }
    setEditing((value) => !value)
  }
  return (
    <div className="page-scroll teams-page">
      <header className="page-intro">
        <h1>{t('teams.org')}</h1>
        <p className="hint">{t('teams.lead')}</p>
        <p className="hint org-summary">
          {t('org.summary', { departments: String(tree.length), teams: String(liveTeams.length), people: String(everyone.filter((person) => person.state === 'ACTIVE').length) })}
        </p>
        {isWorkspaceManager(role) ? (
          <Button type="button" variant="secondary" onClick={() => void toggleEditing()}>
            {editing ? t('teams.editDone') : t('teams.edit')}
          </Button>
        ) : null}
      </header>
      {editing ? null : (
        <div className="org-toolbar">
          <label className="field org-search">
            <span>{t('org.search')}</span>
            <input value={query} onChange={(event) => setQuery(event.target.value)} />
          </label>
          <div className="row-actions" role="group" aria-label={t('org.scope')}>
            <Button type="button" variant={scope === 'all' ? 'primary' : 'secondary'} aria-pressed={scope === 'all'} onClick={() => setScope('all')}>{t('org.scope.all')}</Button>
            <Button type="button" variant={scope === 'mine' ? 'primary' : 'secondary'} aria-pressed={scope === 'mine'} onClick={() => setScope('mine')}>{t('org.scope.mine')}</Button>
          </div>
          {hasArchived ? (
            <label className="switch-line">
              <input type="checkbox" checked={showArchived} onChange={(event) => setShowArchived(event.target.checked)} />
              <span>{t('teams.showArchived')}</span>
            </label>
          ) : null}
        </div>
      )}
      {space.isPending || departments.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {space.isError || departments.isError || people.isError ? <p className="form-error" role="alert">{t('error.generic')}</p> : null}
      {editing ? (
        <OrgStructureEditor workspaceId={workspaceId} departments={tree} teams={liveTeams} moves={moves} onMoves={setMoves} />
      ) : (
        <>
          {allTeams.length === 0 && tree.length === 0 && space.isSuccess ? <p className="hint">{t('teams.none')}</p> : null}
          {nothing ? <p className="hint" role="status">{scope === 'mine' && needle === '' ? t('org.noMine') : t('org.noMatch')}</p> : null}
          <div className={selected ? 'org-layout has-preview' : 'org-layout'}>
            <div className="org-tree">
              {roots.filter(departmentShown).map((department) => (
                <DepartmentBranch
                  key={department.id}
                  workspaceId={workspaceId}
                  department={department}
                  departments={tree}
                  teams={shownTeams}
                  counted={teams}
                  people={everyone}
                  mine={mine}
                  selectedId={selectedId}
                  onSelect={setSelectedId}
                  visible={departmentShown}
                />
              ))}
              {unplaced.length > 0 ? (
                <section className="card settings-card">
                  <h2>{t('teams.unplaced')}</h2>
                  <TeamCards workspaceId={workspaceId} teams={unplaced} people={everyone} mine={mine} selectedId={selectedId} onSelect={setSelectedId} />
                </section>
              ) : null}
              {scope === 'all' && (needle === '' || shownUnaffiliated.length > 0) ? (
                <section className="card settings-card">
                  <h2>{t('org.unaffiliatedTitle', { count: String(shownUnaffiliated.length) })}</h2>
                  <p className="hint">{t('org.unaffiliatedLead')}</p>
                  {shownUnaffiliated.length > 0 ? <PeopleList people={shownUnaffiliated} workspaceId={workspaceId} /> : null}
                </section>
              ) : null}
            </div>
            {selected ? <TeamPreview key={selected.id} workspaceId={workspaceId} team={selected} departments={tree} onClose={() => setSelectedId(null)} /> : null}
          </div>
        </>
      )}
      {dialog}
    </div>
  )
}

function DepartmentBranch({
  workspaceId,
  department,
  departments,
  teams,
  counted,
  people,
  mine,
  selectedId,
  onSelect,
  visible,
}: {
  workspaceId: string
  department: Department
  departments: Department[]
  /** The Teams that are shown. */
  teams: Team[]
  /** The Teams the counts are taken over, whatever the filter. */
  counted: Team[]
  people: WorkspacePerson[]
  mine: Set<string>
  selectedId: string | null
  onSelect: (id: string | null) => void
  visible: (department: Department) => boolean
}) {
  const { t } = useI18n()
  const children = departments.filter((item) => item.parentId === department.id && visible(item))
  const here = teams.filter((team) => team.departmentId === department.id)
  const counts = branchCounts(departments, counted, people, department.id)
  return (
    <section className="department-branch">
      <h2>{department.name}</h2>
      <p className="hint">{t('org.branch', { teams: String(counts.teams), people: String(counts.people) })}</p>
      <TeamCards workspaceId={workspaceId} teams={here} people={people} mine={mine} selectedId={selectedId} onSelect={onSelect} />
      {children.map((child) => (
        <DepartmentBranch key={child.id} workspaceId={workspaceId} department={child} departments={departments} teams={teams} counted={counted} people={people} mine={mine} selectedId={selectedId} onSelect={onSelect} visible={visible} />
      ))}
    </section>
  )
}

function TeamCards({
  workspaceId,
  teams,
  people,
  mine,
  selectedId,
  onSelect,
}: {
  workspaceId: string
  teams: Team[]
  people: WorkspacePerson[]
  mine: Set<string>
  selectedId: string | null
  onSelect: (id: string | null) => void
}) {
  const { t } = useI18n()
  if (teams.length === 0) return null
  return (
    <ul className="team-cards">
      {teams.map((team) => {
        const count = distinctPeople(people, new Set([team.id]))
        const archived = isArchivedTeam(team)
        return (
          <li key={team.id} className={['card team-card', mine.has(team.id) ? 'is-mine' : '', archived ? 'is-archived' : '', selectedId === team.id ? 'is-selected' : ''].filter(Boolean).join(' ')}>
            <button type="button" className="team-card-name" aria-pressed={selectedId === team.id} aria-label={t('org.preview', { name: team.name })} onClick={() => onSelect(selectedId === team.id ? null : team.id)}>
              <strong>{team.name}</strong>
            </button>
            {archived ? <Badge>{t('teams.archivedTag')}</Badge> : null}
            {mine.has(team.id) ? <span className="hint">{t('teams.mine')}</span> : null}
            {team.description ? <p className="hint team-card-description">{team.description}</p> : null}
            {archived ? null : <p className="hint">{t('org.people', { count: String(count) })}</p>}
            <Link to={`/w/${workspaceId}/directory/teams/${team.id}`}>{t('teams.open')}</Link>
          </li>
        )
      })}
    </ul>
  )
}

/** The Teams the viewer may put people into: every active Team for an owner/admin, only the ones they manage for a Team manager. */
function useAssignableTeams(workspaceId: string): Team[] {
  const session = useSession()
  const role = useWorkspaceRole(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const people = usePeople(workspaceId)
  const active = activeTeams(space.data?.teams ?? [])
  const manager = isWorkspaceManager(role)
  const mine = new Set((people.data ?? []).find((person) => person.userId === session.data?.id)?.teams.map((team) => team.id) ?? [])
  const candidates = manager ? [] : active.filter((team) => mine.has(team.id))
  const rosters = useQueries({ queries: candidates.map((team) => queries.adminTeamMembers(workspaceId, team.id)) })
  if (manager) return active
  return candidates.filter((_, index) => rosters[index]?.data?.some((member) => member.userId === session.data?.id && member.role === 'MANAGER'))
}

function MembersDirectory() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const role = useWorkspaceRole(workspaceId)
  const manager = isWorkspaceManager(role)
  const people = usePeople(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const assignable = useAssignableTeams(workspaceId)
  const [query, setQuery] = useState('')
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [assigning, setAssigning] = useState(false)
  // The selection stays while the result is on screen, and is cleared when the dialog closes after a run.
  const [finished, setFinished] = useState(false)
  const everyone = people.data ?? []
  const view: 'all' | 'none' | 'removed' = manager && params.get('state') === 'removed' ? 'removed' : params.get('team') === 'none' ? 'none' : 'all'
  const roleFilter = manager ? params.get('role') ?? '' : ''
  const teamFilter = params.get('in') ?? ''
  const active = everyone.filter((person) => person.state === 'ACTIVE')
  const counts = { all: active.length, none: active.filter((person) => person.teams.length === 0).length, removed: everyone.length - active.length }
  const needle = query.trim().toLowerCase()
  const shown = everyone.filter((person) => {
    if (view === 'removed' ? person.state === 'ACTIVE' : person.state !== 'ACTIVE') return false
    if (view === 'none' && person.teams.length > 0) return false
    if (roleFilter && person.role !== roleFilter) return false
    if (teamFilter && view !== 'none' && !person.teams.some((team) => team.id === teamFilter)) return false
    if (!needle) return true
    return person.displayName.toLowerCase().includes(needle) || person.email.toLowerCase().includes(needle)
  })
  const selectable = assignable.length > 0 && view !== 'removed'
  const chosen = shown.filter((person) => picked.has(person.userId))
  const selectableRows = shown.filter((person) => person.state === 'ACTIVE')
  function setParam(key: string, value: string | null) {
    const next = new URLSearchParams(params)
    if (value) next.set(key, value)
    else next.delete(key)
    setParams(next, { replace: true })
  }
  function setView(next: string) {
    const params2 = new URLSearchParams(params)
    params2.delete('team')
    params2.delete('state')
    if (next === 'none') params2.set('team', 'none')
    if (next === 'removed') params2.set('state', 'removed')
    setParams(params2, { replace: true })
  }
  return (
    <div className="page-scroll teams-page">
      <header className="page-intro">
        <h1>{view === 'none' ? t('teams.unaffiliated') : t('teams.members')}</h1>
        <p className="hint">{t('members.lead')}</p>
        {manager ? <OpenCreate kind="invite" label={t('create.invite')} /> : null}
      </header>
      <Tabs
        tabs={[
          { id: 'all', label: t('members.filter.all', { count: String(counts.all) }) },
          { id: 'none', label: t('members.filter.none', { count: String(counts.none) }) },
          ...(manager ? [{ id: 'removed', label: t('members.filter.removed', { count: String(counts.removed) }) }] : []),
        ]}
        value={view}
        onChange={setView}
      />
      <div className="row-actions members-filters">
        <FieldSearch label={t('teams.search')} value={query} onChange={setQuery} />
        {manager ? (
          <Select aria-label={t('members.col.role')} value={roleFilter} onChange={(event) => setParam('role', event.target.value || null)}>
            <option value="">{t('members.filter.role')}</option>
            {(['OWNER', 'ADMIN', 'MEMBER'] as const).map((value) => <option key={value} value={value}>{t(`admin.role.${value}`)}</option>)}
          </Select>
        ) : null}
        {view !== 'none' ? (
          <Select aria-label={t('members.col.teams')} value={teamFilter} onChange={(event) => setParam('in', event.target.value || null)}>
            <option value="">{t('members.filter.team')}</option>
            {activeTeams(space.data?.teams ?? []).map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
          </Select>
        ) : null}
      </div>
      {selectable ? (
        <div className="batch-bar" role="group" aria-label={t('batch.assign')}>
          <label className="switch-line">
            <input
              type="checkbox"
              aria-label={t('batch.selectAll')}
              checked={selectableRows.length > 0 && selectableRows.every((person) => picked.has(person.userId))}
              onChange={(event) => setPicked(event.target.checked ? new Set(selectableRows.map((person) => person.userId)) : new Set())}
            />
            <span>{chosen.length > 0 ? t('batch.selected', { count: String(chosen.length) }) : t('batch.select')}</span>
          </label>
          {chosen.length > 0 ? (
            <>
              <Button type="button" onClick={() => setAssigning(true)}>{t('batch.assign')}</Button>
              <Button type="button" variant="secondary" onClick={() => setPicked(new Set())}>{t('batch.clear')}</Button>
            </>
          ) : null}
        </div>
      ) : null}
      {people.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {people.isError ? <p className="form-error" role="alert">{t('error.generic')}</p> : null}
      <PeopleList
        people={shown}
        workspaceId={workspaceId}
        adminColumns={manager}
        selection={selectable ? { picked, toggle: (id) => setPicked((current) => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next }) } : undefined}
      />
      <BatchAssignDialog
        open={assigning}
        workspaceId={workspaceId}
        people={chosen}
        teams={assignable}
        onClose={() => { setAssigning(false); if (finished) { setPicked(new Set()); setFinished(false) } }}
        onDone={() => setFinished(true)}
      />
    </div>
  )
}

type TeamTab = 'members' | 'projects' | 'work' | 'channels'

function TeamDetail({ teamId }: { teamId: string }) {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const session = useSession()
  const role = useWorkspaceRole(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const roster = useQuery(queries.adminTeamMembers(workspaceId, teamId))
  const projects = useQuery(queries.teamProjects(workspaceId, teamId))
  const [tab, setTab] = useState<TeamTab>('members')
  const team = space.data?.teams.find((item) => item.id === teamId)
  const archived = isArchivedTeam(team)
  const workspaceManager = isWorkspaceManager(role)
  const manages = (roster.data ?? []).some((member) => member.userId === session.data?.id && member.role === 'MANAGER')
  const canManage = workspaceManager || manages
  const channels = (space.data?.channels ?? []).filter((channel) => channel.teamId === teamId)
  if (space.isSuccess && !team) return <p className="hint">{t('notFound')}</p>
  const counted = (label: string, count: number | undefined) => (count === undefined ? label : `${label} ${count}`)
  return (
    <div className={archived ? 'page-scroll teams-page team-is-archived' : 'page-scroll teams-page'}>
      <header className="page-intro">
        <h1>
          {team?.name ?? t('loading')}
          {archived ? <> <Badge>{t('teams.archivedTag')}</Badge></> : null}
          {manages ? <> <Badge tone="accent">{t('teamPage.iManage')}</Badge></> : null}
        </h1>
        {team?.description ? <p className="team-description">{team.description}</p> : null}
        {roster.data && projects.data ? (
          <p className="hint team-summary">
            {t('teamPage.summary', { members: String(roster.data.length), projects: String(projects.data.length), channels: String(channels.length) })}
          </p>
        ) : null}
      </header>
      {archived && team ? (
        <section className="card settings-card team-archived-banner" role="status">
          <p>{t('teams.archived.banner', { date: formatStamp(team.archivedAt ?? '', locale) })}</p>
          {workspaceManager ? <RestoreTeamButton workspaceId={workspaceId} team={team} /> : null}
        </section>
      ) : null}
      <Tabs
        tabs={[
          { id: 'members', label: counted(t('teams.tab.members'), roster.data?.length) },
          { id: 'projects', label: counted(t('teams.tab.projects'), projects.data?.length) },
          { id: 'work', label: t('teamPage.tab.work') },
          { id: 'channels', label: counted(t('teamPage.tab.channels'), space.isSuccess ? channels.length : undefined) },
        ]}
        value={tab}
        onChange={(id) => setTab(id as TeamTab)}
      />
      {tab === 'members' ? <TeamMembersPanel workspaceId={workspaceId} teamId={teamId} archived={archived} canManage={canManage} showWorkspaceRole={workspaceManager} /> : null}
      {tab === 'projects' ? <TeamProjectsTab workspaceId={workspaceId} teamId={teamId} /> : null}
      {tab === 'work' ? <div className="team-work-tab"><Work target={{ kind: 'TEAM', workspaceId, teamId }} title={team?.name} /></div> : null}
      {tab === 'channels' ? <TeamChannelsTab workspaceId={workspaceId} channels={channels} archived={archived} /> : null}
      {tab === 'members' && canManage && team && !archived ? <TeamInfoForm key={team.id} workspaceId={workspaceId} team={team} /> : null}
      {tab === 'members' && canManage && team && !archived ? (
        <TeamArchiveSection workspaceId={workspaceId} team={team} canArchive={workspaceManager} channelCount={channels.length} memberCount={roster.data?.length ?? 0} />
      ) : null}
    </div>
  )
}

function PeopleList({
  people,
  workspaceId,
  selection,
  adminColumns = false,
}: {
  people: WorkspacePerson[]
  workspaceId: string
  selection?: { picked: Set<string>; toggle: (id: string) => void }
  /** Workspace role and state, shown to owners and admins only. */
  adminColumns?: boolean
}) {
  const { t } = useI18n()
  if (people.length === 0) return <p className="hint">{t('members.empty')}</p>
  return (
    <ul className="plain-list">
      {people.map((person) => (
        <li key={person.userId} className="person-line">
          {selection ? (
            <input
              type="checkbox"
              aria-label={`${t('batch.select')} ${person.displayName}`}
              disabled={person.state !== 'ACTIVE'}
              title={person.state !== 'ACTIVE' ? t('batch.notSelectable') : undefined}
              checked={selection.picked.has(person.userId)}
              onChange={() => selection.toggle(person.userId)}
            />
          ) : null}
          <PersonButton workspaceId={workspaceId} person={person} />
          {person.state === 'ACTIVE' ? <span className="hint">{person.teams.map((team) => team.name).join(', ') || t('teams.unaffiliated')}</span> : null}
          {adminColumns ? (
            <span className="hint person-admin">
              {t(`admin.role.${person.role as 'OWNER' | 'ADMIN' | 'MEMBER'}`)} · {t(`members.state.${person.state as 'ACTIVE' | 'INACTIVE'}`)}
            </span>
          ) : null}
          <AddToTeam workspaceId={workspaceId} person={person} />
        </li>
      ))}
    </ul>
  )
}

function AddToTeam({ workspaceId, person }: { workspaceId: string; person: WorkspacePerson }) {
  const { t } = useI18n()
  const role = useWorkspaceRole(workspaceId)
  const space = useQuery(queries.space(workspaceId))
  const queryClient = useQueryClient()
  const [teamId, setTeamId] = useState('')
  const teams = useMemo(() => activeTeams(space.data?.teams ?? []).filter((team) => !person.teams.some((item) => item.id === team.id)), [space.data, person.teams])
  if (!isWorkspaceManager(role) || person.state !== 'ACTIVE' || teams.length === 0) return null
  const picked = teams.some((team) => team.id === teamId) ? teamId : teams[0]?.id ?? ''
  return (
    <span className="row-actions">
      <Select aria-label={t('teams.addTo')} value={picked} onChange={(event) => setTeamId(event.target.value)}>
        {teams.map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
      </Select>
      <Button
        type="button"
        variant="secondary"
        onClick={() => {
          if (!picked) return
          void api.addTeamMember(workspaceId, picked, person.userId, 'MEMBER').then(async () => {
            await queryClient.invalidateQueries({ queryKey: queryKeys.people(workspaceId) })
            await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
          })
        }}
      >
        {t('teams.addTo')}
      </Button>
    </span>
  )
}

function FieldSearch({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="field">
      <span>{label}</span>
      <input value={value} onChange={(event) => onChange(event.target.value)} />
    </label>
  )
}
