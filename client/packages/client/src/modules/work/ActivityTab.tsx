import { useInfiniteQuery } from '@tanstack/react-query'
import type { TicketActivity } from '@penfii/contracts'
import { api } from '../../api'
import { Avatar, Button } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { ChangeLine } from './ChangeLine'

const PAGE = 10

type Translate = ReturnType<typeof useI18n>['t']

function sentence(entry: TicketActivity, t: Translate): string {
  const actor = entry.actorName ?? t('work.actor.unknown')
  switch (entry.activityKind) {
    case 'ticket.created':
      return t('work.activity.created', { actor })
    case 'ticket.commented':
      return t('work.activity.commented', { actor })
    case 'ticket.restored': {
      const source = entry.data.sourceRevision
      return t('work.activity.restored', { actor, revision: typeof source === 'number' ? String(source) : '?' })
    }
    case 'ticket.dependency_added':
    case 'ticket.dependency_removed': {
      const key = typeof entry.data.predecessorKey === 'string' ? entry.data.predecessorKey : '?'
      return t(entry.activityKind === 'ticket.dependency_added' ? 'work.activity.dependency_added' : 'work.activity.dependency_removed', { actor, key })
    }
    case 'ticket.sprint_changed': {
      const name = (value: unknown) => (value && typeof value === 'object' && 'name' in value && typeof value.name === 'string' ? value.name : t('work.activity.sprint.none'))
      return t('work.activity.sprint_changed', { actor, from: name(entry.data.from), to: name(entry.data.to) })
    }
    case 'ticket.milestones_changed': {
      const names = (value: unknown) => (Array.isArray(value) ? value.map((item) => (item && typeof item === 'object' && 'name' in item ? String(item.name) : '?')) : [])
      const change = [...names(entry.data.added).map((n) => `+${n}`), ...names(entry.data.removed).map((n) => `−${n}`)].join(' ')
      return t('work.activity.milestones_changed', { actor, change })
    }
    default: {
      if (entry.changes.length === 1) return t(`work.activity.${entry.changes[0].field}`, { actor })
      return t('work.activity.updated', { actor })
    }
  }
}

/**
 * Activity is the plain account of what happened, newest first. It shows the most recent
 * events and loads more on request; the full revision list is in History.
 */
export function ActivityTab({ ticketId }: { ticketId: string }) {
  const { t, locale } = useI18n()
  const activity = useInfiniteQuery({
    queryKey: queryKeys.ticketActivity(ticketId),
    queryFn: ({ pageParam }) => api.getTicketActivity(ticketId, { limit: PAGE, before: pageParam }),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  })
  if (activity.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (activity.isError) {
    return (
      <p className="hint" role="alert">
        {t('work.activity.loadError')} <Button variant="ghost" onClick={() => void activity.refetch()}>{t('message.retry')}</Button>
      </p>
    )
  }
  const items = activity.data.pages.flatMap((page) => page.items)
  if (items.length === 0) return <p className="hint">{t('work.activity.empty')}</p>
  return (
    <div className="activity-feed">
      <ol className="activity-list">
        {items.map((entry) => (
          <li key={entry.id} className="activity-item">
            <Avatar name={entry.actorName ?? '?'} />
            <div className="activity-body">
              <p className="activity-sentence">
                {sentence(entry, t)}
                {entry.via ? <span className="via-chip">{t('work.activity.via', { client: entry.via })}</span> : null}
              </p>
              {entry.changes.map((change) => <ChangeLine key={change.field} change={change} />)}
              <time dateTime={entry.occurredAt} title={new Date(entry.occurredAt).toLocaleString(locale)}>
                {formatMessageTime(entry.occurredAt, locale)}
              </time>
            </div>
          </li>
        ))}
      </ol>
      {activity.hasNextPage ? (
        <Button variant="secondary" disabled={activity.isFetchingNextPage} onClick={() => void activity.fetchNextPage()}>
          {activity.isFetchingNextPage ? t('loading') : t('work.activity.more')}
        </Button>
      ) : null}
    </div>
  )
}
