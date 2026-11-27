import { DocTree } from '../../modules/docs/DocTree'
import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { Channel, Project } from '@penfii/contracts'
import { Link, NavLink, useLocation, useNavigate, useSearchParams } from 'react-router'
import {
visibleSlice
} from '../chatGroups'
import { useI18n } from '../../i18n/I18nProvider'
import { canManageChannelSettings, isWorkspaceManager } from '../../modules/spaces/capabilities'
import { useWorkspaceRole } from '../../modules/spaces/useWorkspaceRole'
import { useCreate } from '../create'
import { ChannelLink } from './ChatNav'
import { FavoriteButton } from '../../modules/personal/SavedButtons'
import { queries } from '../../queries'
import { myProjects } from '../../pages/projectsModel'

export const projectViews = [
  ['overview', 'shell.project.overview'],
  ['work', 'shell.project.work'],
  ['planning', 'shell.project.planning'],
  ['timeline', 'shell.project.timeline'],
  ['docs', 'shell.project.docs'],
  ['chat', 'shell.project.chat'],
] as const

/** How many participating Projects the list shows before "all"; every favorite is always shown. A density choice, not a limit on what can be reached. */
const NAV_PROJECT_LIMIT = 8

/**
 * The Projects Context Nav, outside any one Project: shortcuts first (My work, All projects), then the Projects the
 * person takes part in with the starred ones first. Projects the person is not in, and Teams, are reached from All
 * projects and from the global Teams.
 */
