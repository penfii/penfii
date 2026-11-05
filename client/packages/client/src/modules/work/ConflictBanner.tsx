import type { Ticket } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { fieldLabelKey, formatChangeValue } from './ChangeLine'
import { estimateAsChange, scheduleAsChange } from './planValues'
import type { ProjectContext } from './projectContext'
import { descriptionText, allTickets } from './ticketModel'
import type { TicketConflict } from './useTicketEditor'

type Row = { field: Parameters<typeof fieldLabelKey>[0]; mine: string | string[] | null; latest: string | string[] | null }

function rows(conflict: TicketConflict, ctx: ProjectContext): Row[] {
  const { patch, latest } = conflict
  const out: Row[] = []
  if (patch.title !== undefined) out.push({ field: 'title', mine: patch.title, latest: latest.title })
  if (patch.description !== undefined) {
    out.push({ field: 'description', mine: descriptionText(patch.description) || null, latest: descriptionText(latest.description) || null })
  }
  if (patch.assigneeId !== undefined) {
    const mine = patch.assigneeId ? (ctx.members.find((member) => member.userId === patch.assigneeId)?.displayName ?? patch.assigneeId) : null
    out.push({ field: 'assignee', mine, latest: latest.assigneeName })
  }
  if (patch.priority !== undefined) out.push({ field: 'priority', mine: patch.priority, latest: latest.priority })
  if (patch.dueOn !== undefined) out.push({ field: 'dueOn', mine: patch.dueOn, latest: latest.dueOn })
  if (patch.labelIds !== undefined) {
    const names = patch.labelIds.map((id) => ctx.labels.find((label) => label.id === id)?.name ?? id)
    out.push({ field: 'labels', mine: names, latest: latest.labels.map((label) => label.name) })
  }
  if (patch.parentId !== undefined) {
    const mine = patch.parentId ? (allTickets(ctx.board).find((item) => item.id === patch.parentId)?.key ?? patch.parentId) : null
    out.push({ field: 'parent', mine, latest: latest.parent?.key ?? null })
  }
  if (patch.typeId !== undefined) {
    out.push({ field: 'type', mine: patch.typeId ? (ctx.types.find((item) => item.id === patch.typeId)?.name ?? patch.typeId) : null, latest: latest.ticketType?.name ?? null })
  }
  if (patch.estimate !== undefined) {
    out.push({ field: 'estimate', mine: estimateAsChange(patch.estimate, ctx.board.estimateUnit ?? 'POINTS'), latest: estimateAsChange(latest.estimate, ctx.board.estimateUnit ?? 'POINTS') })
  }
  if (patch.schedule !== undefined) out.push({ field: 'schedule', mine: scheduleAsChange(patch.schedule), latest: scheduleAsChange(latest.schedule) })
  return out
}

/**
 * Shown when someone else saved first. The attempted change is kept; the person sees it next
 * to the latest value and chooses to re-apply it on top or drop it. Nothing is overwritten silently.
 */
export function ConflictBanner({
  conflict,
  ticket,
  ctx,
  busy,
  onReapply,
  onDiscard,
}: {
  conflict: TicketConflict
  ticket: Ticket
  ctx: ProjectContext
  busy: boolean
  onReapply: () => void
  onDiscard: () => void
}) {
  const { t, locale } = useI18n()
  return (
    <section className="conflict-banner" role="alert" aria-label={t('work.conflict.title')}>
      <h3>{t('work.conflict.title')}</h3>
      <p>{t('work.conflict.body')}</p>
      {conflict.again ? <p className="conflict-again">{t('work.conflict.stillConflict')}</p> : null}
      <table className="conflict-table">
        <thead>
          <tr>
            <th />
            <th>{t('work.conflict.mine')}</th>
            <th>{t('work.conflict.latest')} · v{ticket.revisionNo}</th>
          </tr>
        </thead>
        <tbody>
          {rows(conflict, ctx).map((row) => (
            <tr key={row.field}>
              <th scope="row">{t(fieldLabelKey(row.field))}</th>
              <td>{formatChangeValue(row.field, row.mine, t, locale)}</td>
              <td>{formatChangeValue(row.field, row.latest, t, locale)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="dialog-actions">
        <Button type="button" variant="secondary" disabled={busy} onClick={onDiscard}>{t('work.conflict.discard')}</Button>
        <Button type="button" disabled={busy} onClick={onReapply}>{t('work.conflict.apply')}</Button>
      </div>
    </section>
  )
}
