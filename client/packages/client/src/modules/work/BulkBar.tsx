import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { BulkChange, BulkItemResult, BulkResult } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { PRIORITIES } from './ticketModel'
import type { WorkTicket } from './viewModel'
import { queries } from '../../queries'

export type Field = 'STATUS' | 'ASSIGNEE' | 'PRIORITY' | 'ADD_LABELS' | 'REMOVE_LABELS' | 'SPRINT' | 'ADD_MILESTONES' | 'REMOVE_MILESTONES'

/** Fields that mean something only inside one Project (a State, a Sprint, a label...). */
export const BULK_FIELDS: Field[] = ['STATUS', 'ASSIGNEE', 'PRIORITY', 'ADD_LABELS', 'REMOVE_LABELS', 'SPRINT', 'ADD_MILESTONES', 'REMOVE_MILESTONES']

const ONE_PROJECT: Field[] = ['STATUS', 'ADD_LABELS', 'REMOVE_LABELS', 'SPRINT', 'ADD_MILESTONES', 'REMOVE_MILESTONES']

export function failureKey(item: BulkItemResult): MessageKey | null {
  switch (item.code) {
    case 'dependency_blocked':
      return 'work.move.blocked'
    case 'unfinished_subtasks':
      return 'work.move.unfinished'
    case 'forbidden':
      return 'bulk.fail.forbidden'
    case 'not_found':
      return 'bulk.fail.notFound'
    case 'version_conflict':
      return 'bulk.fail.conflict'
    case 'open_children':
      return 'bulk.fail.open_children'
    case 'ticket_trashed':
      return 'bulk.fail.ticket_trashed'
    case 'not_processed':
      return 'bulk.lifecycle.notProcessed'
    case null:
    default:
      return null
  }
}

/** The choice of one change for the chosen Tickets and its request; the desktop bar and the phone sheet both use it. */
export function useBulkForm(workspaceId: string, selected: WorkTicket[], onResult: (result: BulkResult | null) => void) {
  const { t } = useI18n()
  const projectIds = [...new Set(selected.map((ticket) => ticket.projectId))]
  const single = projectIds.length === 1 ? projectIds[0] : null
  const first = projectIds[0] ?? null
  const [field, setField] = useState<Field>('PRIORITY')
  const [value, setValue] = useState('')
  const bulk = useMutation({
    mutationFn: (body: BulkChange) => api.bulkTickets(workspaceId, selected.map((ticket) => ticket.id), body, newIdempotencyKey()),
    onSuccess: (outcome) => onResult(outcome),
  })

  const workflow = useQuery({ ...queries.projectWorkflow(single ?? ''), enabled: single !== null && field === 'STATUS' })
  const labels = useQuery({ ...queries.projectLabels(single ?? ''), enabled: single !== null && (field === 'ADD_LABELS' || field === 'REMOVE_LABELS') })
  const planning = useQuery({ ...queries.projectPlanning(single ?? ''), enabled: single !== null && (field === 'SPRINT' || field.endsWith('MILESTONES')) })
  const members = useQuery({ ...queries.projectMembers(first ?? ''), enabled: first !== null && field === 'ASSIGNEE' })

  const locked = single === null && ONE_PROJECT.includes(field)
  const options: Array<{ value: string; label: string }> =
    field === 'STATUS'
      ? (workflow.data?.states ?? []).map((state) => ({ value: state.id, label: state.name }))
      : field === 'ASSIGNEE'
        ? [{ value: '', label: t('bulk.value.unassign') }, ...(members.data ?? []).map((member) => ({ value: member.userId, label: member.displayName }))]
        : field === 'PRIORITY'
          ? [{ value: '', label: t('bulk.value.noPriority') }, ...PRIORITIES.map((priority) => ({ value: priority, label: t(`work.priority.${priority}`) }))]
          : field === 'ADD_LABELS' || field === 'REMOVE_LABELS'
            ? (labels.data ?? []).map((label) => ({ value: label.id, label: label.name }))
            : field === 'SPRINT'
              ? [
                  { value: '', label: t('bulk.value.backlog') },
                  ...(planning.data?.sprints ?? []).filter((sprint) => sprint.state === 'PLANNED' || sprint.state === 'ACTIVE').map((sprint) => ({ value: sprint.id, label: sprint.name })),
                ]
              : (planning.data?.milestones ?? []).map((milestone) => ({ value: milestone.id, label: milestone.name }))
  // "No assignee", "no priority" and "backlog" are real choices with an empty value.
  const needsValue = !['ASSIGNEE', 'PRIORITY', 'SPRINT'].includes(field)
  const ready = !locked && selected.length > 0 && (!needsValue || value !== '')

  function change(): BulkChange {
    switch (field) {
      case 'STATUS':
        return { type: 'STATUS', statusId: value }
      case 'ASSIGNEE':
        return { type: 'ASSIGNEE', assigneeId: value || null }
      case 'PRIORITY':
        return { type: 'PRIORITY', priority: value || null }
      case 'SPRINT':
        return { type: 'SPRINT', sprintId: value || null }
      case 'ADD_LABELS':
      case 'REMOVE_LABELS':
        return { type: field, labelIds: [value] }
      case 'ADD_MILESTONES':
      case 'REMOVE_MILESTONES':
        return { type: field, milestoneIds: [value] }
    }
  }

  function apply() {
    if (!ready || bulk.isPending) return
    onResult(null)
    bulk.mutate(change())
  }

  return { field, setField, value, setValue, options, locked, needsValue, ready, apply, bulk }
}

