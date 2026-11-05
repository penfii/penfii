import { useState } from 'react'
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { RevisionSummary, Ticket, TicketField } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { ChangeLine, fieldLabelKey } from './ChangeLine'
import { refreshTicket } from './refresh'
import { errorCode, errorDetails, sortFields } from './ticketModel'

const PAGE = 20

/**
 * History lists the saved versions (revisions), newest first. A version's values are opened
 * one at a time, so a Ticket with hundreds of versions stays quick to read.
 */
export function HistoryTab({ ticket, canEdit }: { ticket: Ticket; canEdit: boolean }) {
  const { t, locale } = useI18n()
  const [openRevision, setOpenRevision] = useState<number | null>(null)
  const history = useInfiniteQuery({
    queryKey: queryKeys.ticketHistory(ticket.id),
    queryFn: ({ pageParam }) => api.getTicketRevisions(ticket.id, { limit: PAGE, before: pageParam }),
    initialPageParam: null as number | null,
    getNextPageParam: (last) => last.nextBefore ?? undefined,
  })
  if (history.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (history.isError) {
    return (
      <p className="hint" role="alert">
        {t('work.history.loadError')} <Button variant="ghost" onClick={() => void history.refetch()}>{t('message.retry')}</Button>
      </p>
    )
  }
  const items = history.data.pages.flatMap((page) => page.items)
  if (items.length === 0) return <p className="hint">{t('work.history.empty')}</p>
  return (
    <div className="history">
      <p className="hint">{t('work.history.lead')}</p>
      <ol className="history-list">
        {items.map((revision) => (
          <HistoryRow
            key={revision.revisionNo}
            ticket={ticket}
            revision={revision}
            current={revision.revisionNo === ticket.revisionNo}
            open={openRevision === revision.revisionNo}
            canEdit={canEdit}
            onToggle={() => setOpenRevision(openRevision === revision.revisionNo ? null : revision.revisionNo)}
            locale={locale}
          />
        ))}
      </ol>
      {history.hasNextPage ? (
        <Button variant="secondary" disabled={history.isFetchingNextPage} onClick={() => void history.fetchNextPage()}>
          {history.isFetchingNextPage ? t('loading') : t('work.history.more')}
        </Button>
      ) : null}
    </div>
  )
}

function HistoryRow({
  ticket,
  revision,
  current,
  open,
  canEdit,
  onToggle,
  locale,
}: {
  ticket: Ticket
  revision: RevisionSummary
  current: boolean
  open: boolean
  canEdit: boolean
  onToggle: () => void
  locale: string
}) {
  const { t } = useI18n()
  const fields = sortFields(revision.changedFields)
  return (
    <li className={open ? 'history-item open' : 'history-item'}>
      <div className="history-row">
        <strong className="history-version">{t('work.history.version', { revision: String(revision.revisionNo) })}</strong>
        {current ? <span className="current-chip">{t('work.history.current')}</span> : null}
        <span className="history-summary">
          {revision.revisionNo === 1
            ? t('work.history.created')
            : revision.restoreSourceRevisionNo !== null
              ? t('work.history.restoredFrom', { revision: String(revision.restoreSourceRevisionNo) })
              : fields.map((field) => t(fieldLabelKey(field))).join(', ')}
        </span>
        <small className="history-meta">
          {revision.actorName ?? t('work.actor.unknown')}
          {revision.via ? ` ${t('work.activity.via', { client: revision.via })}` : ''} · {formatMessageTime(revision.recordedAt, locale)}
        </small>
        <Button variant="ghost" aria-expanded={open} onClick={onToggle}>
          {open ? t('work.history.close') : t('work.history.open')}
        </Button>
      </div>
      {open ? <RevisionView ticket={ticket} revisionNo={revision.revisionNo} canEdit={canEdit} /> : null}
    </li>
  )
}

function RevisionView({ ticket, revisionNo, canEdit }: { ticket: Ticket; revisionNo: number; canEdit: boolean }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const detail = useQuery(queries.ticketRevision(ticket.id, revisionNo))
  const [selected, setSelected] = useState<Set<TicketField>>(() => new Set())
  const [result, setResult] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null)
  const restoring = useMutation({
    mutationFn: () => api.restoreTicket(ticket.id, { expectedRevision: ticket.revisionNo, sourceRevision: revisionNo, fields: [...selected] }, newIdempotencyKey()),
    onSuccess: async (restored) => {
      queryClient.setQueryData(queryKeys.ticket(ticket.id), restored)
      await refreshTicket(queryClient, ticket.id, ticket.projectId)
      setSelected(new Set())
      setResult(
        restored.revisionNo === ticket.revisionNo
          ? { tone: 'ok', text: t('work.history.restoreNoChange') }
          : { tone: 'ok', text: t('work.history.restored', { revision: String(restored.revisionNo) }) },
      )
    },
    onError: async (cause) => {
      const code = errorCode(cause)
      if (code === 'restore_not_applicable') {
        const rows = errorDetails(cause).fields
        const reasons = Array.isArray(rows)
          ? rows.flatMap((row: unknown) => {
              const item = row as { field?: unknown; reason?: unknown }
              if (typeof item.field !== 'string' || typeof item.reason !== 'string') return []
              const reasonKey = `work.restore.reason.${item.reason}` as const
              const known = ['assignee_unavailable', 'label_unavailable', 'type_unavailable', 'invalid_value'].includes(item.reason)
              return [`${t(fieldLabelKey(item.field as TicketField))}: ${known ? t(reasonKey as 'work.restore.reason.invalid_value') : item.reason}`]
            })
          : []
        setResult({ tone: 'error', text: reasons.join(' · ') || t('work.history.restoreError') })
      } else {
        if (code === 'version_conflict') await refreshTicket(queryClient, ticket.id, ticket.projectId)
        setResult({ tone: 'error', text: code === 'version_conflict' ? t('work.move.conflict') : t('work.history.restoreError') })
      }
    },
  })
  const busy = restoring.isPending

  if (detail.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (detail.isError) return <p className="hint" role="alert">{t('work.history.detailError')}</p>
  const data = detail.data
  const restorable = new Map(data.restorable.map((item) => [item.field, item]))

  function toggle(field: TicketField) {
    setSelected((items) => {
      const next = new Set(items)
      if (next.has(field)) next.delete(field)
      else next.add(field)
      return next
    })
  }

  function restore() {
    if (selected.size === 0 || busy) return
    setResult(null)
    restoring.mutate()
  }

  return (
    <div className="revision-view">
      {revisionNo > 1 ? (
        <section>
          <h4>{t('work.history.fromPrevious')}</h4>
          {data.changesFromPrevious.length === 0 ? <p className="hint">{t('work.history.nothingChanged')}</p> : data.changesFromPrevious.map((change) => <ChangeLine key={change.field} change={change} />)}
        </section>
      ) : null}
      {revisionNo !== ticket.revisionNo ? (
        <section>
          <h4>{t('work.history.vsCurrent')}</h4>
          <p className="hint">{t('work.history.vsCurrentHint')}</p>
          {data.changesToCurrent.length === 0 ? <p className="hint">{t('work.history.noDifference')}</p> : null}
          {data.changesToCurrent.map((change) => {
            const entry = restorable.get(change.field)
            const reason = entry && !entry.applicable ? entry.reason : null
            return (
              <div key={change.field} className="restore-row">
                {canEdit && entry ? (
                  <input
                    type="checkbox"
                    aria-label={t(fieldLabelKey(change.field))}
                    disabled={!entry.applicable || busy}
                    checked={selected.has(change.field)}
                    onChange={() => toggle(change.field)}
                  />
                ) : (
                  <span className="restore-spacer" aria-hidden="true" />
                )}
                <div>
                  <ChangeLine change={change} />
                  {reason ? <small className="restore-reason">{t(`work.restore.reason.${reason}` as 'work.restore.reason.invalid_value')}</small> : null}
                  {!entry ? <small className="restore-reason">{t('work.history.notRestorable')}</small> : null}
                </div>
              </div>
            )
          })}
          {canEdit ? (
            data.restorable.length > 0 ? (
              <div className="restore-actions">
                <small className="field-hint">{t('work.history.restoreHelp')}</small>
                <Button disabled={selected.size === 0 || busy} onClick={restore}>
                  {busy ? t('loading') : t('work.history.restore')}
                </Button>
              </div>
            ) : null
          ) : (
            <small className="field-hint">{t('work.history.readOnly')}</small>
          )}
          {result ? <p className={result.tone === 'ok' ? 'restore-result' : 'board-error'} role={result.tone === 'ok' ? 'status' : 'alert'}>{result.text}</p> : null}
        </section>
      ) : null}
    </div>
  )
}
