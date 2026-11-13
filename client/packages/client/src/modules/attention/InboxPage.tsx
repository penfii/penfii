import { useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router'
import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import type { InboxItem, InboxLink } from '@penfii/contracts'
import { isVersionConflict } from '@penfii/api-client'
import { api } from '../../api'
import { Button, EmptyState } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { InboxChips, InboxRows, InboxViewOptionsMenu } from './InboxParts'
import { DEFAULT_VIEW_OPTIONS, emptyKind, inboxSearch, parseInboxLocation, readState, type InboxLocation } from './inboxModel'
import { useInboxViewOptions } from './useInboxViewOptions'

/** Where an Inbox item leads. */
export function inboxHref(workspaceId: string, link: InboxLink): string | null {
  const base = `/w/${workspaceId}`
  if (link.type === 'ticket') return link.projectId ? `${base}/projects/${link.projectId}/work?panel=ticket:${link.ticketId}` : null
  if (link.type === 'project') return `${base}/projects/${link.projectId}`
  if (link.type === 'doc') return `${base}/docs?doc=${link.docId}`
  if (!link.conversationId) return null
  return link.threadRootId ? `${base}/c/${link.conversationId}/t/${link.threadRootId}` : `${base}/c/${link.conversationId}`
}

/**
 * The Inbox and the archive (one screen, two Views). What happened that concerns me. Reading it never
 * changes chat, Later or any Ticket. The address holds the View, the types and the unread Filter; the
 * View Options are a local preference.
 */
export function InboxPage() {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [options, setOptions] = useInboxViewOptions()
  const location = parseInboxLocation(params)
  const archive = location.view === 'archived'
  // The unread Filter wins over "show read"; the server has the one `state`.
  const state = archive ? 'all' : readState(location.unread, options.showRead)
  const includeSnoozed = !archive && options.showSnoozed
  const [notice, setNotice] = useState('')
  const types = location.types
  const inbox = useInfiniteQuery({
    queryKey: [...queryKeys.inbox(workspaceId, location.view, `${state}:${includeSnoozed ? 'snoozed' : ''}`), types],
    queryFn: ({ pageParam }) =>
      api.getInbox(workspaceId, {
        view: location.view,
        filter: types.length > 0 ? types : undefined,
        state,
        includeSnoozed: includeSnoozed || undefined,
        cursor: pageParam,
        limit: 30,
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: workspaceId.length > 0,
    refetchOnWindowFocus: true,
  })
  const items = inbox.data?.pages.flatMap((page) => page.items) ?? []
  const heading = t(archive ? 'inbox.view.archived' : 'inbox.view.inbox')

  function go(next: InboxLocation) {
    setParams(new URLSearchParams(inboxSearch(next)), { replace: false })
  }
  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.inbox })
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.inboxUnread })
  }
  type Command = 'read' | 'unread' | 'archive' | 'unarchive' | 'snooze'
  // What can be taken back, for a few seconds. Only what makes a row leave the list (or hides it
  // for a while) is offered: the read toggle stays on the row and flips back with its own button.
  const [undo, setUndo] = useState<{ message: string; undo: () => Promise<void> } | null>(null)
  useEffect(() => {
    if (!undo) return
    const timer = setTimeout(() => setUndo(null), 8000)
    return () => clearTimeout(timer)
  }, [undo])
  async function send(item: InboxItem, command: Command, until?: string) {
    return api.inboxCommand(item.id, command, { expectedVersion: item.version, until })
  }
  /** What taking `command` back looks like, or null when it is not worth offering. */
  function undoFor(item: InboxItem, command: Command, until: string | undefined, done: { version: number }) {
    const next = (command: Command, version: number) => api.inboxCommand(item.id, command, { expectedVersion: version })
    if (command === 'archive') {
      // Leaving the archive puts it back as read; it was unread when it went.
      return { message: t('inbox.undo.archived'), undo: async () => { const back = await next('unarchive', done.version); if (item.state === 'UNREAD') await next('unread', back.version) } }
    }
    if (command === 'unarchive') return { message: t('inbox.undo.restored'), undo: async () => { await next('archive', done.version) } }
    if (command === 'snooze' && until) {
      // Waking it clears the snooze and leaves it unread; it may have been read before.
      return { message: t('inbox.undo.snoozed', { time: formatMessageTime(until, locale) }), undo: async () => { const back = await next('unread', done.version); if (item.state === 'READ') await next('read', back.version) } }
    }
    if (command === 'read' && state === 'unread') return { message: t('inbox.undo.read'), undo: async () => { await next('unread', done.version) } }
    return null
  }
  async function run(item: InboxItem, command: Command, until?: string, quiet = false) {
    setNotice('')
    setUndo(null)
    try {
      const done = await send(item, command, until)
      if (!quiet) setUndo(undoFor(item, command, until, done))
    } catch (error) {
      if (isVersionConflict(error)) setNotice(t('inbox.conflict'))
      else setNotice(t('inbox.error'))
    }
    await refresh()
  }
  async function takeBack() {
    const pending = undo
    if (!pending) return
    setUndo(null)
    try {
      await pending.undo()
    } catch {
      setNotice(t('inbox.conflict'))
    }
    await refresh()
  }
  async function open(item: InboxItem) {
    const href = inboxHref(workspaceId, item.link)
    if (!archive && item.state === 'UNREAD') await run(item, 'read', undefined, true)
    if (href) void navigate(href)
  }
  async function readAll() {
    await api.inboxReadAll(workspaceId, location.types.length > 0 ? location.types : undefined)
    await refresh()
  }
  function clearFilters() {
    go({ ...location, types: [], unread: false })
    setOptions({ showRead: DEFAULT_VIEW_OPTIONS.showRead, showSnoozed: DEFAULT_VIEW_OPTIONS.showSnoozed })
  }

  const empty = inbox.isSuccess && items.length === 0
  const nothingAtAll = empty && emptyKind(location, options) === 'none'
  return (
    <div className="page-scroll">
      <header className="page-intro inbox-heading">
        <div>
          <h1>{heading}</h1>
          <p>{t(archive ? 'inbox.archiveLead' : 'inbox.lead')}</p>
        </div>
        {archive || nothingAtAll ? null : <Button type="button" variant="secondary" onClick={() => void readAll()}>{t('inbox.markAllRead')}</Button>}
      </header>
      {nothingAtAll ? null : (
        <div className="inbox-toolbar">
          <InboxChips location={location} onChange={go} showUnread={!archive} />
          <InboxViewOptionsMenu options={options} onChange={setOptions} archive={archive} unreadFilter={location.unread} />
        </div>
      )}
      {notice ? <p className="board-note" role="status">{notice}</p> : null}
      {inbox.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {inbox.isError ? <p className="board-error" role="alert">{t('inbox.error')}</p> : null}
      {nothingAtAll ? <EmptyState title={heading} body={archive ? t('inbox.archiveEmpty') : t('empty.inbox')} /> : null}
      {empty && !nothingAtAll ? (
        <EmptyState
          title={t('inbox.noMatch')}
          body={!archive && location.types.length === 1 && location.types[0] === 'invitations' ? t('inbox.invitationsEmpty') : t('inbox.emptyFilter')}
        >
          <Button type="button" variant="secondary" onClick={clearFilters}>{t('inbox.clearFilters')}</Button>
        </EmptyState>
      ) : null}
      <InboxRows items={items} archive={archive} groupByDay={options.groupByDay} actions={{ open: (item) => void open(item), run: (item, command, until) => void run(item, command, until) }} />
      {inbox.hasNextPage ? (
        <div className="inbox-more">
          <Button type="button" variant="secondary" disabled={inbox.isFetchingNextPage} onClick={() => void inbox.fetchNextPage()}>{t('inbox.more')}</Button>
        </div>
      ) : null}
      {/* After the list, so showing it never moves the rows; it stays at the bottom edge while the list scrolls. */}
      {undo ? (
        <div className="inbox-undo" role="status">
          <span>{undo.message}</span>
          <button type="button" className="inbox-undo-action" onClick={() => void takeBack()}>{t('inbox.undo')}</button>
        </div>
      ) : null}
    </div>
  )
}
