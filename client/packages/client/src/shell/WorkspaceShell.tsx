import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isNotFound } from '@penfii/api-client'
import { NavLink, useLocation, useNavigate, useParams } from 'react-router'
import { api } from '../api'
import { Avatar, Button, Dialog, Field, useDismiss } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { LogoutButton, useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { errorText } from '../ui/errorText'
import { NotFoundState } from '../ui/NotFoundState'
import { ContextSidebar } from './ContextSidebar'
import { DETAIL_WIDTH, OVERLAY_MAX_WIDTH, THREAD_DEFAULT, THREAD_MIN, THREAD_WIDTH_KEY, readStoredWidth, shouldCollapseNav } from './paneWidth'
import { usePaneWidth } from './usePaneWidth'
import { CreateProvider, useCreate, type CreateKind } from './create'
import { DetailPanel } from './DetailPanel'
import { useUserSocket } from '../modules/attention/useUserSocket'
import { CommandPalette } from '../modules/attention/CommandPalette'
import { ShortcutHelpProvider } from '../modules/help/ShortcutHelp'
import { useUnread } from '../modules/attention/useUnread'
import { IconChat, IconDocs, IconHome, IconInbox, IconProjects, IconSearch, IconSettings, IconTeams } from './icons'

export const LAST_WORKSPACE_KEY = 'penfii.lastWorkspace'

/**
 * App Shell은 Global Nav, Context Sidebar, Main, 선택 시 열리는 Detail Panel이다.
 * 이전 상단 바와 항상 같은 작업공간 트리는 이 구조와 맞지 않아 화면에서 뺐다.
 * 작업공간 전환은 Global Nav 위쪽 표시에 남긴다. 채팅 대화와 검색은 기존 API를 그대로 연다.
 * 프로젝트 보드의 Task는 아직 서버 모델이 없어 화면용 데이터로 둔다.
 */
export type ShellSection = 'home' | 'inbox' | 'chat' | 'projects' | 'docs' | 'teams' | 'search' | 'settings' | 'account'

export function WorkspaceShell({
  section,
  children,
  workspaceId: workspaceIdProp,
}: {
  section: ShellSection
  children: ReactNode
  workspaceId?: string
}) {
  const params = useParams()
  const workspaceId = params.workspaceId ?? workspaceIdProp
  const { t } = useI18n()
  const session = useSession()
  const location = useLocation()
  const panel = new URLSearchParams(location.search).get('panel')
  const ticketOpen = Boolean(panel?.startsWith('ticket'))
  const closeDetail = useRef<() => void>(() => {})
  const bindDetailClose = useCallback((close: () => void) => {
    closeDetail.current = close
  }, [])
  const ticketPane = usePaneWidth('ticket', { threadOpen: false, detailOpen: false, onTooNarrow: () => closeDetail.current() })
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
  useUserSocket()
  const unread = useUnread(workspaceId ?? '')
  const workspaces = useQuery(queries.workspaces())
  const space = useQuery({ ...queries.space(workspaceId ?? ''), enabled: Boolean(workspaceId) })

  useEffect(() => {
    if (!workspaceId) return
    try {
      sessionStorage.setItem(LAST_WORKSPACE_KEY, workspaceId)
    } catch {
      // The switcher still works from the loaded workspace list.
    }
  }, [workspaceId])

  if (!workspaceId) return <NotFoundState escape />
  const workspace = workspaces.data?.find((item) => item.id === workspaceId)
  if (workspaces.isSuccess && !workspace) return <NotFoundState escape />
  if (space.isError && isNotFound(space.error)) return <NotFoundState escape />

  const chatActive = section === 'chat' || location.pathname.includes('/c/')
  const area = section === 'account' ? 'settings' : section
  const rail = [
    { id: 'home', href: `/w/${workspaceId}`, label: t('nav.home'), icon: <IconHome />, active: area === 'home' },
    { id: 'inbox', href: `/w/${workspaceId}/inbox`, label: t('nav.inbox'), icon: <IconInbox />, active: area === 'inbox' },
    { id: 'chat', href: `/w/${workspaceId}/chat`, label: t('nav.chat'), icon: <IconChat />, active: chatActive },
    { id: 'projects', href: `/w/${workspaceId}/projects`, label: t('nav.projects'), icon: <IconProjects />, active: area === 'projects' },
    { id: 'docs', href: `/w/${workspaceId}/docs`, label: t('nav.docs'), icon: <IconDocs />, active: area === 'docs' },
    { id: 'teams', href: `/w/${workspaceId}/directory`, label: t('nav.teams'), icon: <IconTeams />, active: area === 'teams' },
    { id: 'search', href: `/w/${workspaceId}/search`, label: t('nav.search'), icon: <IconSearch />, active: area === 'search' },
  ]

  return (
    <ShortcutHelpProvider><CreateProvider workspaceId={workspaceId} space={space.data}>
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
          <NavLink to={`/w/${workspaceId}/settings`} className={area === 'settings' ? 'rail-link active' : 'rail-link'} aria-current={area === 'settings' ? 'page' : undefined}>
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
          section={chatActive ? 'chat' : area}
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
        <div className="product-main">{children}</div>
        <CommandPalette workspaceId={workspaceId} />
        <DetailPanel ticketPane={ticketPane} bindClose={bindDetailClose} />
      </div>
    </CreateProvider></ShortcutHelpProvider>
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
  const queryClient = useQueryClient()
  const prefs = useQuery(queries.notificationPreferences())
  const [open, setOpen] = useState(false)
  const [help, setHelp] = useState(false)
  const [statusOpen, setStatusOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useDismiss(open, () => setOpen(false), ref)
  const status = [statusEmoji, statusText].filter(Boolean).join(' ')
  async function snooze() {
    const current = prefs.data
    if (!current) return
    try {
      await api.saveNotificationPreferences({
        level: current.level,
        dnd: { ...current.dnd, enabled: true, snoozeUntil: new Date(Date.now() + 3_600_000).toISOString() },
        external: current.external,
        expectedVersion: current.version,
      })
      await queryClient.invalidateQueries({ queryKey: queryKeys.notificationPreferences })
      setOpen(false)
    } catch {
      // Preferences stay as they were; the notifications page reports a conflict.
    }
  }
  return (
    <div className="popover" ref={ref}>
      <button type="button" className="user-button" aria-label={name || t('account.title')} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((current) => !current)}>
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
          <button type="button" onClick={() => void snooze()}>{t('menu.dnd')}</button>
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
        onClose={() => setStatusOpen(false)}
        displayName={name}
        jobTitle={jobTitle}
        timeZone={timeZone}
        statusEmoji={statusEmoji}
        statusText={statusText}
        statusClearAt={statusClearAt}
      />
      <Dialog open={help} title={t('help.open')} onClose={() => setHelp(false)}>
        <p>{t('help.body')}</p>
      </Dialog>
    </div>
  )
}

function StatusDialog({
  open,
  onClose,
  displayName,
  jobTitle,
  timeZone,
  statusEmoji,
  statusText,
  statusClearAt,
}: {
  open: boolean
  onClose: () => void
  displayName: string
  jobTitle: string
  timeZone: string
  statusEmoji: string
  statusText: string
  statusClearAt: string
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [emoji, setEmoji] = useState(statusEmoji)
  const [text, setText] = useState(statusText)
  const [clearAt, setClearAt] = useState(statusClearAt ? toLocalInput(statusClearAt) : '')
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open) return
    setEmoji(statusEmoji)
    setText(statusText)
    setClearAt(statusClearAt ? toLocalInput(statusClearAt) : '')
    setError(null)
  }, [open, statusEmoji, statusText, statusClearAt])
  return (
    <Dialog open={open} title={t('menu.status')} onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          setError(null)
          void api.updateProfile({
            displayName,
            jobTitle: jobTitle || null,
            timeZone: timeZone || null,
            statusEmoji: emoji.trim() || null,
            statusText: text.trim() || null,
            statusClearAt: clearAt ? new Date(clearAt).toISOString() : null,
          }).then(async () => {
            await queryClient.invalidateQueries({ queryKey: queryKeys.me })
            onClose()
          }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'))))
        }}
      >
        <Field label={t('profile.status')}>
          <input value={emoji} onChange={(event) => setEmoji(event.target.value)} maxLength={16} />
        </Field>
        <Field label={t('profile.statusText')}>
          <input value={text} onChange={(event) => setText(event.target.value)} maxLength={80} />
        </Field>
        <Field label={t('profile.statusClear')}>
          <input type="datetime-local" value={clearAt} onChange={(event) => setClearAt(event.target.value)} />
        </Field>
        {error ? <p className="form-error">{error}</p> : null}
        <Button type="submit">{t('account.save')}</Button>
      </form>
    </Dialog>
  )
}

