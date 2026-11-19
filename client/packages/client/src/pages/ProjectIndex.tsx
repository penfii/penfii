import { Link, useSearchParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { FavoriteItem, Project } from '@penfii/contracts'
import { EmptyState } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { useCreate } from '../shell/create'
import { isWorkspaceManager } from '../modules/spaces/capabilities'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { QueryError } from '../ui/QueryError'
import { browseProjects, joinFilter, myProjects, takesPart, type JoinFilter } from './projectsModel'

/**
 * The Projects entry. By default it is "My projects": the ones the person takes part in, starred ones first. Every
 * Project of the Workspace is still found, by search and filter, in "All projects" (`?browse=all`), which keeps the
 * join state apart from the right to see a Project at all.
 */
export function ProjectIndex({ workspaceId }: { workspaceId: string }) {
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  const [params] = useSearchParams()
  const { t } = useI18n()
  if (space.isError) return <div className="page-scroll"><QueryError query={space} /></div>
  if (!space.data) return <div className="page-scroll"><p className="hint">{t('loading')}</p></div>
  const projects = space.data.projects
  return params.get('browse') === 'all' ? (
    <AllProjects workspaceId={workspaceId} projects={projects} />
  ) : (
    <MyProjects workspaceId={workspaceId} projects={projects} favorites={favorites.data ?? []} />
  )
}

function ProjectCard({ workspaceId, project, starred }: { workspaceId: string; project: Project; starred: boolean }) {
  const { t } = useI18n()
  return (
    <li>
      <Link className="project-card" to={`/w/${workspaceId}/projects/${project.id}`}>
        <strong>
          {starred ? <span className="project-card-star" role="img" aria-label={t('projects.favorite')}>★</span> : null}
          {project.name}
        </strong>
        {project.ticketPrefix ? <small>{project.ticketPrefix}</small> : null}
      </Link>
    </li>
  )
}

function MyProjects({ workspaceId, projects, favorites }: { workspaceId: string; projects: Project[]; favorites: FavoriteItem[] }) {
  const { t } = useI18n()
  const open = useCreate()
  const canCreate = isWorkspaceManager(useWorkspaceRole(workspaceId))
  const mine = myProjects(projects, favorites)
  const count = mine.favorites.length + mine.others.length
  return (
    <div className="page-scroll">
      <header className="page-intro project-index-head">
        <div>
          <h1>{t('shell.projects.mine')}</h1>
          <p>{t('projects.mine.lead')}</p>
        </div>
        <div className="project-index-actions">
          <Link className="btn btn-secondary" to={`/w/${workspaceId}/projects?browse=all`}>{t('projects.browseAll', { count: String(projects.length) })}</Link>
          {canCreate ? <button type="button" className="btn btn-primary" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
        </div>
      </header>
      {count === 0 ? (
        <EmptyState title={t('projects.mine.emptyTitle')} body={projects.length === 0 ? t('empty.projectsBody') : t('projects.mine.emptyBody', { count: String(projects.length) })}>
          {projects.length > 0 ? <Link className="btn btn-primary" to={`/w/${workspaceId}/projects?browse=all`}>{t('projects.browseAllShort')}</Link> : null}
          {canCreate ? <button type="button" className="btn btn-secondary" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
        </EmptyState>
      ) : (
        <>
          {mine.favorites.length > 0 ? (
            <section aria-labelledby="mine-favorites">
              <h2 id="mine-favorites" className="project-group-title">{t('projects.favorites')} <span className="list-count">{mine.favorites.length}</span></h2>
              <ul className="project-grid">
                {mine.favorites.map((project) => <ProjectCard key={project.id} workspaceId={workspaceId} project={project} starred />)}
              </ul>
            </section>
          ) : null}
          {mine.others.length > 0 ? (
            <section aria-labelledby="mine-others">
              <h2 id="mine-others" className="project-group-title">{mine.favorites.length > 0 ? t('projects.otherMine') : t('projects.participatingList')} <span className="list-count">{mine.others.length}</span></h2>
              <ul className="project-grid">
                {mine.others.map((project) => <ProjectCard key={project.id} workspaceId={workspaceId} project={project} starred={false} />)}
              </ul>
            </section>
          ) : null}
        </>
      )}
    </div>
  )
}

const FILTERS: JoinFilter[] = ['all', 'in', 'out']

function AllProjects({ workspaceId, projects }: { workspaceId: string; projects: Project[] }) {
  const { t } = useI18n()
  const [params, setParams] = useSearchParams()
  const query = params.get('q') ?? ''
  const join = joinFilter(params.get('join'))
  const open = useCreate()
  const canCreate = isWorkspaceManager(useWorkspaceRole(workspaceId))
  const shown = browseProjects(projects, join, query)
  const counts: Record<JoinFilter, number> = {
    all: browseProjects(projects, 'all', query).length,
    in: browseProjects(projects, 'in', query).length,
    out: browseProjects(projects, 'out', query).length,
  }
  // Searching and filtering are this person's own way of looking: they live in the address (replace), never on the server.
  const update = (patch: { q?: string; join?: JoinFilter }) =>
    setParams((current) => {
      const next = new URLSearchParams(current)
      next.set('browse', 'all')
      const q = patch.q ?? query
      const nextJoin = patch.join ?? join
      if (q) next.set('q', q)
      else next.delete('q')
      if (nextJoin === 'all') next.delete('join')
      else next.set('join', nextJoin)
      return next
    }, { replace: true })
  return (
    <div className="page-scroll">
      <header className="page-intro project-index-head">
        <div>
          <h1>{t('shell.projects.all')}</h1>
          <p>{t('projects.all.lead')}</p>
        </div>
        <div className="project-index-actions">
          <Link className="btn btn-secondary" to={`/w/${workspaceId}/projects`}>{t('projects.backToMine')}</Link>
          {canCreate ? <button type="button" className="btn btn-primary" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
        </div>
      </header>
      <div className="project-browse-tools">
        <input type="search" className="view-search" aria-label={t('projects.search')} placeholder={t('projects.search')} value={query} onChange={(event) => update({ q: event.target.value })} />
        <div className="view-layouts" role="group" aria-label={t('projects.joinFilter')}>
          {FILTERS.map((item) => (
            <button key={item} type="button" className={item === join ? 'view-layout on' : 'view-layout'} aria-pressed={item === join} onClick={() => update({ join: item })}>
              {t(`projects.join.${item}`)} <span className="list-count">{counts[item]}</span>
            </button>
          ))}
        </div>
      </div>
      <p className="hint" role="status">{t('projects.shownCount', { shown: String(shown.length), total: String(projects.length) })}</p>
      {projects.length === 0 ? (
        <EmptyState title={t('tree.emptyProjects')} body={t('empty.projectsBody')} />
      ) : shown.length === 0 ? (
        <EmptyState title={t('projects.noMatch')} body={t('projects.noMatchBody')}>
          <button type="button" className="btn btn-secondary" onClick={() => setParams({ browse: 'all' }, { replace: true })}>{t('projects.clearFilters')}</button>
        </EmptyState>
      ) : (
        <ul className="result-list project-rows">
          {shown.map((project) => (
            <li key={project.id}>
              <Link className="result-hit" to={`/w/${workspaceId}/projects/${project.id}`}>
                <strong>{project.name}</strong>
                <small>
                  {project.ticketPrefix ? `${project.ticketPrefix} · ` : ''}
                  {takesPart(project) ? t('projects.participating') : t('projects.joinNeeded')}
                </small>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
