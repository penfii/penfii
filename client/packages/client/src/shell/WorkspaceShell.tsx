import { useEffect, useRef, useState } from 'react'
import { NavLink, useLocation } from 'react-router'
import { Avatar, Button, useDismiss } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { LogoutButton, useSession } from '../modules/identity'
import { ContextSidebar } from './ContextSidebar'
import { DETAIL_WIDTH, OVERLAY_MAX_WIDTH, THREAD_DEFAULT, THREAD_MIN, THREAD_WIDTH_KEY, readStoredWidth, shouldCollapseNav } from './paneWidth'
import { usePaneWidth } from './usePaneWidth'
import { useCreate, type CreateKind } from './create'
import { HostSlot, type ShellSlots } from './stableSlots'
import type { PaneControls } from './ResizeHandle'
import { useUnread } from '../modules/attention/useUnread'
import { IconSettings } from './icons'
import { NAV_AREAS, activeArea } from './navAreas'
import { HelpDialog, StatusDialog, WorkspaceCreate, useActiveWorkspace, useSnooze } from './account'

/**
 * App Shell은 Global Nav, Context Sidebar, Main, 선택 시 열리는 Detail Panel이다.
 * 이전 상단 바와 항상 같은 작업공간 트리는 이 구조와 맞지 않아 화면에서 뺐다.
 * 작업공간 전환은 Global Nav 위쪽 표시에 남긴다. 채팅 대화와 검색은 기존 API를 그대로 연다.
 * 프로젝트 보드의 Task는 아직 서버 모델이 없어 화면용 데이터로 둔다.
 */
export type ShellSection = 'home' | 'inbox' | 'chat' | 'projects' | 'docs' | 'teams' | 'search' | 'settings' | 'account'

/**
 * The desktop frame: rail, Context Sidebar and the places for the page and the Detail Panel. What goes in those
 * places is owned by `ResponsiveShell`, so it stays mounted when the window is resized into the phone layout.
 */
export function WorkspaceShell({
  section,
  workspaceId,
  slots,
  ticketPane,
}: {
  section: ShellSection
  workspaceId: string
  slots: ShellSlots
  ticketPane: PaneControls
}) {  const { t } = useI18n()
  const session = useSession()
  const location = useLocation()
  const panel = new URLSearchParams(location.search).get('panel')
  const ticketOpen = Boolean(panel?.startsWith('ticket'))
  const threadOpen = location.pathname.includes('/t/')
  const navPane = usePaneWidth('nav', {
    threadOpen,
    detailOpen: Boolean(panel),
    detailWidth: ticketOpen ? ticketPane.width : undefined,
  })
  const [viewport, setViewport] = useState(() => (typeof window === 'undefined' ? 1440 : window.innerWidth))
  const [navRevealed, setNavRevealed] = useState(false)
  useEffect(() => {
    const apply = () => setViewport(window.innerWidth)
    window.addEventListener('resize', apply)
    return () => window.removeEventListener('resize', apply)
  }, [])
  const threadColumn = threadOpen ? readStoredWidth(THREAD_WIDTH_KEY, THREAD_DEFAULT, THREAD_MIN) : 0
  // Below the overlay breakpoint a ticket panel floats, so it does not take a grid column.
  const ticketColumn = ticketOpen && viewport > OVERLAY_MAX_WIDTH ? ticketPane.width : 0
  const detailColumn = panel && !ticketOpen ? DETAIL_WIDTH : 0
  const sideWidth = threadColumn + ticketColumn + detailColumn
  const navCollapsed = shouldCollapseNav(viewport, navPane.width, sideWidth)
  useEffect(() => { if (!navCollapsed) setNavRevealed(false) }, [navCollapsed])
  const unread = useUnread(workspaceId)
  const { workspaces, workspace } = useActiveWorkspace(workspaceId)

  const current = activeArea(section, location.pathname)
  // Settings sits at the foot of the rail; the other areas are its links.
  const rail = NAV_AREAS.filter((item) => item.id !== 'settings').map(({ id, path, label, Icon }) => ({
    id,
    href: `/w/${workspaceId}${path}`,
    label: t(label),
    icon: <Icon />,
    active: current === id,
  }))

  return (
    <>
      <div
        className={[
          'product-shell',
          panel ? 'has-detail' : '',
          panel?.startsWith('ticket') ? 'detail-ticket' : '',
          navCollapsed ? 'nav-collapsed' : '',
          navCollapsed && navRevealed ? 'nav-revealed' : '',
        ].filter(Boolean).join(' ')}
        style={{
          ['--nav-width' as string]: `${String(navPane.width)}px`,
          ...(ticketOpen ? { ['--ticket-width' as string]: `${String(ticketPane.width)}px` } : {}),
        }}
      >
        <nav className="rail" aria-label={t('nav.workspace')}>
          <WorkspaceMark name={workspace?.name ?? t('loading')} workspaceId={workspaceId} workspaces={workspaces.data ?? []} />
          {rail.map((item) => (
            <NavLink key={item.id} to={item.href} end={item.id === 'home'} className={item.active ? 'rail-link active' : 'rail-link'} aria-current={item.active ? 'page' : undefined}>
              {item.icon}
              {item.label}
              {item.id === 'inbox' && unread.count > 0 ? (
                <span className="rail-badge" role="status" aria-label={t('inbox.unreadBadge', { count: unread.capped ? '99+' : String(unread.count) })}>{unread.capped ? '99+' : unread.count}</span>
              ) : null}
            </NavLink>
          ))}
          <div className="rail-spacer" />
          <NavLink to={`/w/${workspaceId}/settings`} className={current === 'settings' ? 'rail-link active' : 'rail-link'} aria-current={current === 'settings' ? 'page' : undefined}>
            <IconSettings />
            {t('nav.settings')}
          </NavLink>
          <UserMenu
            workspaceId={workspaceId}
            name={session.data?.displayName ?? ''}
            email={session.data?.email ?? ''}
            userId={session.data?.id}
            hasAvatar={session.data?.hasAvatar}
            statusEmoji={session.data?.statusEmoji ?? ''}
            statusText={session.data?.statusText ?? ''}
            statusClearAt={session.data?.statusClearAt ?? ''}
            jobTitle={session.data?.jobTitle ?? ''}
            timeZone={session.data?.timeZone ?? ''}
            workspaces={workspaces.data ?? []}
          />
        </nav>
        <ContextSidebar
          workspaceId={workspaceId}
          section={current}
          navWidth={navPane.width}
          navMin={navPane.min}
          navMax={navPane.max}
          onResizePointerDown={(event) => navPane.beginDrag(event, 1)}
        />
        {navCollapsed ? (
          <button type="button" className="nav-edge" aria-expanded={navRevealed} aria-controls="workspace-nav" onClick={() => setNavRevealed((open) => !open)}>
            {t('layout.showNav')}
          </button>
        ) : null}
        {navCollapsed && navRevealed ? <button type="button" className="nav-backdrop" aria-label={t('close')} onClick={() => setNavRevealed(false)} /> : null}
        <HostSlot host={slots.page} />
        <HostSlot host={slots.detail} />
      </div>
    </>
  )
}

