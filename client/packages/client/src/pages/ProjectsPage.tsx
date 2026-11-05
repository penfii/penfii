import { DocsScreen } from '../modules/docs/DocsScreen'
import { Link, NavLink, Navigate, useParams, useSearchParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { EmptyState } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { shellProject, shellProjects, tasksFor } from '../preview/shellModel'
import { queries } from '../queries'
import { MemberStack } from '../modules/people/MemberStack'
import { useTicketPanel } from '../modules/work/panelParam'
import { Work } from '../modules/work/Work'
import { PlanningPage } from '../modules/work/PlanningPage'
import { FavoriteButton } from '../modules/personal/SavedButtons'
import { PinsPanel } from '../modules/personal/PinsPanel'
import { WatchButton } from '../modules/attention/WatchButton'
import { ProjectSettings, SettingsLink } from '../modules/attention/ProjectSettings'
import { StatusSummary } from '../modules/work/ReportPanels'
import { TimelinePage } from '../modules/work/Timeline'
import { panelOpen } from '../shell/DetailPanel'
import { ConversationFrame } from './SpacePage'

export function ProjectsPage() {
  const { workspaceId = '', projectId, view } = useParams()
  const [params] = useSearchParams()
  if (!projectId) {
    return <ProjectIndex workspaceId={workspaceId} mode={params.get('view')} />
  }
  // The tab that used to be named Board is Work now; old links keep working.
  if (view === 'board') return <Navigate to={`/w/${workspaceId}/projects/${projectId}/work${params.toString() ? `?${params.toString()}` : ''}`} replace />
  const screen = view === 'work' || view === 'planning' || view === 'settings' || view === 'timeline' || view === 'docs' || view === 'chat' ? view : 'overview'
  return <ProjectScreen workspaceId={workspaceId} projectId={projectId} view={screen} />
}

function ProjectIndex({ workspaceId, mode }: { workspaceId: string; mode: string | null }) {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const apiProjects = space.data?.projects ?? []
  if (mode === 'tasks') {
    const tasks = shellProjects.flatMap((project) => tasksFor(project.id))
    return (
      <div className="page-scroll">
        <header className="page-intro"><h1>{t('shell.projects.tasks')}</h1></header>
        {tasks.length === 0 ? <EmptyState title={t('shell.projects.tasks')} body={t('empty.tasks')} /> : null}
        <div className="result-list">
          {tasks.map((task) => (
            <Link key={task.id} className="result-hit" to={`/w/${workspaceId}/projects/${task.projectId}/work?panel=${panelOpen('task', task.id)}`}>
              <strong>{task.title}</strong>
              <small>{shellProject(task.projectId)?.name}</small>
            </Link>
          ))}
        </div>
      </div>
    )
  }
  const projects = mode === 'recent' ? [...shellProjects].reverse() : shellProjects
  if (projects.length === 0 && apiProjects.length === 0) {
    return (
      <div className="page-scroll">
        <header className="page-intro"><h1>{mode === 'recent' ? t('shell.projects.recent') : t('shell.projects.all')}</h1></header>
        {space.isSuccess ? <EmptyState title={t('tree.emptyProjects')} body={t('empty.projectsBody')} /> : <p className="hint">{t('loading')}</p>}
      </div>
    )
  }
  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{mode === 'recent' ? t('shell.projects.recent') : t('shell.projects.all')}</h1>
        <p>{t('projects.lead')}</p>
      </header>
      <div className="result-list">
        {projects.map((project) => (
          <Link key={project.id} className="result-hit" to={`/w/${workspaceId}/projects/${project.id}`}>
            <strong>{project.name}</strong>
            <small>{project.group === 'mine' ? t('shell.projects.mine') : t('shell.projects.team')}</small>
          </Link>
        ))}
        {apiProjects.map((project) => (
          <Link key={project.id} className="result-hit" to={`/w/${workspaceId}/projects/${project.id}`}>
            <strong>{project.name}</strong>
            <small>{project.participating === false ? t('projects.joinNeeded') : t('projects.participating')}</small>
          </Link>
        ))}
      </div>
    </div>
  )
}

export function ProjectChannelPage() {
  const { workspaceId = '', projectId = '' } = useParams()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  if (space.data?.projects.find((item) => item.id === projectId)?.participating === false) return <NotParticipating name={space.data.projects.find((item) => item.id === projectId)?.name} />
  return <ConversationFrame />
}

/** A Project is found by everyone in the Workspace but opened only by its participants: say so, with nothing to create. */
function NotParticipating({ name }: { name?: string }) {
  const { t } = useI18n()
  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{name ?? ''}</h1>
      </header>
      <EmptyState title={t('projects.notParticipating.title')} body={t('projects.notParticipating.body')} />
    </div>
  )
}

const PROJECT_TABS = [
  ['overview', 'shell.project.overview'],
  ['work', 'shell.project.work'],
  ['planning', 'shell.project.planning'],
  ['timeline', 'shell.project.timeline'],
  ['docs', 'shell.project.docs'],
  ['chat', 'shell.project.chat'],
] as const

/** The members, the settings button (for people who manage the Project) and the primary action (frame 2.1). */
function ProjectHeaderActions({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t } = useI18n()
  const members = useQuery(queries.projectMembers(projectId))
  const board = useQuery(queries.projectBoard(projectId))
  const { openCreate } = useTicketPanel()
  return (
    <div className="project-title-right">
      <MemberStack members={members.data ?? []} />
      <SettingsLink workspaceId={workspaceId} projectId={projectId} />
      {board.data?.canEdit ? (
        <button type="button" className="btn btn-primary create-ticket" onClick={() => openCreate({ projectId })}>
          <span aria-hidden="true">+</span> {t('work.createTicket')}
        </button>
      ) : null}
    </div>
  )
}

