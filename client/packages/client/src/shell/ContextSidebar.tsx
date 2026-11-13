import { QueryError } from '../ui/QueryError'
import { useEffect,useState,type PointerEvent as ReactPointerEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useLocation,useParams } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { canCreateChannel } from '../modules/spaces/capabilities'
import { activeTeams } from '../modules/spaces/teamState'
import { TeamsNav } from '../pages/TeamsNav'
import { ChatNav } from './nav/ChatNav'
import { ProjectList,ProjectNav,projectViews } from './nav/ProjectNav'
import { DocsNav,HomeNav,InboxNav,SearchNav,SettingsNav } from './nav/SectionNavs'

export function ContextSidebar({
  workspaceId,
  section,
  navWidth,
  navMin = 160,
  navMax = 160,
  onResizePointerDown,
}: {
  workspaceId: string
  section: string
  navWidth?: number
  navMin?: number
  navMax?: number
  onResizePointerDown?: (event: ReactPointerEvent<HTMLElement>) => void
}) {
  const { t } = useI18n()
  const { projectId, conversationId } = useParams()
  const space = useQuery(queries.space(workspaceId))
  const workspaceName = useQuery(queries.workspaces()).data?.find((item) => item.id === workspaceId)?.name
  const projectMode = section === 'projects' && Boolean(projectId)
  // With a conversation open the list of conversations folds behind a button on a narrow screen, like a Project's sections.
  const chatMode = section === 'chat' && Boolean(conversationId)
  const collapsible = projectMode || chatMode
  const location = useLocation()
  // On a narrow screen the Project's sections collapse behind one button; a choice closes it again.
  const [menuOpen, setMenuOpen] = useState(false)
  useEffect(() => setMenuOpen(false), [location.pathname])
  const currentView = projectViews.find(([id]) => (id === 'overview' ? !/\/projects\/[^/]+\/[^/]+/.test(location.pathname) : location.pathname.includes(`/projects/${projectId ?? ''}/${id}`)))
  const openConversation = space.data?.channels.find((item) => item.conversationId === conversationId)
  const menuLabel = chatMode ? (openConversation ? `# ${openConversation.name}` : t('nav.chat')) : currentView ? t(currentView[1]) : t('nav.projects')
  const title = projectMode
    ? (space.data?.projects.find((item) => item.id === projectId)?.name ?? t('nav.projects'))
    : section === 'home'
      ? t('nav.home')
      : section === 'inbox'
      ? t('nav.inbox')
      : section === 'chat'
        ? t('nav.chat')
        : section === 'projects'
          ? t('nav.projects')
          : section === 'docs'
            ? t('nav.docs')
            : section === 'teams'
              ? t('nav.teams')
              : section === 'search'
                ? t('nav.search')
                : t('nav.settings')

  return (
    <aside id="workspace-nav" data-section={section} className={['workspace-nav', collapsible ? 'is-collapsible' : '', section === 'home' ? 'is-home' : '', menuOpen ? 'menu-open' : ''].filter(Boolean).join(' ')}>
      <div className="nav-head">
        <div className="nav-titles">
          {workspaceName ? <small className="nav-workspace" title={workspaceName}>{workspaceName}</small> : null}
          <h2>{title}</h2>
        </div>
        {collapsible ? (
          <button
            type="button"
            className="nav-menu-toggle"
            aria-expanded={menuOpen}
            aria-controls="nav-collapse-body"
            aria-label={`${chatMode ? t('shell.chat.menu') : t('shell.project.menu')}: ${menuLabel}`}
            onClick={() => setMenuOpen((open) => !open)}
          >
            {menuLabel} <span aria-hidden="true">{menuOpen ? '▴' : '▾'}</span>
          </button>
        ) : null}
      </div>
      {space.isError && (section === 'chat' || section === 'projects') ? <QueryError query={space} compact /> : null}
      {section === 'home' ? <HomeNav workspaceId={workspaceId} /> : null}
      {section === 'inbox' ? <InboxNav workspaceId={workspaceId} /> : null}
      {section === 'chat' ? (
        <div className="nav-collapse-body" id="nav-collapse-body">
          <ChatNav workspaceId={workspaceId} space={space.data} />
        </div>
      ) : null}
      {section === 'projects' && projectId ? (
        <ProjectNav
          workspaceId={workspaceId}
          projectId={projectId}
          channels={space.data?.channels ?? []}
          canCreate={canCreateChannel(undefined, space.data?.projects.find((project) => project.id === projectId))}
          participating={space.data?.projects.find((project) => project.id === projectId)?.participating !== false}
        />
      ) : null}
      {section === 'projects' && !projectId ? <ProjectList workspaceId={workspaceId} apiProjects={space.data?.projects ?? []} teams={activeTeams(space.data?.teams ?? [])} /> : null}
      {section === 'docs' ? <DocsNav workspaceId={workspaceId} /> : null}
      {section === 'teams' ? <TeamsNav workspaceId={workspaceId} /> : null}
      {section === 'search' ? <SearchNav workspaceId={workspaceId} /> : null}
      {section === 'settings' || section === 'account' ? <SettingsNav workspaceId={workspaceId} /> : null}
      {onResizePointerDown ? (
        <div
          className="resize-handle end"
          role="separator"
          aria-orientation="vertical"
          aria-label={t('layout.resizeContext')}
          title={t('layout.resetWidth')}
          aria-valuemin={navMin}
          aria-valuemax={navMax}
          aria-valuenow={navWidth}
          onPointerDown={onResizePointerDown}
        />
      ) : null}
    </aside>
  )
}

