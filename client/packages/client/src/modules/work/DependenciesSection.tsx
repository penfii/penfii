import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { DependencyEdge, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import type { ProjectContext } from './projectContext'
import { refreshTicket } from './refresh'
import { allTickets, errorCode } from './ticketModel'
import { CloseIcon } from '../../design/icons'

type Failure = 'cycle' | 'exists' | 'invalid' | 'forbidden' | 'generic' | 'reason' | null

function failureOf(cause: unknown): Failure {
  const code = errorCode(cause)
  if (code === 'dependency_cycle') return 'cycle'
  if (code === 'dependency_exists') return 'exists'
  if (code === 'forbidden') return 'forbidden'
  if (code === 'validation' || code === 'not_found') return 'invalid'
  if (code === 'invalid_waiver_state' || code === 'dependency_satisfied') return 'generic'
  return 'generic'
}

/** Tickets that must finish before this one starts, and the ones waiting for it. */
export function DependenciesSection({ ticket, ctx, onOpen }: { ticket: Ticket; ctx: ProjectContext; onOpen: (id: string) => void }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const query = useQuery(queries.ticketDependencies(ticket.id))
  const [pick, setPick] = useState('')
  const [pickRelated, setPickRelated] = useState('')
  const [waiving, setWaiving] = useState<DependencyEdge | null>(null)
  const [reason, setReason] = useState('')
  const [failure, setFailure] = useState<Failure>(null)
  const mutation = useMutation({
    mutationFn: (action: () => Promise<unknown>) => action(),
    onError: (cause) => setFailure(failureOf(cause)),
    onSettled: () => refreshTicket(queryClient, ticket.id, ticket.projectId),
  })
  const busy = mutation.isPending

  function change(action: () => Promise<unknown>): Promise<boolean> {
    setFailure(null)
    return mutation.mutateAsync(action).then(() => true, () => false)
  }

  if (query.isPending) return null
  if (query.isError) return <p className="hint" role="alert">{t('work.dep.loadError')}</p>
  const deps = query.data
  const canEdit = deps.canEdit && ctx.board.canEdit && !ticket.archivedAt
  const taken = new Set([ticket.id, ...deps.blockedBy.map((edge) => edge.ticket.id)])
  const tickets = allTickets(ctx.board)
  const byId = new Map(tickets.map((item) => [item.id, item]))
  const family = new Set<string>()
  for (let up = ticket.parentId ? byId.get(ticket.parentId) : undefined; up; up = up.parentId ? byId.get(up.parentId) : undefined) family.add(up.id)
  for (const item of tickets) {
    for (let up = item.parentId ? byId.get(item.parentId) : undefined; up; up = up.parentId ? byId.get(up.parentId) : undefined) {
      if (up.id === ticket.id) family.add(item.id)
    }
  }
  const candidates = tickets.filter((item) => !taken.has(item.id) && !family.has(item.id))
  const linked = new Set([ticket.id, ...deps.blockedBy.map((edge) => edge.ticket.id), ...deps.blocks.map((edge) => edge.ticket.id), ...deps.related.map((edge) => edge.ticket.id)])
  const relatedCandidates = tickets.filter((item) => !linked.has(item.id))
  const canceled = deps.blockedBy.filter((edge) => edge.predecessorCanceled && !edge.satisfied)
  if (deps.blockedBy.length === 0 && deps.blocks.length === 0 && deps.related.length === 0 && !canEdit) return null

  return (
    <section className="ticket-section dependencies">
      <h3>{t('work.dep.title')}</h3>
      {failure ? <p className="board-error" role="alert">{t(`work.dep.error.${failure}`)}</p> : null}
      {canceled.length > 0 ? <p className="board-note" role="note">{t('work.dep.canceledNote', { keys: canceled.map((edge) => edge.ticket.key).join(', ') })}</p> : null}
      <EdgeList
        title={t('work.dep.blockedBy')}
        edges={deps.blockedBy}
        onOpen={onOpen}
        onRemove={canEdit ? (edge) => void change(() => api.removeDependency(edge.id)) : undefined}
        onWaive={deps.canWaive ? (edge) => { setReason(''); setWaiving(edge) } : undefined}
        onRestore={deps.canWaive ? (edge) => { setReason(''); setWaiving(edge) } : undefined}
        busy={busy}
      />
      {waiving ? (
        <form
          className="dep-waive"
          onSubmit={(event) => {
            event.preventDefault()
            if (!reason.trim()) return
            const edge = waiving
            void change(() => (edge.waived ? api.restoreDependency(edge.id, reason.trim(), newIdempotencyKey()) : api.waiveDependency(edge.id, reason.trim(), newIdempotencyKey()))).then((ok) => {
              if (ok) setWaiving(null)
            })
          }}
        >
          <label className="field">
            <span>{t(waiving.waived ? 'work.dep.restoreReason' : 'work.dep.waiveReason', { key: waiving.ticket.key })}</span>
            <input autoFocus maxLength={300} value={reason} onChange={(event) => setReason(event.target.value)} />
          </label>
          <small className="field-hint">{t(waiving.waived ? 'work.dep.restoreHint' : 'work.dep.waiveHint')}</small>
          <div className="dialog-actions">
            <Button type="button" variant="ghost" onClick={() => setWaiving(null)}>{t('view.cancel')}</Button>
            <Button type="submit" disabled={!reason.trim() || busy}>{t(waiving.waived ? 'work.dep.restore' : 'work.dep.waive')}</Button>
          </div>
        </form>
      ) : null}
      {canEdit ? (
        <div className="dep-add">
          <select aria-label={t('work.dep.pick')} value={pick} disabled={busy} onChange={(event) => setPick(event.target.value)}>
            <option value="">{candidates.length === 0 ? t('work.dep.noCandidates') : t('work.dep.pick')}</option>
            {candidates.map((item) => (
              <option key={item.id} value={item.id}>{item.key} · {item.title}</option>
            ))}
          </select>
          <Button
            type="button"
            variant="secondary"
            disabled={!pick || busy}
            onClick={() => void change(() => api.addTicketDependency(ticket.id, pick, newIdempotencyKey())).then((ok) => ok && setPick(''))}
          >
            {t('work.dep.add')}
          </Button>
        </div>
      ) : null}
      <small className="field-hint">{t('work.dep.hint')}</small>
      {deps.blocks.length > 0 ? <EdgeList title={t('work.dep.blocks')} edges={deps.blocks} onOpen={onOpen} busy={busy} /> : null}
      <EdgeList title={t('work.dep.related')} edges={deps.related} onOpen={onOpen} onRemove={canEdit ? (edge) => void change(() => api.removeDependency(edge.id)) : undefined} busy={busy} />
      {canEdit ? (
        <div className="dep-add">
          <select aria-label={t('work.dep.pickRelated')} value={pickRelated} disabled={busy} onChange={(event) => setPickRelated(event.target.value)}>
            <option value="">{t('work.dep.pickRelated')}</option>
            {relatedCandidates.map((item) => (
              <option key={item.id} value={item.id}>{item.key} · {item.title}</option>
            ))}
          </select>
          <Button type="button" variant="secondary" disabled={!pickRelated || busy} onClick={() => void change(() => api.addTicketDependency(ticket.id, pickRelated, newIdempotencyKey(), 'RELATED')).then((ok) => ok && setPickRelated(''))}>{t('work.dep.addRelated')}</Button>
        </div>
      ) : null}
    </section>
  )
}

function EdgeList({
  title,
  edges,
  onOpen,
  onRemove,
  onWaive,
  onRestore,
  busy,
}: {
  title: string
  edges: DependencyEdge[]
  onOpen: (id: string) => void
  onRemove?: (edge: DependencyEdge) => void
  onWaive?: (edge: DependencyEdge) => void
  onRestore?: (edge: DependencyEdge) => void
  busy: boolean
}) {
  const { t } = useI18n()
  return (
    <div className="dep-group">
      <h4>{title}</h4>
      {edges.length === 0 ? <p className="hint">{t('work.dep.empty')}</p> : null}
      <ul className="dep-list">
        {edges.map((edge) => (
          <li key={edge.id} className={edge.satisfied ? 'dep-row done' : 'dep-row'}>
            <button type="button" className="child-open" onClick={() => onOpen(edge.ticket.id)} aria-label={t('work.hierarchy.open', { key: edge.ticket.key })}>
              <small className="ticket-key">{edge.ticket.key}</small>
              <span className="child-title">{edge.ticket.title}</span>
            </button>
            <span className={edge.satisfied ? 'dep-state ok' : 'dep-state'}>
              {edge.kind === 'RELATED' ? edge.stateName : `${edge.waived ? t('work.dep.waived') : edge.satisfied ? t('work.dep.done') : edge.predecessorCanceled ? t('work.dep.canceled') : t('work.dep.waiting')} · ${edge.stateName}`}
            </span>
            {edge.waived && edge.waiveReason ? <small className="hint" title={edge.waiveReason}>{t('work.dep.reason', { reason: edge.waiveReason })}</small> : null}
            {edge.needsReview ? <span className="blocked-chip" title={t('work.dep.reviewHelp')}>{t('work.dep.review')}</span> : null}
            {onWaive && edge.kind === 'BLOCKS' && !edge.waived && !edge.satisfied ? (
              <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => onWaive(edge)}>{t('work.dep.waive')}</button>
            ) : null}
            {onRestore && edge.waived ? (
              <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => onRestore(edge)}>{t('work.dep.restore')}</button>
            ) : null}
            {onRemove ? (
              <button type="button" className="icon-button" disabled={busy} aria-label={t(edge.kind === 'RELATED' ? 'work.dep.removeRelated' : 'work.dep.remove', { key: edge.ticket.key })} onClick={() => onRemove(edge)}><CloseIcon /></button>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  )
}