function ProjectScreen({ workspaceId, projectId, view }: { workspaceId: string; projectId: string; view: 'overview' | 'work' | 'planning' | 'settings' | 'timeline' | 'docs' | 'chat' }) {
  const { t } = useI18n()
  const sample = shellProject(projectId)
  const space = useQuery({ ...queries.space(workspaceId), enabled: !sample })
  const apiProject = space.data?.projects.find((item) => item.id === projectId)
  const name = sample?.name ?? apiProject?.name
  if (!sample && space.isSuccess && !apiProject) {
    return <EmptyState title={t('notFound')} body={t('projects.emptyBody')} />
  }
  // Until the Space says whether the person takes part, nothing of the inside is asked for.
  if (!sample && !apiProject) return <div className="page-scroll"><p className="hint">{t('loading')}</p></div>
  if (apiProject?.participating === false) return <NotParticipating name={name} />
  return (
    <div className={view === 'work' ? 'page-scroll board-page project-page' : 'page-scroll project-page'}>
      <header className="page-intro project-header">
        <div className="project-title-row">
          <div className="project-title-left">
            <h1>{name ?? t('loading')}</h1>
            {apiProject ? <FavoriteButton workspaceId={workspaceId} kind="PROJECT" targetId={projectId} label={name} /> : null}
            {apiProject ? <WatchButton kind="PROJECT" targetId={projectId} className="btn watch-chip" /> : null}
          </div>
          {apiProject ? <ProjectHeaderActions workspaceId={workspaceId} projectId={projectId} /> : null}
        </div>
        <nav className="project-tabs" aria-label={name ?? t('nav.projects')}>
          {PROJECT_TABS.map(([id, key]) => (
            <NavLink key={id} end={id === 'overview'} to={id === 'overview' ? `/w/${workspaceId}/projects/${projectId}` : `/w/${workspaceId}/projects/${projectId}/${id}`}>{t(key)}</NavLink>
          ))}
        </nav>
      </header>
      {view === 'overview' ? <Overview workspaceId={workspaceId} projectId={projectId} name={name ?? ''} /> : null}
      {view === 'work' ? <Work target={{ kind: 'PROJECT', workspaceId, projectId }} /> : null}
      {view === 'planning' ? <PlanningPage workspaceId={workspaceId} projectId={projectId} /> : null}
      {view === 'settings' ? <div className="project-pad"><ProjectSettings workspaceId={workspaceId} projectId={projectId} /></div> : null}
      {view === 'timeline' ? <div className="project-pad"><TimelinePage workspaceId={workspaceId} projectId={projectId} /></div> : null}
      {view === 'docs' ? <DocsScreen projectId={projectId} /> : null}
      {view === 'chat' ? <ProjectChat workspaceId={workspaceId} projectId={projectId} name={name ?? ''} /> : null}
    </div>
  )
}

function Overview({ workspaceId, projectId, name }: { workspaceId: string; projectId: string; name: string }) {
  const { t } = useI18n()
  const project = shellProject(projectId)
  const tasks = tasksFor(projectId)
  // A server Project shows what the server holds; the sample Projects keep their preview figures.
  const board = useQuery({ ...queries.projectBoard(projectId), enabled: !project })
  const members = useQuery({ ...queries.projectMembers(projectId), enabled: !project })
  const real = board.data ? board.data.columns.flatMap((column) => column.tickets) : []
  const done = real.filter((ticket) => board.data?.states.find((state) => state.id === ticket.statusId)?.category === 'DONE').length
  const progress = project ? `${project.progress}%` : real.length > 0 ? `${Math.round((done / real.length) * 100)}%` : '0%'
  const count = project ? tasks.length || 18 : real.length
  const memberCount = project ? project.members : (members.data?.length ?? 0)
  return (
    <>
      <div className="metric-row">
        <article className="metric">
          <span>{t('shell.overview.progress')}</span>
          <strong>{progress}</strong>
        </article>
        <article className="metric">
          <span>{t('shell.overview.tasks')}</span>
          <strong>{count}</strong>
        </article>
        <article className="metric">
          <span>{t('shell.overview.members')}</span>
          <strong>{memberCount}</strong>
        </article>
      </div>
      {/* Where the Project stands comes before the notes and pins, which change rarely. */}
      {project ? null : <StatusSummary projectId={projectId} />}
      {project ? (
        <section className="doc-sheet">
          <h2>{t('shell.overview.activity')}</h2>
          <ul>{project.activity.map((item) => <li key={item}>{item}</li>)}</ul>
          <h2>{t('shell.overview.upcoming')}</h2>
          <ul>{project.upcoming.map((item) => <li key={item}>{item}</li>)}</ul>
        </section>
      ) : (
        <section className="doc-sheet">
          <p>{t('projects.overviewBody', { name })}</p>
          <p><Link to={`/w/${workspaceId}/projects/${projectId}/work`}>{t('projects.openBoard')}</Link></p>
          <PinsPanel projectId={projectId} />
        </section>
      )}
    </>
  )
}

function ProjectChat({ workspaceId, projectId, name }: { workspaceId: string; projectId: string; name: string }) {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const channels = (space.data?.channels ?? []).filter((channel) => channel.projectId === projectId)
  const preferred = channels.find((channel) => channel.name === name) ?? channels[0]
  if (space.isSuccess && preferred) {
    return <Navigate to={`/w/${workspaceId}/projects/${projectId}/chat/${preferred.conversationId}`} replace />
  }
  if (!space.isSuccess) return <p>{t('loading')}</p>
  return <EmptyState title={t('shell.project.chat')} body={t('shell.project.noChannels')} />
}
