import { settingsGroups } from './settingsEntries'
import { DocTree } from '../modules/docs/DocTree'
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { Channel, Dm, Space } from '@penfii/contracts'
import { Link, NavLink, useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import { Avatar, Button, Dialog, useDismiss } from '../design/ui'
import {
  createSection,
  deleteSection,
  loadSections,
  moveConversation,
  placedIds,
  pruneSections,
  renameSection,
  reorderSections,
  saveSections,
  visibleSlice,
  type ChatSection,
  type ChatSections,
} from './chatGroups'
import { formatMessageTime } from '../format'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { shellProjects } from '../preview/shellModel'
import { SEARCH_TABS } from '../modules/search/resultHref'
import { canCreateChannel, isWorkspaceManager } from '../modules/spaces/capabilities'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'
import { activeTeams } from '../modules/spaces/teamState'
import { TeamsNav } from '../pages/TeamsPage'
import { useCreate } from './create'

const inboxBoxes = [
  ['all', 'inbox.filter.all'],
  ['mentions', 'inbox.filter.mentions'],
  ['replies', 'inbox.filter.replies'],
  ['watching', 'inbox.filter.watching'],
  ['reminders', 'inbox.filter.reminders'],
  ['invitations', 'inbox.filter.invitations'],
  ['assignments', 'inbox.filter.assignments'],
  ['unread', 'shell.inbox.unread'],
] as const

const projectViews = [
  ['overview', 'shell.project.overview'],
  ['work', 'shell.project.work'],
  ['planning', 'shell.project.planning'],
  ['timeline', 'shell.project.timeline'],
  ['docs', 'shell.project.docs'],
  ['chat', 'shell.project.chat'],
] as const

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
    ? (shellProjects.find((item) => item.id === projectId)?.name ?? space.data?.projects.find((item) => item.id === projectId)?.name ?? t('nav.projects'))
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

function SettingsNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const role = useWorkspaceRole(workspaceId)
  return (
    <>
      {settingsGroups(isWorkspaceManager(role)).map((group) => (
        <section key={group.id} className="tree-block">
          <h3>{t(group.heading)}</h3>
          <ul>
            {group.entries.map((entry) => (
              <li key={entry.id}><NavLink end={entry.path === ''} className="tree-link" to={`/w/${workspaceId}/settings${entry.path}`}>{t(entry.label)}</NavLink></li>
            ))}
          </ul>
        </section>
      ))}
    </>
  )
}

function HomeNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  return (
    <ul>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/inbox`}>{t('nav.inbox')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/saved`}>{t('shell.saved')}</NavLink></li>
      <li><NavLink className="tree-link" to={`/w/${workspaceId}/drafts`}>{t('shell.drafts')}</NavLink></li>
    </ul>
  )
}

function InboxNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const current = params.get('box') ?? 'all'
  return (
    <>
    <ul>
      {inboxBoxes.map(([id, label]) => (
        <li key={id}>
          <Link className={current === id ? 'tree-link active' : 'tree-link'} aria-current={current === id ? 'page' : undefined} to={`/w/${workspaceId}/inbox?box=${id}`}>
            {t(label)}
          </Link>
        </li>
      ))}
    </ul>
    <section className="tree-block">
      <h3>{t('shell.mySpace')}</h3>
      <ul>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/saved`}>{t('shell.saved')}</NavLink></li>
        <li><NavLink className="tree-link" to={`/w/${workspaceId}/drafts`}>{t('shell.drafts')}</NavLink></li>
      </ul>
    </section>
    </>
  )
}

function historyDms(space: Space | undefined): Dm[] {
  return (space?.dms ?? []).filter((dm) => dm.lastMessageSeq == null || dm.lastMessageSeq > 0)
}

type DragKind = { kind: 'conversation' | 'section'; id: string }
let dragging: DragKind | null = null

function Fold({
  id,
  title,
  badge = 0,
  menu,
  dropOn = false,
  draggable = false,
  onHeaderDragOver,
  onHeaderDrop,
  children,
}: {
  id: string
  title: string
  badge?: number
  menu?: ReactNode
  dropOn?: boolean
  draggable?: boolean
  onHeaderDragOver?: () => void
  onHeaderDrop?: () => void
  children: ReactNode
}) {
  const key = `penfii.chat.fold.${id}`
  const [open, setOpen] = useState(() => localStorage.getItem(key) !== '0')
  const [peek, setPeek] = useState(false)
  const peekTimer = useRef<number | null>(null)
  const shown = open || peek
  // A group opened by hovering closes again when the drag ends, wherever it ended.
  useEffect(() => {
    if (!peek) return undefined
    const done = () => {
      if (peekTimer.current != null) window.clearTimeout(peekTimer.current)
      peekTimer.current = null
      setPeek(false)
    }
    document.addEventListener('dragend', done)
    document.addEventListener('drop', done)
    return () => {
      document.removeEventListener('dragend', done)
      document.removeEventListener('drop', done)
    }
  }, [peek])
  function armPeek() {
    if (open || peekTimer.current != null) return
    peekTimer.current = window.setTimeout(() => setPeek(true), 600)
  }
  function clearPeek() {
    if (peekTimer.current != null) window.clearTimeout(peekTimer.current)
    peekTimer.current = null
    setPeek(false)
  }
  return (
    <section
      className={dropOn ? 'tree-block drop-on' : 'tree-block'}
      onDragLeave={(event) => {
        // Browsers often leave relatedTarget empty during a drag, so judge by where the pointer is.
        const box = event.currentTarget.getBoundingClientRect()
        const inside = event.clientX > box.left && event.clientX < box.right && event.clientY > box.top && event.clientY < box.bottom
        if (!inside) clearPeek()
      }}
    >
      <div className="tree-fold-row">
        <button
          type="button"
          className="tree-fold"
          draggable={draggable}
          aria-expanded={shown}
          onClick={() => {
            const next = !open
            localStorage.setItem(key, next ? '1' : '0')
            setOpen(next)
            setPeek(false)
          }}
          onDragStart={(event) => {
            if (!draggable) return
            dragging = { kind: 'section', id }
            event.dataTransfer.effectAllowed = 'move'
            event.dataTransfer.setData('text/plain', id)
          }}
          onDragOver={(event) => {
            if (!dragging) return
            if (dragging.kind === 'section' && !draggable) return
            event.preventDefault()
            if (dragging.kind === 'conversation') {
              onHeaderDragOver?.()
              armPeek()
            }
          }}
          onDrop={(event) => {
            if (!dragging) return
            if (dragging.kind === 'section' && !draggable) return
            event.preventDefault()
            clearPeek()
            onHeaderDrop?.()
          }}
        >
          <span aria-hidden="true">{shown ? '▾' : '▸'}</span>
          <h3>{title}</h3>
          {!shown && badge > 0 ? <span className="unread-count">{badge}</span> : null}
        </button>
        {menu}
      </div>
      {shown ? children : null}
    </section>
  )
}

function ChatNav({ workspaceId, space }: { workspaceId: string; space: Space | undefined }) {
  const { t, locale } = useI18n()
  const channels = (space?.channels ?? []).filter((channel) => !channel.projectId)
  // A Project's channels are the same conversations the Project screen shows; Chat lists them too,
  // under the Project they belong to, so no chat history is reachable from only one of the two.
  const projectChannels = (space?.channels ?? []).filter((channel) => channel.projectId)
  const projectName = (id: string | null) => space?.projects.find((project) => project.id === id)?.name ?? ''
  const dms = historyDms(space)
  const [layout, setLayout] = useState<ChatSections>(() => loadSections(workspaceId))
  const [naming, setNaming] = useState(false)
  const [draftName, setDraftName] = useState('')
  const [dropOn, setDropOn] = useState<string | null>(null)
  const [dropIndex, setDropIndex] = useState<{ sectionId: string; index: number } | null>(null)
  const [expandedLists, setExpandedLists] = useState<Record<string, boolean>>({})
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameText, setRenameText] = useState('')
  const [removing, setRemoving] = useState<ChatSection | null>(null)
  const liveIds = useMemoIds(channels, projectChannels, dms)
  useEffect(() => {
    setLayout(pruneSections(loadSections(workspaceId), liveIds))
  }, [workspaceId, liveIds])
  function commit(next: ChatSections) {
    const pruned = pruneSections(next, liveIds)
    setLayout(pruned)
    saveSections(workspaceId, pruned)
  }
  // A drag that ends anywhere, dropped or cancelled, clears the drop markers.
  useEffect(() => {
    const clear = () => {
      setDropOn(null)
      setDropIndex(null)
    }
    document.addEventListener('dragend', clear)
    return () => document.removeEventListener('dragend', clear)
  }, [])
  const taken = placedIds(layout)
  const freeChannels = channels.filter((channel) => !taken.has(channel.conversationId))
  const freeProjects = projectChannels.filter((channel) => !taken.has(channel.conversationId))
  const freeDms = dms.filter((dm) => !taken.has(dm.conversationId))
  function entry(id: string): { kind: 'channel'; channel: Channel } | { kind: 'dm'; dm: Dm } | null {
    const channel = channels.find((item) => item.conversationId === id) ?? projectChannels.find((item) => item.conversationId === id)
    if (channel) return { kind: 'channel', channel }
    const dm = dms.find((item) => item.conversationId === id)
    return dm ? { kind: 'dm', dm } : null
  }
  function place(conversationId: string, sectionId: string | null, index?: number) {
    commit(moveConversation(layout, conversationId, sectionId, index))
    setDropOn(null)
    setDropIndex(null)
    dragging = null
  }
  return (
    <>
      <NewConversationMenu workspaceId={workspaceId} onNewSection={() => { setNaming(true); setDraftName('') }} />
      {naming ? (
        <form
          className="section-name"
          onSubmit={(event) => {
            event.preventDefault()
            commit(createSection(layout, draftName))
            setNaming(false)
            setDraftName('')
          }}
        >
          <input
            aria-label={t('shell.chat.sectionName')}
            value={draftName}
            autoFocus
            onChange={(event) => setDraftName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape') setNaming(false)
            }}
          />
        </form>
      ) : null}
      <Fold id={`${workspaceId}:favorites`} title={t('shell.chat.favorites')}>
        <p className="tree-empty">{t('shell.empty.favorites')}</p>
      </Fold>
      {layout.sections.map((section) => {
        const rows = section.conversationIds.map((id) => entry(id)).filter((item): item is NonNullable<typeof item> => item != null)
        const badge = rows.reduce((sum, row) => sum + (row.kind === 'channel' ? row.channel.unreadCount : row.dm.unreadCount), 0)
        return (
          <Fold
            key={section.id}
            id={`${workspaceId}:section:${section.id}`}
            title={section.name}
            badge={badge}
            draggable
            dropOn={dropOn === section.id}
            onHeaderDragOver={() => setDropOn(section.id)}
            onHeaderDrop={() => {
              if (dragging?.kind === 'section') {
                commit(reorderSections(layout, dragging.id, section.id))
                dragging = null
                return
              }
              if (dragging?.kind === 'conversation') place(dragging.id, section.id)
            }}
            menu={<SectionMenu onRename={() => { setRenaming(section.id); setRenameText(section.name) }} onDelete={() => setRemoving(section)} />}
          >
            {renaming === section.id ? (
              <form
                className="section-name"
                onSubmit={(event) => {
                  event.preventDefault()
                  commit(renameSection(layout, section.id, renameText))
                  setRenaming(null)
                }}
              >
                <input aria-label={t('shell.chat.renameSection')} value={renameText} autoFocus onChange={(event) => setRenameText(event.target.value)} />
              </form>
            ) : null}
            <ul className="conv-list">
              {rows.map((row, index) => {
                const id = row.kind === 'channel' ? row.channel.conversationId : row.dm.conversationId
                return (
                  <ConversationSlot
                    key={id}
                    conversationId={id}
                    marked={dropIndex?.sectionId === section.id && dropIndex.index === index}
                    onDragOver={() => setDropIndex({ sectionId: section.id, index })}
                    onDrop={() => { if (dragging?.kind === 'conversation') place(dragging.id, section.id, index) }}
                  >
                    {row.kind === 'channel' ? (
                      <ChannelLink workspaceId={workspaceId} channel={row.channel} context={row.channel.projectId ? projectName(row.channel.projectId) : undefined} />
                    ) : (
                      <DmLink workspaceId={workspaceId} dm={row.dm} locale={locale} fallback={t('dm.short')} />
                    )}
                    <MoveMenu sections={layout.sections} current={section.id} onMove={(target) => place(id, target)} />
                  </ConversationSlot>
                )
              })}
            </ul>
          </Fold>
        )
      })}
      <Fold id={`${workspaceId}:channels`} title={t('shell.chat.channels')} badge={freeChannels.reduce((sum, channel) => sum + channel.unreadCount, 0)}>
        {freeChannels.length === 0 ? <p className="tree-empty">{t('tree.emptyChannels')}</p> : null}
        <ChannelList
          channels={freeChannels}
          expanded={expandedLists.channels === true}
          onMore={() => setExpandedLists((current) => ({ ...current, channels: !current.channels }))}
          render={(channel) => (
            <ConversationSlot key={channel.id} conversationId={channel.conversationId} onDrop={() => { if (dragging?.kind === 'conversation') place(dragging.id, null) }}>
              <ChannelLink workspaceId={workspaceId} channel={channel} />
              <MoveMenu sections={layout.sections} current={null} onMove={(target) => place(channel.conversationId, target)} />
            </ConversationSlot>
          )}
        />
      </Fold>
      {freeProjects.length > 0 ? (
        <Fold id={`${workspaceId}:projects`} title={t('shell.chat.projectChannels')} badge={freeProjects.reduce((sum, channel) => sum + channel.unreadCount, 0)}>
          <ChannelList
            channels={freeProjects}
            expanded={expandedLists.projects === true}
            onMore={() => setExpandedLists((current) => ({ ...current, projects: !current.projects }))}
            render={(channel) => (
              <ConversationSlot key={channel.id} conversationId={channel.conversationId} onDrop={() => { if (dragging?.kind === 'conversation') place(dragging.id, null) }}>
                <ChannelLink workspaceId={workspaceId} channel={channel} context={projectName(channel.projectId)} />
                <MoveMenu sections={layout.sections} current={null} onMove={(target) => place(channel.conversationId, target)} />
              </ConversationSlot>
            )}
          />
        </Fold>
      ) : null}
      <Fold id={`${workspaceId}:dms`} title={t('shell.chat.dms')} badge={freeDms.reduce((sum, dm) => sum + dm.unreadCount, 0)}>
        {freeDms.length === 0 ? <p className="tree-empty">{t('tree.emptyDms')}</p> : null}
        <ul className="conv-list">
          {freeDms.map((dm) => (
            <ConversationSlot key={dm.id} conversationId={dm.conversationId} onDrop={() => { if (dragging?.kind === 'conversation') place(dragging.id, null) }}>
              <DmLink workspaceId={workspaceId} dm={dm} locale={locale} fallback={t('dm.short')} />
              <MoveMenu sections={layout.sections} current={null} onMove={(target) => place(dm.conversationId, target)} />
            </ConversationSlot>
          ))}
        </ul>
      </Fold>
      <Dialog open={removing != null} title={t('shell.chat.deleteSection')} onClose={() => setRemoving(null)}>
        <p>{t('shell.chat.deleteSectionBody')}</p>
        <div className="dialog-actions">
          <Button variant="secondary" onClick={() => setRemoving(null)}>{t('shell.chat.cancel')}</Button>
          <Button onClick={() => { if (removing) commit(deleteSection(layout, removing.id)); setRemoving(null) }}>{t('shell.chat.confirm')}</Button>
        </div>
      </Dialog>
    </>
  )
}

function useMemoIds(channels: Channel[], projectChannels: Channel[], dms: Dm[]): Set<string> {
  const key = [...channels, ...projectChannels].map((channel) => channel.conversationId).concat(dms.map((dm) => dm.conversationId)).join('\n')
  const [ids, setIds] = useState(() => new Set(key ? key.split('\n') : []))
  useEffect(() => {
    setIds(new Set(key ? key.split('\n') : []))
  }, [key])
  return ids
}

function ConversationSlot({
  conversationId,
  children,
  marked = false,
  onDragOver,
  onDrop,
  onDragEnd,
}: {
  conversationId: string
  children: ReactNode
  marked?: boolean
  onDragOver?: () => void
  onDrop?: () => void
  onDragEnd?: () => void
}) {
  const [lifted, setLifted] = useState(false)
  return (
    <li
      className={`${marked ? 'conv-slot drop-before' : 'conv-slot'}${lifted ? ' is-dragging' : ''}`}
      draggable
      onDragStart={(event) => {
        setLifted(true)
        dragging = { kind: 'conversation', id: conversationId }
        event.dataTransfer.effectAllowed = 'move'
        event.dataTransfer.setData('text/plain', conversationId)
      }}
      onDragEnd={() => { dragging = null; setLifted(false); onDragEnd?.() }}
      onDragOver={(event) => {
        if (dragging?.kind !== 'conversation') return
        event.preventDefault()
        onDragOver?.()
      }}
      onDrop={(event) => {
        if (dragging?.kind !== 'conversation') return
        event.preventDefault()
        onDrop?.()
      }}
    >
      {children}
    </li>
  )
}

function ChannelList({
  channels,
  expanded,
  onMore,
  render,
}: {
  channels: Channel[]
  expanded: boolean
  onMore: () => void
  render: (channel: Channel) => ReactNode
}) {
  const { t } = useI18n()
  const slice = visibleSlice(channels, expanded, (channel) => channel.unreadCount > 0)
  return (
    <>
      <ul>{slice.shown.map((channel) => render(channel))}</ul>
      {slice.more > 0 || (expanded && channels.length > 10) ? (
        <button type="button" className="tree-more" onClick={onMore}>{expanded ? t('shell.chat.less') : t('shell.chat.more')}</button>
      ) : null}
    </>
  )
}

function MoveMenu({ sections, current, onMove }: { sections: ChatSection[]; current: string | null; onMove: (sectionId: string | null) => void }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(open, () => setOpen(false), ref)
  return (
    <div className="popover row-menu" ref={ref}>
      <button type="button" className="icon-add" aria-label={t('shell.chat.rowMenu')} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen((value) => !value)}>⋯</button>
      {open ? (
        <div className="menu-panel left" role="menu">
          <p className="menu-label">{t('shell.chat.moveTo')}</p>
          <button type="button" role="menuitem" disabled={current == null} onClick={() => { setOpen(false); onMove(null) }}>{t('shell.chat.defaultSection')}</button>
          {sections.map((section) => (
            <button key={section.id} type="button" role="menuitem" disabled={section.id === current} onClick={() => { setOpen(false); onMove(section.id) }}>{section.name}</button>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function SectionMenu({ onRename, onDelete }: { onRename: () => void; onDelete: () => void }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(open, () => setOpen(false), ref)
  return (
    <div className="popover row-menu" ref={ref}>
      <button type="button" className="icon-add" aria-label={t('shell.chat.sectionMenu')} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen((value) => !value)}>⋯</button>
      {open ? (
        <div className="menu-panel left" role="menu">
          <button type="button" role="menuitem" onClick={() => { setOpen(false); onRename() }}>{t('shell.chat.renameSection')}</button>
          <button type="button" role="menuitem" onClick={() => { setOpen(false); onDelete() }}>{t('shell.chat.deleteSection')}</button>
        </div>
      ) : null}
    </div>
  )
}

function NewConversationMenu({ workspaceId, onNewSection }: { workspaceId: string; onNewSection: () => void }) {
  const { t } = useI18n()
  const workspaceRole = useWorkspaceRole(workspaceId)
  const open = useCreate()
  const [menu, setMenu] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(menu, () => setMenu(false), ref)
  useEffect(() => {
    if (!menu) return
    function onKey(event: KeyboardEvent) {
      if (event.key === 'Escape') setMenu(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [menu])
  const items = [
    ['dm', t('shell.chat.newDm')],
    ['group', t('shell.chat.newGroup')],
    ...(canCreateChannel(workspaceRole) ? ([['channel', t('shell.chat.newChannel')]] as const) : []),
  ] as const
  return (
    <div className="popover" ref={ref}>
      <button type="button" className="nav-add" aria-expanded={menu} onClick={() => setMenu((current) => !current)}>
        + {t('shell.chat.new')}
      </button>
      {menu ? (
        <div className="menu-panel left" role="menu">
          {items.map(([kind, label]) => (
            <button
              key={kind}
              type="button"
              role="menuitem"
              onClick={() => {
                setMenu(false)
                open(kind)
              }}
            >
              {label}
            </button>
          ))}
          <button type="button" role="menuitem" onClick={() => { setMenu(false); onNewSection() }}>{t('shell.chat.newSection')}</button>
        </div>
      ) : null}
    </div>
  )
}

function ChannelLink({ workspaceId, channel, to, context }: { workspaceId: string; channel: Channel; to?: string; context?: string }) {
  return (
    <NavLink className="tree-link" to={to ?? `/w/${workspaceId}/c/${channel.conversationId}`}>
      <span className="hash">#</span> {channel.name}
      {context && context !== channel.name ? <small className="tree-context"> · {context}</small> : null}
      {channel.unreadCount > 0 ? <span className="unread-count">{channel.unreadCount}</span> : null}
    </NavLink>
  )
}

function DmLink({ workspaceId, dm, locale, fallback }: { workspaceId: string; dm: Dm; locale: string; fallback: string }) {
  const name = dm.peers.map((peer) => peer.displayName).join(', ') || fallback
  return (
    <NavLink className="conv-row" to={`/w/${workspaceId}/c/${dm.conversationId}`}>
      <Avatar name={name} />
      <span className="conv-copy">
        <strong>{name}</strong>
        {dm.lastMessagePreview ? <small>{dm.lastMessagePreview}</small> : null}
      </span>
      <span className="conv-meta">
        {dm.lastMessageAt ? <time dateTime={dm.lastMessageAt}>{formatMessageTime(dm.lastMessageAt, locale)}</time> : null}
        {dm.unreadCount > 0 ? <span className="unread-count">{dm.unreadCount}</span> : null}
      </span>
    </NavLink>
  )
}

function ProjectList({ workspaceId, apiProjects, teams }: { workspaceId: string; apiProjects: Array<{ id: string; name: string; participating?: boolean }>; teams: Array<{ id: string; name: string }> }) {
  const { t } = useI18n()
  const open = useCreate()
  const [params] = useSearchParams()
  const view = params.get('view')
  // Creating a Team or a Project is for a Workspace owner or admin.
  const canCreate = isWorkspaceManager(useWorkspaceRole(workspaceId))
  const mine = shellProjects.filter((project) => project.group === 'mine')
  const team = shellProjects.filter((project) => project.group === 'team')
  return (
    <>
      {canCreate ? <button type="button" className="nav-add" onClick={() => open('project')}>+ {t('shell.projects.new')}</button> : null}
      <section className="tree-block">
        <h3>{t('shell.projects.mine')}</h3>
        {mine.length === 0 && apiProjects.length === 0 ? <p className="tree-empty">{t('tree.emptyProjects')}</p> : null}
        <ul>
          {mine.map((project) => (
            <li key={project.id}>
              <NavLink className="tree-link" to={`/w/${workspaceId}/projects/${project.id}`}>{project.name}</NavLink>
            </li>
          ))}
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
      {team.length > 0 ? (
        <section className="tree-block">
          <h3>{t('shell.projects.team')}</h3>
          <ul>
            {team.map((project) => (
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
          {/* The three lists share one path, so the address query, not the path, says which is open. */}
          <li><NavLink end aria-current={view === null ? 'page' : 'false'} className={({ isActive }) => (isActive && view === null ? 'tree-link active' : 'tree-link')} to={`/w/${workspaceId}/projects`}>{t('shell.projects.all')}</NavLink></li>
          <li><NavLink className="tree-link" to={`/w/${workspaceId}/my-work`}>{t('shell.projects.tasks')}</NavLink></li>
          <li><NavLink end aria-current={view === 'recent' ? 'page' : 'false'} className={({ isActive }) => (isActive && view === 'recent' ? 'tree-link active' : 'tree-link')} to={`/w/${workspaceId}/projects?view=recent`}>{t('shell.projects.recent')}</NavLink></li>
        </ul>
      </section>
    </>
  )
}

function ProjectNav({
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

function DocsNav({ workspaceId }: { workspaceId: string }) {
  const [params, setParams] = useSearchParams()
  const selected = params.get('doc')
  return (
    <DocTree
      workspaceId={workspaceId}
      selectedId={selected === 'new' ? null : selected}
      onSelect={(id, options) => {
        const next = new URLSearchParams(params)
        next.delete('draft')
        next.delete('parent')
        if (id) next.set('doc', id)
        else next.delete('doc')
        if (id && options?.edit) next.set('edit', '1')
        else next.delete('edit')
        setParams(next)
      }}
      onDraft={(parentId) => {
        const next = new URLSearchParams(params)
        next.delete('doc')
        next.delete('edit')
        next.set('draft', '1')
        if (parentId) next.set('parent', parentId)
        else next.delete('parent')
        setParams(next)
      }}
    />
  )
}

function SearchNav({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const tab = params.get('tab') ?? 'all'
  const q = params.get('q') ?? ''
  return (
    <section className="tree-block">
      <h3>{t('shell.search.filters')}</h3>
      <ul>
        {SEARCH_TABS.map((id) => (
          <li key={id}>
            {/* Every tab shares one path, so a NavLink would mark all of them current; the tab parameter decides. */}
            <Link className={tab === id ? 'tree-link active' : 'tree-link'} aria-current={tab === id ? 'page' : undefined} to={`/w/${workspaceId}/search?tab=${id}${q ? `&q=${encodeURIComponent(q)}` : ''}`}>{t(`search2.tab.${id}`)}</Link>
          </li>
        ))}
      </ul>
    </section>
  )
}