/**
 * One change for the Tickets chosen in a Table. Every Ticket goes through the same command a
 * single edit uses, so a Ticket that may not take the change is refused with its own reason while
 * the others still change; the result is never summed into a plain "done".
 */
export function BulkBar({
  workspaceId,
  selected,
  result,
  names,
  onClear,
  onResult,
}: {
  workspaceId: string
  selected: WorkTicket[]
  /** The last result stays on screen after the selection is gone, so a full success is still reported. */
  result: BulkResult | null
  /** Key and title of every Ticket the result may mention. */
  names: Map<string, { key: string; title: string }>
  onClear: () => void
  /** Called with the result, so the page can refresh and keep the failed Tickets selected. */
  onResult: (result: BulkResult | null) => void
}) {
  const { t } = useI18n()
  const { field, setField, value, setValue, options, locked, needsValue, ready, apply, bulk } = useBulkForm(workspaceId, selected, onResult)
  return (
    <div className="bulk-bar" role="region" aria-label={t('bulk.title')}>
      <div className="bulk-controls">
        <strong>{selected.length > 0 ? t('bulk.selected', { count: String(selected.length) }) : t('bulk.title')}</strong>
        <label className="view-select">
          <span>{t('bulk.field')}</span>
          <select value={field} onChange={(event) => { setField(event.target.value as Field); setValue('') }}>
            {BULK_FIELDS.map((item) => (
              <option key={item} value={item}>{t(`bulk.field.${item}`)}</option>
            ))}
          </select>
        </label>
        <label className="view-select">
          <span>{t('bulk.value')}</span>
          <select value={value} disabled={locked} onChange={(event) => setValue(event.target.value)}>
            {needsValue ? <option value="">{t('bulk.value.pick')}</option> : null}
            {options.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
        </label>
        <Button type="button" disabled={!ready || bulk.isPending} onClick={apply}>{t('bulk.apply')}</Button>
        <Button type="button" variant="ghost" onClick={onClear}>{t('bulk.clear')}</Button>
      </div>
      {locked ? <small className="field-hint">{t('bulk.oneProject')}</small> : null}
      {bulk.isError ? <p className="board-error" role="alert">{t('bulk.error')}</p> : null}
      <BulkResultView result={result} names={names} />
    </div>
  )
}

/** The outcome of the last change: a count, and a reason for every Ticket that was refused. */
export function BulkResultView({ result, names }: { result: BulkResult | null; names: Map<string, { key: string; title: string }> }) {
  const { t } = useI18n()
  if (!result) return null
  const failed = result.items.filter((item) => !item.ok)
  return (
        <div className={failed.length > 0 ? 'bulk-result has-failures' : 'bulk-result'} role="status">
          <strong>{t('bulk.result', { total: String(result.total), ok: String(result.succeeded), failed: String(result.failed) })}</strong>
          {failed.length > 0 ? (
            <ul className="bulk-failures">
              {failed.map((item) => {
                const ticket = names.get(item.ticketId)
                const key = failureKey(item)
                return (
                  <li key={item.ticketId}>
                    <small className="ticket-key">{ticket?.key ?? ''}</small> {ticket?.title ?? ''}
                    <span className="bulk-reason"> — {key ? t(key) : (item.message ?? item.code ?? '')}</span>
                  </li>
                )
              })}
            </ul>
          ) : null}
        </div>
  )
}