function WorkspaceMark({
  name,
  workspaceId,
  workspaces,
}: {
  name: string
  workspaceId: string
  workspaces: Array<{ id: string; name: string }>
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(open, () => setOpen(false), ref)
  const initial = name.trim().slice(0, 1).toUpperCase() || 'O'
  return (
    <div className="popover rail-mark" ref={ref}>
      <button type="button" className="brand-mark" aria-label={`${t('shell.switcher')}: ${name}`} title={name} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        {initial}
      </button>
      {open ? (
        <div className="popover-panel">
          {workspaces.map((item) => (
            <NavLink key={item.id} to={`/w/${item.id}`} aria-current={item.id === workspaceId ? 'page' : undefined} onClick={() => setOpen(false)}>
              {item.name}
              {item.id === workspaceId ? <span className="current-mark" aria-hidden="true">✓</span> : null}
            </NavLink>
          ))}
          <WorkspaceCreate />
        </div>
      ) : null}
    </div>
  )
}

function UserMenu({
  workspaceId,
  name,
  email,
  userId,
  hasAvatar,
  statusEmoji,
  statusText,
  statusClearAt,
  jobTitle,
  timeZone,
  workspaces,
}: {
  workspaceId: string
  name: string
  email: string
  userId?: string
  hasAvatar?: boolean
  statusEmoji: string
  statusText: string
  statusClearAt: string
  jobTitle: string
  timeZone: string
  workspaces: Array<{ id: string; name: string }>
}) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const [help, setHelp] = useState(false)
  const [statusOpen, setStatusOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  // The menu item that opened a dialog is gone when the dialog closes, so focus goes back to the button that opened the menu.
  const closeDialog = (close: () => void) => () => {
    close()
    trigger.current?.focus()
  }
  useDismiss(open, () => setOpen(false), ref)
  const status = [statusEmoji, statusText].filter(Boolean).join(' ')
  const snooze = useSnooze()
  return (
    <div className="popover" ref={ref}>
      <button ref={trigger} type="button" className="user-button" aria-label={name || t('account.title')} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((current) => !current)}>
        <Avatar name={name || t('account.title')} userId={userId} hasAvatar={hasAvatar} />
      </button>
      {open ? (
        <div className="popover-panel" style={{ top: 'auto', bottom: 0, left: 'calc(100% + 8px)', right: 'auto' }}>
          <div className="menu-identity">
            <Avatar name={name || t('account.title')} userId={userId} hasAvatar={hasAvatar} />
            <span>
              <strong>{name}</strong>
              {email ? <small className="hint">{email}</small> : null}
              {status ? <small className="hint">{status}</small> : null}
            </span>
          </div>
          <button type="button" onClick={() => { setOpen(false); setStatusOpen(true) }}>{t('menu.status')}</button>
          <NavLink to={`/w/${workspaceId}/settings`} onClick={() => setOpen(false)}>{t('menu.profile')}</NavLink>
          <NavLink to={`/w/${workspaceId}/settings/account`} onClick={() => setOpen(false)}>{t('nav.settings')}</NavLink>
          <button type="button" onClick={() => void snooze().then((saved) => saved && setOpen(false))}>{t('menu.dnd')}</button>
          {workspaces.length > 1 ? workspaces.map((item) => (
            <NavLink key={item.id} to={`/w/${item.id}`} onClick={() => setOpen(false)}>
              {t('menu.switch')}: {item.name}
            </NavLink>
          )) : null}
          <LogoutButton />
          <button type="button" onClick={() => { setOpen(false); setHelp(true) }}>{t('help.open')}</button>
        </div>
      ) : null}
      <StatusDialog
        open={statusOpen}
        onClose={closeDialog(() => setStatusOpen(false))}
        displayName={name}
        jobTitle={jobTitle}
        timeZone={timeZone}
        statusEmoji={statusEmoji}
        statusText={statusText}
        statusClearAt={statusClearAt}
      />
      <HelpDialog open={help} onClose={closeDialog(() => setHelp(false))} />
    </div>
  )
}

export function OpenCreate({ kind, label }: { kind: CreateKind; label: string }) {
  const open = useCreate()
  return <Button onClick={() => open(kind)}>{label}</Button>
}