export function ProjectList({ workspaceId, apiProjects }: { workspaceId: string; apiProjects: Project[] }) {
  const { t } = useI18n()
  const open = useCreate()
  const location = useLocation()
  const [params] = useSearchParams()
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  // Creating a Project is for a Workspace owner or admin.
  const canCreate = isWorkspaceManager(useWorkspaceRole(workspaceId))
  const mine = myProjects(apiProjects, favorites.data ?? [])
  const total = mine.favorites.length + mine.others.length
  // Every favorite is listed; the other Projects fill the rest of the usual length.
  const room = Math.max(NAV_PROJECT_LIMIT - mine.favorites.length, 0)
  const hidden = mine.others.length - Math.min(room, mine.others.length)
  const base = `/w/${workspaceId}/projects`
  const atProjects = location.pathname === base
  const browsing = params.get('browse') === 'all'
  return (
    <>
      {canCreate ? <button type="button" className="nav-add" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
      <section className="tree-block" aria-label={t('projects.nav.shortcuts')}>
        <ul>
          <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
          <li>
            <Link className={atProjects && browsing ? 'tree-link active' : 'tree-link'} aria-current={atProjects && browsing ? 'page' : undefined} to={`${base}?browse=all`}>{t('shell.projects.all')}</Link>
          </li>
        </ul>
      </section>
      <section className="tree-block">
        <h3>
          <Link className={atProjects && !browsing ? 'tree-heading-link active' : 'tree-heading-link'} aria-current={atProjects && !browsing ? 'page' : undefined} to={base}>
            {t('shell.projects.mine')} <span className="list-count">{total}</span>
          </Link>
        </h3>
        {total === 0 ? <p className="tree-empty">{apiProjects.length === 0 ? t('tree.emptyProjects') : t('projects.nav.empty')}</p> : null}
        <ul>
          {mine.favorites.map((project) => (
            <li key={project.id}>
              <NavLink className="tree-link" to={`${base}/${project.id}`}>
                <span className="tree-star" role="img" aria-label={t('projects.favorite')}>★</span>
                {project.name}
              </NavLink>
            </li>
          ))}
          {mine.others.slice(0, room).map((project) => (
            <li key={project.id}>
              <NavLink className="tree-link" to={`${base}/${project.id}`}>{project.name}</NavLink>
            </li>
          ))}
        </ul>
        {hidden > 0 ? <Link className="tree-more" to={base}>{t('projects.nav.more', { count: String(total) })}</Link> : null}
      </section>
    </>
  )
}

export function ProjectNav({
  workspaceId,
  projectId,
  channels,
  canCreate,
  projectCanManage,
  participating,
}: {
  workspaceId: string
  projectId: string
  channels: Channel[]
  canCreate: boolean
  projectCanManage: boolean
  participating: boolean
}) {
  const { t } = useI18n()
  const open = useCreate()
  const navigate = useNavigate()
  const workspaceRole = useWorkspaceRole(workspaceId)
  const location = useLocation()
  const [params] = useSearchParams()
  const docsHere = location.pathname.includes(`/projects/${projectId}/docs`)
  const chatHere = location.pathname.includes(`/projects/${projectId}/chat`)
  const [docsOpen, setDocsOpen] = useState(docsHere)
  const [chatOpen, setChatOpen] = useState(chatHere)
  const [channelsExpanded, setChannelsExpanded] = useState(false)
  useEffect(() => { if (docsHere) setDocsOpen(true) }, [docsHere])
  useEffect(() => { if (chatHere) setChatOpen(true) }, [chatHere])
  const projectChannels = channels.filter((channel) => channel.projectId === projectId)
  const unread = projectChannels.reduce((sum, channel) => sum + channel.unreadCount, 0)
  // A Project the person does not take part in has no inside to navigate: only the way back to the list.
  if (!participating) return <NavLink className="tree-link" to={`/w/${workspaceId}/projects`}>{t('nav.projects')}</NavLink>
  const docId = params.get('doc')
  return (
    <div className="nav-collapse-body" id="nav-collapse-body">
      <NavLink className="tree-link nav-back" to={`/w/${workspaceId}/projects`}>{t('nav.projects')}</NavLink>
      <ul>
        {projectViews.map(([id, label]) => {
          const text = t(label)
          const to = id === 'overview' ? `/w/${workspaceId}/projects/${projectId}` : `/w/${workspaceId}/projects/${projectId}/${id}`
          if (id === 'docs' || id === 'chat') {
            const expanded = id === 'docs' ? docsOpen : chatOpen
            return (
              <li key={id} className="nav-branch">
                <div className="nav-branch-row">
                  <button
                    type="button"
                    className="nav-twist"
                    aria-expanded={expanded}
                    aria-label={t(expanded ? 'shell.project.collapse' : 'shell.project.expand', { name: text })}
                    onClick={() => (id === 'docs' ? setDocsOpen((value) => !value) : setChatOpen((value) => !value))}
                  >
                    {expanded ? '▾' : '▸'}
                  </button>
                  <NavLink className="tree-link" to={to}>{text}</NavLink>
                  {id === 'chat' && !expanded && unread > 0 ? <span className="unread-count">{unread}</span> : null}
                  {id === 'chat' && canCreate ? <button type="button" className="icon-add" aria-label={t('shell.channel.create')} onClick={() => open('channel', projectId)}>+</button> : null}
                </div>
                {id === 'docs' && expanded ? (
                  <div className="nav-doc-tree">
                    <DocTree
                      workspaceId={workspaceId}
                      projectId={projectId}
                      selectedId={docsHere && docId !== 'new' ? docId : null}
                      onSelect={(pageId, options) => {
                        const next = new URLSearchParams()
                        if (pageId) next.set('doc', pageId)
                        if (pageId && options?.edit) next.set('edit', '1')
                        const query = next.toString()
                        void navigate(`/w/${workspaceId}/projects/${projectId}/docs${query ? `?${query}` : ''}`)
                      }}
                      onDraft={(parentId) => {
                        const next = new URLSearchParams({ draft: '1' })
                        if (parentId) next.set('parent', parentId)
                        void navigate(`/w/${workspaceId}/projects/${projectId}/docs?${next.toString()}`)
                      }}
                    />
                  </div>
                ) : null}
                {id === 'chat' && expanded ? (
                  <>
                    {projectChannels.length === 0 ? <p className="tree-empty">{t('shell.project.noChannels')}</p> : null}
                    <ul>
                      {visibleSlice(projectChannels, channelsExpanded, (channel) => channel.unreadCount > 0).shown.map((channel) => (
                        <li key={channel.id}>
                          <ChannelLink workspaceId={workspaceId} channel={channel} to={`/w/${workspaceId}/projects/${projectId}/chat/${channel.conversationId}`} canManageSettings={canManageChannelSettings(channel, workspaceRole, { canManage: projectCanManage })} />
                          <FavoriteButton workspaceId={workspaceId} kind="CHANNEL" targetId={channel.id} label={`# ${channel.name}`} className="icon-add tree-favorite" />
                        </li>
                      ))}
                    </ul>
                    {projectChannels.length > 10 ? (
                      <button type="button" className="tree-more" onClick={() => setChannelsExpanded((value) => !value)}>
                        {channelsExpanded ? t('shell.chat.less') : t('shell.chat.more')}
                      </button>
                    ) : null}
                  </>
                ) : null}
              </li>
            )
          }
          return (
            <li key={id}>
              <NavLink className="tree-link" end={id === 'overview'} to={to}>{text}</NavLink>
            </li>
          )
        })}
      </ul>
    </div>
  )
}