function toLocalInput(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function WorkspaceCreate() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const creation = useQuery(queries.workspaceCreation())
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [timeZone, setTimeZone] = useState(Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC')
  const [error, setError] = useState<string | null>(null)
  const zones = useMemo(() => {
    const all = Intl.supportedValuesOf('timeZone')
    const current = Intl.DateTimeFormat().resolvedOptions().timeZone
    return current && !all.includes(current) ? [current, ...all] : all
  }, [])
  // Only the first Workspace of an instance can be created; afterwards the server refuses.
  if (!creation.data) return null
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>{t('workspaces.create')}</button>
      <Dialog open={open} title={t('workspaces.create')} onClose={() => setOpen(false)}>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            setError(null)
            void api.createWorkspace({ name: name.trim(), timeZone }).then(async (created) => {
              setName('')
              await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces })
              setOpen(false)
              void navigate(`/w/${created.id}`)
            }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'))))
          }}
        >
          <Field label={t('workspaces.name')}>
            <input required value={name} onChange={(event) => setName(event.target.value)} />
          </Field>
          <Field label={t('workspaces.timeZone')}>
            <select value={timeZone} onChange={(event) => setTimeZone(event.target.value)}>
              {zones.map((zone) => (
                <option key={zone} value={zone}>{zone}</option>
              ))}
            </select>
          </Field>
          {error ? <p className="form-error">{error}</p> : null}
          <Button type="submit">{t('workspaces.create')}</Button>
        </form>
      </Dialog>
    </>
  )
}

export function OpenCreate({ kind, label }: { kind: CreateKind; label: string }) {
  const open = useCreate()
  return <Button onClick={() => open(kind)}>{label}</Button>
}
