import { createContext,useContext,useEffect,useLayoutEffect,useRef,useState,type KeyboardEvent as ReactKeyboardEvent,type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import type { Channel,Dm,Space } from '@penfii/contracts'
import { NavLink, useNavigate } from 'react-router'
import { Avatar,Button,Dialog,useDismiss } from '../../design/ui'
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
} from '../chatGroups'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { canCreateChannel, canManageChannelSettings } from '../../modules/spaces/capabilities'
import { useWorkspaceRole } from '../../modules/spaces/useWorkspaceRole'
import { useCreate } from '../create'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { FavoriteMenuItem } from '../../modules/personal/SavedButtons'
import { floatingMenuPosition } from './floatingMenuPosition'

function historyDms(space: Space | undefined): Dm[] {
  return (space?.dms ?? []).filter((dm) => dm.lastMessageSeq == null || dm.lastMessageSeq > 0)
}

type DragKind = { kind: 'conversation' | 'section'; id: string }

/** What is being dragged in one chat list. Kept per list (not in a module variable): two lists, or two tests, never share it. */
const DragContext = createContext<{ current: DragKind | null } | null>(null)

function DragScope({ children }: { children: ReactNode }) {
  const drag = useRef<DragKind | null>(null)
  return <DragContext.Provider value={drag}>{children}</DragContext.Provider>
}

