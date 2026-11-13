import { DocTree } from '../../modules/docs/DocTree'
import { useEffect,useState } from 'react'
import type { Channel } from '@penfii/contracts'
import { NavLink,useLocation,useNavigate,useSearchParams } from 'react-router'
import {
visibleSlice
} from '../chatGroups'
import { useI18n } from '../../i18n/I18nProvider'
import { isWorkspaceManager } from '../../modules/spaces/capabilities'
import { useWorkspaceRole } from '../../modules/spaces/useWorkspaceRole'
import { useCreate } from '../create'
import { ChannelLink } from './ChatNav'

export const projectViews = [
  ['overview', 'shell.project.overview'],
  ['work', 'shell.project.work'],
  ['planning', 'shell.project.planning'],
  ['timeline', 'shell.project.timeline'],
  ['docs', 'shell.project.docs'],
  ['chat', 'shell.project.chat'],
] as const


export function ProjectList({ workspaceId, apiProjects, teams }: { workspaceId: string; apiProjects: Array<{ id: string; name: string; participating?: boolean }>; teams: Array<{ id: string; name: string }> }) {
  const { t } = useI18n()
  const open = useCreate()
  // Creating a Team or a Project is for a Workspace owner or admin.
  const canCreate = isWorkspaceManager(useWorkspaceRole(workspaceId))
  return (
    <>
      {canCreate ? <button type="button" className="nav-add" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
      <section className="tree-block">
        <h3>{t('shell.projects.mine')}</h3>
        {apiProjects.length === 0 ? <p className="tree-empty">{t('tree.emptyProjects')}</p> : null}
        <ul>
          {apiProjects.filter((project) => project.participating !== false).map((project) => (
            <li key={project.id}>
              <NavLink className="tree-link" to={`/w/${workspaceId}/projects/${project.id}`}>{project.name}</NavLink>
            </li>
          ))}
        </ul>
      </section>
      {apiProjects.some((project) => project.participating === false) ? (
        <section className="tree-block">
          <h3>{t('shell.projects.others')}</h3>
          <ul>
            {apiProjects.filter((project) => project.participating === false).map((project) => (
              <li key={project.id}>
                <NavLink className="tree-link" to={`/w/${workspaceId}/projects/${project.id}`}>{project.name}</NavLink>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <section className="tree-block">
        <div className="section-title">
          <h3>{t('shell.teams')}</h3>
          {canCreate ? <button type="button" className="icon-add" aria-label={t('create.team')} onClick={() => open('team')}>+</button> : null}
        </div>
        <ul>
          {teams.map((team) => (
            <li key={team.id}>
              <NavLink className="tree-link" to={`/w/${workspaceId}/teams/${team.id}`}>{team.name}</NavLink>
            </li>
          ))}
        </ul>
      </section>
      <section className="tree-block">
        <h3>{t('shell.projects.views')}</h3>
        <ul>
          <li><NavLink end className={({ isActive }) => (isActive ? 'tree-link active' : 'tree-link')} to={`/w/${workspaceId}/projects`}>{t('shell.projects.all')}</NavLink></li>
          <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
        </ul>
      </section>
    </>
  )
}

export function ProjectNav({
  workspaceId,
  projectId,
  channels,
  canCreate,
  participating,
}: {
  workspaceId: string
  projectId: string
  channels: Channel[]
  canCreate: boolean
  participating: boolean
}) {
  const { t } = useI18n()
  const open = useCreate()
  const navigate = useNavigate()
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
                          <ChannelLink workspaceId={workspaceId} channel={channel} to={`/w/${workspaceId}/projects/${projectId}/chat/${channel.conversationId}`} />
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

