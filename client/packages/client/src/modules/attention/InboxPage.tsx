import { useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router'
import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import type { InboxFilter, InboxItem, InboxLink } from '@penfii/contracts'
import { isVersionConflict } from '@penfii/api-client'
import { api } from '../../api'
import { Button, EmptyState } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'

export const INBOX_FILTERS: InboxFilter[] = ['all', 'mentions', 'replies', 'watching', 'reminders', 'invitations', 'assignments']

/** Where an Inbox item leads. */
export function inboxHref(workspaceId: string, link: InboxLink): string | null {
  const base = `/w/${workspaceId}`
  if (link.type === 'ticket') return link.projectId ? `${base}/projects/${link.projectId}/work?panel=ticket:${link.ticketId}` : null
  if (link.type === 'project') return `${base}/projects/${link.projectId}`
  if (link.type === 'doc') return `${base}/docs?doc=${link.docId}`
  if (!link.conversationId) return null
  return link.threadRootId ? `${base}/c/${link.conversationId}/t/${link.threadRootId}` : `${base}/c/${link.conversationId}`
}

function tomorrowMorning(): string {
  const next = new Date()
  next.setDate(next.getDate() + 1)
  next.setHours(9, 0, 0, 0)
  return next.toISOString()
}

/** The Inbox: what happened that concerns me. Reading it never changes chat, Later or any Ticket. */
export function InboxPage() {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const box = params.get('box') ?? 'all'
  const unreadOnly = box === 'unread'
  const filter: InboxFilter = INBOX_FILTERS.includes(box as InboxFilter) ? (box as InboxFilter) : 'all'
  const state = unreadOnly ? 'unread' : 'all'
  const [notice, setNotice] = useState('')
  const inbox = useInfiniteQuery({
    queryKey: queryKeys.inbox(workspaceId, filter, state),
    queryFn: ({ pageParam }) => api.getInbox(workspaceId, { filter, state, cursor: pageParam, limit: 30 }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: workspaceId.length > 0,
    refetchOnWindowFocus: true,
  })
  const items = inbox.data?.pages.flatMap((page) => page.items) ?? []
  const heading = unreadOnly ? t('inbox.unreadOnly') : t(`inbox.filter.${filter}`)

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.inbox })
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.inboxUnread })
  }
  async function run(item: InboxItem, command: 'read' | 'unread' | 'archive' | 'snooze', until?: string) {
    setNotice('')
    try {
      await api.inboxCommand(item.id, command, { expectedVersion: item.version, until })
    } catch (error) {
      if (isVersionConflict(error)) setNotice(t('inbox.conflict'))
      else setNotice(t('inbox.error'))
    }
    await refresh()
  }
  async function open(item: InboxItem) {
    const href = inboxHref(workspaceId, item.link)
    if (item.state === 'UNREAD') await run(item, 'read')
    if (href) void navigate(href)
  }
  async function readAll() {
    await api.inboxReadAll(workspaceId, filter === 'all' ? undefined : filter)
    await refresh()
  }

  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{heading}</h1>
        <p>{t('inbox.lead')}</p>
      </header>
      <div className="inbox-tools">
        <Button type="button" variant="secondary" onClick={() => void readAll()}>{t('inbox.markAllRead')}</Button>
      </div>
      {notice ? <p className="board-note" role="status">{notice}</p> : null}
      {inbox.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {inbox.isError ? <p className="board-error" role="alert">{t('inbox.error')}</p> : null}
      {inbox.isSuccess && items.length === 0 ? (
        <EmptyState title={heading} body={filter === 'invitations' ? t('inbox.invitationsEmpty') : unreadOnly ? t('empty.inbox') : t('inbox.emptyFilter')} />
      ) : null}
      <ul className="result-list inbox-list">
        {items.map((item) => {
          const late = typeof item.params.lateSeconds === 'number' ? Math.round(item.params.lateSeconds / 60) : 0
          return (
            <li key={item.id} className={item.state === 'UNREAD' ? 'result-hit inbox-row unread' : 'result-hit inbox-row'}>
              <button type="button" className="inbox-open" onClick={() => void open(item)}>
                <span className="inbox-text">
                  {item.state === 'UNREAD' ? <span className="unread-dot" role="img" aria-label={t('inbox.unreadDot')} /> : null}
                  {t(`inbox.kind.${item.kind}`, { actor: item.actor?.name ?? t('inbox.someone') })}
                </span>
                <strong className="inbox-subject">{item.subject.title}</strong>
                <small className="hint">
                  {formatMessageTime(item.createdAt, locale)}
                  {late > 0 ? ` · ${t('inbox.late', { minutes: String(late) })}` : ''}
                </small>
              </button>
              <span className="saved-actions">
                <button type="button" className="icon-button" title={item.state === 'UNREAD' ? t('inbox.markRead') : t('inbox.markUnread')} aria-label={item.state === 'UNREAD' ? t('inbox.markRead') : t('inbox.markUnread')} onClick={() => void run(item, item.state === 'UNREAD' ? 'read' : 'unread')}>{item.state === 'UNREAD' ? '○' : '●'}</button>
                <button type="button" className="icon-button" title={t('inbox.snooze1h')} aria-label={`${t('inbox.snooze')}: ${t('inbox.snooze1h')}`} onClick={() => void run(item, 'snooze', new Date(Date.now() + 3_600_000).toISOString())}>⏰</button>
                <button type="button" className="icon-button" title={t('inbox.snoozeTomorrow')} aria-label={`${t('inbox.snooze')}: ${t('inbox.snoozeTomorrow')}`} onClick={() => void run(item, 'snooze', tomorrowMorning())}>☀</button>
                <button type="button" className="icon-button" title={t('inbox.archive')} aria-label={t('inbox.archive')} onClick={() => void run(item, 'archive')}>▤</button>
              </span>
            </li>
          )
        })}
      </ul>
      {inbox.hasNextPage ? (
        <Button type="button" variant="secondary" disabled={inbox.isFetchingNextPage} onClick={() => void inbox.fetchNextPage()}>{t('inbox.more')}</Button>
      ) : null}
    </div>
  )
}
