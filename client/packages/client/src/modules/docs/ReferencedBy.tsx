import { useInfiniteQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'

/** Docs that reference a Ticket. Sources the viewer cannot read are absent. */
export function ReferencedBy({ ticketId, workspaceId }: { ticketId: string; workspaceId: string }) {
  return (
    <ReferenceList
      workspaceId={workspaceId}
      labelKey="docs.refs.title"
      emptyKey="docs.refs.empty"
      queryKey={queryKeys.ticketReferences(ticketId)}
      queryFn={(cursor) => api.getTicketReferences(ticketId, cursor)}
    />
  )
}

/** Docs that reference this Doc. Tickets and chat are not sources. */
export function DocReferencedBy({ docId, workspaceId }: { docId: string; workspaceId: string }) {
  return (
    <ReferenceList
      workspaceId={workspaceId}
      labelKey="docs.incoming.title"
      emptyKey="docs.incoming.empty"
      queryKey={queryKeys.docReferences(docId)}
      queryFn={(cursor) => api.getDocReferences(docId, cursor)}
    />
  )
}

function ReferenceList({
  workspaceId,
  labelKey,
  emptyKey,
  queryKey,
  queryFn,
}: {
  workspaceId: string
  labelKey: 'docs.refs.title' | 'docs.incoming.title'
  emptyKey: 'docs.refs.empty' | 'docs.incoming.empty'
  queryKey: readonly unknown[]
  queryFn: (cursor?: string) => Promise<{ items: Array<{ sourceKind: string; sourceId: string; title: string; context: { parentTitlePath?: string[] }; updatedAt: string }>; nextCursor: string | null }>
}) {
  const { t, locale } = useI18n()
  const refs = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => queryFn(pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  })
  const items = refs.data?.pages.flatMap((p) => p.items) ?? []
  return (
    <section className="referenced-by" aria-label={t(labelKey)}>
      <h3>{t(labelKey)}</h3>
      {refs.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {refs.isError ? (
        <p className="board-error" role="alert">
          {t('docs.refs.error')} <Button variant="ghost" onClick={() => void refs.refetch()}>{t('docs.refs.retry')}</Button>
        </p>
      ) : null}
      {refs.isSuccess && items.length === 0 ? <p className="hint">{t(emptyKey)}</p> : null}
      <ul className="result-list">
        {items.map((item) => {
          const href = `/w/${workspaceId}/docs?doc=${item.sourceId}`
          const where = (item.context.parentTitlePath ?? []).join(' / ')
          return (
            <li key={`${item.sourceKind}:${item.sourceId}`} className="result-hit">
              <Link to={href}>
                <small className="where-chip">{t('docs.refs.doc')}</small> <strong>{item.title || t('docs.untitled')}</strong>
                {where ? <small className="hint"> · {where}</small> : null}
                <small className="hint"> · {formatMessageTime(item.updatedAt, locale)}</small>
              </Link>
            </li>
          )
        })}
      </ul>
      {refs.hasNextPage ? <Button variant="secondary" disabled={refs.isFetchingNextPage} onClick={() => void refs.fetchNextPage()}>{t('docs.refs.more')}</Button> : null}
    </section>
  )
}