function useDrag(): { current: DragKind | null } {
  const shared = useContext(DragContext)
  const own = useRef<DragKind | null>(null)
  return shared ?? own
}

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
  const drag = useDrag()
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
            drag.current = { kind: 'section', id }
            event.dataTransfer.effectAllowed = 'move'
            event.dataTransfer.setData('text/plain', id)
          }}
          onDragOver={(event) => {
            if (!drag.current) return
            if (drag.current.kind === 'section' && !draggable) return
            event.preventDefault()
            if (drag.current.kind === 'conversation') {
              onHeaderDragOver?.()
              armPeek()
            }
          }}
          onDrop={(event) => {
            if (!drag.current) return
            if (drag.current.kind === 'section' && !draggable) return
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

/** The Chat section of the side list. Each list keeps its own drag state (see `DragScope`). */
export function ChatNav(props: { workspaceId: string; space: Space | undefined }) {
  return (
    <DragScope>
      <ChatNavBody {...props} />
    </DragScope>
  )
}

function ChatNavBody({ workspaceId, space }: { workspaceId: string; space: Space | undefined }) {
  const { t, locale } = useI18n()
  const drag = useDrag()
  const workspaceRole = useWorkspaceRole(workspaceId)
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
    drag.current = null
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
        <FavoriteConversations workspaceId={workspaceId} space={space} sections={layout.sections} projectName={projectName} />
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
              if (drag.current?.kind === 'section') {
                commit(reorderSections(layout, drag.current.id, section.id))
                drag.current = null
                return
              }
              if (drag.current?.kind === 'conversation') place(drag.current.id, section.id)
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
                    onDrop={() => { if (drag.current?.kind === 'conversation') place(drag.current.id, section.id, index) }}
                  >
                    {row.kind === 'channel' ? (
                      <ChannelLink workspaceId={workspaceId} channel={row.channel} context={row.channel.projectId ? projectName(row.channel.projectId) : undefined} canManageSettings={canManageChannelSettings(row.channel, workspaceRole, space?.projects.find((item) => item.id === row.channel.projectId))} />
                    ) : (
                      <DmLink workspaceId={workspaceId} dm={row.dm} locale={locale} fallback={t('dm.short')} />
                    )}
                    <MoveMenu workspaceId={workspaceId} favorite={row.kind === 'channel' ? { kind: 'CHANNEL', targetId: row.channel.id } : { kind: 'DM', targetId: row.dm.conversationId }} sections={layout.sections} current={section.id} onMove={(target) => place(id, target)} />
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
            <ConversationSlot key={channel.id} conversationId={channel.conversationId} onDrop={() => { if (drag.current?.kind === 'conversation') place(drag.current.id, null) }}>
              <ChannelLink workspaceId={workspaceId} channel={channel} canManageSettings={canManageChannelSettings(channel, workspaceRole, space?.projects.find((item) => item.id === channel.projectId))} />
              <MoveMenu workspaceId={workspaceId} favorite={{ kind: 'CHANNEL', targetId: channel.id }} sections={layout.sections} current={null} onMove={(target) => place(channel.conversationId, target)} />
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
              <ConversationSlot key={channel.id} conversationId={channel.conversationId} onDrop={() => { if (drag.current?.kind === 'conversation') place(drag.current.id, null) }}>
                <ChannelLink workspaceId={workspaceId} channel={channel} context={projectName(channel.projectId)} canManageSettings={canManageChannelSettings(channel, workspaceRole, space?.projects.find((item) => item.id === channel.projectId))} />
                <MoveMenu workspaceId={workspaceId} favorite={{ kind: 'CHANNEL', targetId: channel.id }} sections={layout.sections} current={null} onMove={(target) => place(channel.conversationId, target)} />
              </ConversationSlot>
            )}
          />
        </Fold>
      ) : null}
      <Fold id={`${workspaceId}:dms`} title={t('shell.chat.dms')} badge={freeDms.reduce((sum, dm) => sum + dm.unreadCount, 0)}>
        {freeDms.length === 0 ? <p className="tree-empty">{t('tree.emptyDms')}</p> : null}
        <ul className="conv-list">
          {freeDms.map((dm) => (
            <ConversationSlot key={dm.id} conversationId={dm.conversationId} onDrop={() => { if (drag.current?.kind === 'conversation') place(drag.current.id, null) }}>
              <DmLink workspaceId={workspaceId} dm={dm} locale={locale} fallback={t('dm.short')} />
              <MoveMenu workspaceId={workspaceId} favorite={{ kind: 'DM', targetId: dm.conversationId }} sections={layout.sections} current={null} onMove={(target) => place(dm.conversationId, target)} />
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
  const drag = useDrag()
  const [lifted, setLifted] = useState(false)
  return (
    <li
      className={`${marked ? 'conv-slot drop-before' : 'conv-slot'}${lifted ? ' is-dragging' : ''}`}
      draggable
      onDragStart={(event) => {
        setLifted(true)
        drag.current = { kind: 'conversation', id: conversationId }
        event.dataTransfer.effectAllowed = 'move'
        event.dataTransfer.setData('text/plain', conversationId)
      }}
      onDragEnd={() => { drag.current = null; setLifted(false); onDragEnd?.() }}
      onDragOver={(event) => {
        if (drag.current?.kind !== 'conversation') return
        event.preventDefault()
        onDragOver?.()
      }}
      onDrop={(event) => {
        if (drag.current?.kind !== 'conversation') return
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

const MENU_ITEMS = '[role="menuitem"]:not(:disabled), [role="menuitemcheckbox"]:not(:disabled)'

type FavoriteTarget = { kind: 'CHANNEL' | 'DM'; targetId: string }

/**
 * The chat's Favorites: shortcuts to the channels and DMs the person starred, in their Saved order
 * (folders do not show here). They are extra ways in: each conversation stays in its own group too.
 * A shortcut opens the conversation in Chat and shows the same unread count as its row elsewhere; the
 * group itself adds no count, so nothing is counted twice.
 */
function FavoriteConversations({ workspaceId, space, sections, projectName }: { workspaceId: string; space: Space | undefined; sections: ChatSection[]; projectName: (id: string | null) => string }) {
  const { t, locale } = useI18n()
  const workspaceRole = useWorkspaceRole(workspaceId)
  const queryClient = useQueryClient()
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  const conversations = (favorites.data ?? []).filter((item): item is typeof item & FavoriteTarget => item.kind === 'CHANNEL' || item.kind === 'DM')
  // The side list holds every conversation the person can read. Once it is loaded, a shortcut missing from it was
  // left or lost: it is not shown, and the Favorites list is asked for again (once per change).
  const listed = (item: FavoriteTarget) => !space || (item.kind === 'CHANNEL' ? space.channels.some((entry) => entry.id === item.targetId) : space.dms.some((entry) => entry.conversationId === item.targetId))
  const missing = conversations.filter((item) => !listed(item)).map((item) => item.targetId).join(',')
  const count = conversations.length
  // The list, or its empty line: whichever is shown, it sits in the group whose heading takes the focus.
  const anchor = useRef<HTMLUListElement & HTMLParagraphElement>(null)
  const removing = useRef(false)
  // A shortcut removed from its own menu takes its row (and the focused menu button) with it: focus goes to
  // the group's heading instead of being lost.
  useEffect(() => {
    if (!removing.current) return
    removing.current = false
    const lost = document.activeElement === null || document.activeElement === document.body
    if (lost) anchor.current?.closest('section')?.querySelector<HTMLElement>('.tree-fold')?.focus()
  }, [count])
  useEffect(() => {
    if (missing) void queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) })
  }, [missing, queryClient, workspaceId])
  if (!favorites.data) {
    if (favorites.isError) {
      return (
        <p className="tree-empty" role="alert">
          {t('saved.favorites.loadError')}{' '}
          <button type="button" className="tree-more" onClick={() => void favorites.refetch()}>{t('saved.favorites.retry')}</button>
        </p>
      )
    }
    return <p className="tree-empty" role="status">{t('saved.favorites.loading')}</p>
  }
  const rows = conversations.filter(listed)
  if (rows.length === 0) return <p ref={anchor} className="tree-empty">{t('shell.empty.favorites')}</p>
  return (
    <ul ref={anchor} className="conv-list favorite-conversations">
      {rows.map((item) => {
        const target: FavoriteTarget = { kind: item.kind, targetId: item.targetId }
        const channel = item.kind === 'CHANNEL' ? space?.channels.find((entry) => entry.id === item.targetId) : undefined
        const dm = item.kind === 'DM' ? space?.dms.find((entry) => entry.conversationId === item.targetId) : undefined
        return (
          <li key={`${item.kind}:${item.targetId}`} className="conv-slot">
            {channel ? (
              <ChannelLink workspaceId={workspaceId} channel={channel} context={channel.projectId ? projectName(channel.projectId) : undefined} canManageSettings={canManageChannelSettings(channel, workspaceRole, space?.projects.find((entry) => entry.id === channel.projectId))} />
            ) : dm ? (
              <DmLink workspaceId={workspaceId} dm={dm} locale={locale} fallback={t('dm.short')} />
            ) : (
              // The side list is not loaded yet: the server says it can be read, so it opens.
              <NavLink className="tree-link" to={`/w/${workspaceId}/c/${item.targetId}`}>
                {item.kind === 'CHANNEL' ? <span className="hash">#</span> : null} {item.title || t('dm.short')}
              </NavLink>
            )}
            <MoveMenu workspaceId={workspaceId} favorite={target} sections={sections} current={undefined} onFavoriteDone={() => { removing.current = true }} />
          </li>
        )
      })}
    </ul>
  )
}

/**
 * A conversation row's menu: the chat group to move it to, then Favorite. A shortcut in Favorites has
 * no `onMove` (it is not placed in a group; its own row is).
 */
function MoveMenu({ workspaceId, favorite, sections, current, onMove, onFavoriteDone }: { workspaceId: string; favorite: FavoriteTarget; sections: ChatSection[]; current: string | null | undefined; onMove?: (sectionId: string | null) => void; onFavoriteDone?: () => void }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const menu = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<{ top: number; left: number; maxHeight: number; maxWidth: number } | null>(null)
  const positioned = position !== null
  useEffect(() => {
    if (!open) return
    const closeOutside = (event: PointerEvent) => {
      const target = event.target as Node
      if (!ref.current?.contains(target) && !menu.current?.contains(target)) {
        setOpen(false)
        trigger.current?.focus()
      }
    }
    const closeEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        setOpen(false)
        trigger.current?.focus()
      }
    }
    document.addEventListener('pointerdown', closeOutside, true)
    document.addEventListener('keydown', closeEscape, true)
    return () => {
      document.removeEventListener('pointerdown', closeOutside, true)
      document.removeEventListener('keydown', closeEscape, true)
    }
  }, [open])
  useLayoutEffect(() => {
    if (!open) { setPosition(null); return }
    const update = () => {
      const anchor = trigger.current?.getBoundingClientRect()
      const popup = menu.current?.getBoundingClientRect()
      if (!anchor || !popup) return
      setPosition(floatingMenuPosition(anchor, popup, { width: window.innerWidth, height: window.innerHeight }))
    }
    update()
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update)
    if (menu.current) observer?.observe(menu.current)
    window.addEventListener('resize', update)
    window.addEventListener('scroll', update, true)
    return () => { observer?.disconnect(); window.removeEventListener('resize', update); window.removeEventListener('scroll', update, true) }
  }, [open, sections.length])
  // The first render is visibility:hidden until its fixed position is measured. Browsers refuse to
  // focus a hidden menu item, so wait for the measured/visible render before moving focus.
  useEffect(() => { if (open && positioned) menu.current?.querySelector<HTMLElement>(MENU_ITEMS)?.focus() }, [open, positioned])
  function navigateItems(event: ReactKeyboardEvent<HTMLDivElement>) {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>(MENU_ITEMS) ?? [])]
    if (!items.length) return
    const currentIndex = items.indexOf(document.activeElement as HTMLElement)
    let next: number | null = null
    if (event.key === 'ArrowDown') next = (currentIndex + 1 + items.length) % items.length
    if (event.key === 'ArrowUp') next = (currentIndex - 1 + items.length) % items.length
    if (event.key === 'Home') next = 0
    if (event.key === 'End') next = items.length - 1
    if (next !== null) { event.preventDefault(); items[next]?.focus() }
  }
  return (
    <div className="popover row-menu" ref={ref}>
      <button ref={trigger} type="button" className="icon-add" aria-label={t('shell.chat.rowMenu')} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen((value) => !value)}>⋯</button>
      {open ? createPortal(
        <div ref={menu} className="menu-panel floating-nav-menu" style={position ? { top: position.top, left: position.left, maxHeight: position.maxHeight, maxWidth: position.maxWidth, minWidth: 0, visibility: 'visible' } : { visibility: 'hidden' }} role="menu" onKeyDown={navigateItems}>
          {onMove ? (
            <>
              <p className="menu-label">{t('shell.chat.moveTo')}</p>
              <button type="button" role="menuitem" disabled={current == null} onClick={() => { setOpen(false); trigger.current?.focus(); onMove(null) }}>{t('shell.chat.defaultSection')}</button>
              {sections.map((section) => (
                <button key={section.id} type="button" role="menuitem" disabled={section.id === current} onClick={() => { setOpen(false); trigger.current?.focus(); onMove(section.id) }}>{section.name}</button>
              ))}
              <hr className="menu-separator" />
            </>
          ) : null}
          <FavoriteMenuItem workspaceId={workspaceId} kind={favorite.kind} targetId={favorite.targetId} onDone={() => { setOpen(false); trigger.current?.focus(); onFavoriteDone?.() }} />
        </div>, document.body,
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

export function ChannelLink({ workspaceId, channel, to, context, canManageSettings = false }: { workspaceId: string; channel: Channel; to?: string; context?: string; canManageSettings?: boolean }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const destination = to ?? `/w/${workspaceId}/c/${channel.conversationId}`
  return (
    <>
      <NavLink className="tree-link" to={destination}>
        <span className="hash">#</span> {channel.name}
        {context && context !== channel.name ? <small className="tree-context"> · {context}</small> : null}
        {channel.unreadCount > 0 ? <span className="unread-count">{channel.unreadCount}</span> : null}
      </NavLink>
      {canManageSettings ? <button type="button" className="channel-settings-trigger" aria-label={t('channel.settings')} title={t('channel.settings')} onClick={() => { void navigate(`${destination}?settings=1`) }}>⚙</button> : null}
    </>
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

