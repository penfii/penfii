import { newIdempotencyKey } from '@penfii/api-client'
import type { Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { AssigneeField, DueDateField, LabelField, MilestoneField, PriorityField, PropRow, SprintField, TypeField } from './fields'
import { EstimateRow } from './PlanFields'
import type { ProjectContext } from './projectContext'
import type { TicketPatch } from './useTicketEditor'

/** The fields a person changes most, one click each. Each change is saved as it is made. */
export function TicketProperties({
  ticket,
  ctx,
  canEdit,
  busy,
  onSave,
  onStatus,
}: {
  ticket: Ticket
  ctx: ProjectContext
  canEdit: boolean
  busy: boolean
  onSave: (patch: TicketPatch) => void
  onStatus: (statusId: string) => void
}) {
  const { t } = useI18n()
  const disabled = !canEdit || busy
  return (
    <div className="prop-grid">
      <PropRow label={t('work.field.status')}>
        <select aria-label={t('work.field.status')} value={ticket.statusId} disabled={disabled} onChange={(event) => onStatus(event.target.value)}>
          {ctx.board.states.map((state) => (
            <option key={state.id} value={state.id}>{state.name}</option>
          ))}
        </select>
      </PropRow>
      <PropRow label={t('work.field.assignee')}>
        <AssigneeField value={ticket.assigneeId} fallbackName={ticket.assigneeName} members={ctx.members} disabled={disabled} onChange={(assigneeId) => onSave({ assigneeId })} />
      </PropRow>
      <PropRow label={t('work.field.priority')}>
        <PriorityField value={ticket.priority} disabled={disabled} onChange={(priority) => onSave({ priority })} />
      </PropRow>
      <PropRow label={t('work.field.dueOn')}>
        <DueDateField value={ticket.dueOn} disabled={disabled} onChange={(dueOn) => onSave({ dueOn })} />
        <p className="field-hint">{t('work.due.hint')}</p>
      </PropRow>
      <EstimateRow ticket={ticket} unit={ctx.board.estimateUnit ?? 'NONE'} disabled={disabled} onSave={onSave} />
      <PropRow label={t('work.field.type')}>
        <TypeField value={ticket.ticketType?.id ?? null} types={ctx.types} disabled={disabled} onChange={(typeId) => onSave({ typeId })} />
      </PropRow>
      <PropRow label={t('work.field.labels')}>
        <LabelField
          projectId={ticket.projectId}
          labels={ctx.labels}
          selected={ticket.labels.map((label) => label.id)}
          disabled={disabled}
          canCreate={canEdit}
          onChange={(labelIds) => onSave({ labelIds })}
        />
      </PropRow>
    </div>
  )
}

/** Sprint and Milestone of a Ticket: Project planning, kept apart from the Ticket's own properties. */
export function TicketPlanning({
  ticket,
  ctx,
  canEdit,
  busy,
  onPlan,
}: {
  ticket: Ticket
  ctx: ProjectContext
  canEdit: boolean
  busy: boolean
  onPlan: (action: () => Promise<Ticket>) => void
}) {
  const { t } = useI18n()
  const disabled = !canEdit || busy
  return (
    <section className="ticket-section planning">
      <h3>{t('work.planning.title')}</h3>
      <div className="prop-grid">
      <PropRow label={t('work.field.sprint')}>
        <SprintField ticket={ticket} ctx={ctx} disabled={disabled} onChange={(sprintId) => onPlan(() => api.assignSprint(ticket.id, sprintId, newIdempotencyKey()))} />
      </PropRow>
      <PropRow label={t('work.field.milestones')}>
        <MilestoneField
          ticket={ticket}
          ctx={ctx}
          disabled={disabled}
          onChange={(ids) => onPlan(() => api.setTicketMilestones(ticket.id, ids, newIdempotencyKey()))}
        />
      </PropRow>
      </div>
    </section>
  )
}
