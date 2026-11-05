import type { TableColumn, ViewSortField, WorkflowState } from '@penfii/contracts'
import { Avatar } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { StateMenu } from './StateMenu'
import { formatEstimate } from './planValues'
import { formatDue, isOverdue, isPriority } from './ticketModel'
import { BlockedChip } from './WorkChips'
import type { Section, WorkTicket } from './viewModel'

const SORTABLE: Partial<Record<TableColumn, ViewSortField>> = { TITLE: 'TITLE', PRIORITY: 'PRIORITY', DUE_ON: 'DUE_ON', ESTIMATE: 'ESTIMATE' }

/**
 * The Table layout: the same Tickets as columns of properties, for comparing and for choosing
 * several at once. Selecting is the only way into a bulk change; each cell is a read, and a State
 * is changed from its own menu.
 */
export function TableLayout({
  sections,
  columns,
  sort,
  selectedId,
  selection,
  compact,
  states,
  busy,
  canEdit,
  onSort,
  onSelect,
  onOpen,
  onState,
}: {
  sections: Section[]
  columns: TableColumn[]
  sort: { field: ViewSortField; dir: 'ASC' | 'DESC' }
  selectedId: string | null
  selection: Set<string>
  compact: boolean
  states?: WorkflowState[]
  busy: Set<string>
  canEdit: (ticket: WorkTicket) => boolean
  onSort: (field: ViewSortField) => void
  onSelect: (ids: string[], on: boolean) => void
  onOpen: (id: string) => void
  onState: (ticket: WorkTicket, stateId: string) => void
}) {
  const { t, locale } = useI18n()
  // Only Tickets the person can edit can be chosen for a change; a read-only table has no checkboxes.
  const all = sections.flatMap((section) => section.tickets).filter(canEdit)
  const selectable = all.length > 0
  const allSelected = all.length > 0 && all.every((ticket) => selection.has(ticket.id))
  const span = columns.length + 1

  function cell(column: TableColumn, ticket: WorkTicket) {
    switch (column) {
      case 'KEY':
        return (
          <>
            <small className="ticket-key">{ticket.key}</small>
            {ticket.hierarchyKind !== 'TASK' ? <span className={`kind-chip kind-${ticket.hierarchyKind.toLowerCase()}`}>{t(`work.kind.${ticket.hierarchyKind}`)}</span> : null}
          </>
        )
      case 'TITLE':
        return (
          <button type="button" className="table-open" aria-current={ticket.id === selectedId ? 'true' : undefined} aria-label={t('work.board.openTicket', { key: ticket.key })} onClick={() => onOpen(ticket.id)}>
            {ticket.parentId ? <span aria-hidden="true" className="table-nest">↳ </span> : null}
            {ticket.title}
            <BlockedChip ticket={ticket} />
          </button>
        )
      case 'STATE':
        return <StateMenu ticket={ticket} known={states} disabled={!canEdit(ticket) || busy.has(ticket.id)} onPick={(id) => onState(ticket, id)} />
      case 'ASSIGNEE':
        return ticket.assigneeName ? (
          <span className="ticket-assignee" title={ticket.assigneeName}>
            <Avatar name={ticket.assigneeName} />
            <span>{ticket.assigneeName}</span>
          </span>
        ) : null
      case 'PRIORITY':
        return isPriority(ticket.priority) ? <span className={`priority-chip priority-${ticket.priority.toLowerCase()}`}>{t(`work.priority.${ticket.priority}`)}</span> : null
      case 'LABELS':
        return ticket.labels.map((label) => (
          <span key={label.id} className="label-chip">{label.name}</span>
        ))
      case 'PROJECT':
        return ticket.projectName
      case 'CHANNEL':
        return ticket.channelName ? `# ${ticket.channelName}` : null
      case 'SPRINT':
        return ticket.sprint ? <span className="sprint-chip">{ticket.sprint.name}</span> : null
      case 'MILESTONE':
        return ticket.milestones.map((milestone) => (
          <span key={milestone.id} className="label-chip">{milestone.name}</span>
        ))
      case 'PARENT':
        return ticket.parent ? (
          <span title={ticket.parent.title}>
            <small className="ticket-key">{ticket.parent.key}</small>
          </span>
        ) : null
      case 'ESTIMATE':
        return ticket.estimate ? <span className="estimate-chip">{formatEstimate(ticket.estimate)}</span> : null
      default: {
        if (!ticket.dueOn) return null
        const done = ticket.stateCategory === 'DONE' || ticket.stateCategory === 'CANCELED'
        return (
          <time dateTime={ticket.dueOn} className={isOverdue(ticket.dueOn, done) ? 'ticket-due overdue' : 'ticket-due'}>
            {formatDue(ticket.dueOn, locale)}
          </time>
        )
      }
    }
  }

  return (
    <div className={compact ? 'work-table-wrap compact' : 'work-table-wrap'}>
      <table className="work-table">
        <thead>
          <tr>
            <th className="table-check">
              {selectable ? <input type="checkbox" aria-label={t('work.table.selectAll')} checked={allSelected} onChange={(event) => onSelect(all.map((ticket) => ticket.id), event.target.checked)} /> : null}
            </th>
            {columns.map((column) => {
              const field = SORTABLE[column]
              const active = field !== undefined && sort.field === field
              return (
                <th key={column} scope="col" aria-sort={active ? (sort.dir === 'ASC' ? 'ascending' : 'descending') : undefined}>
                  {field ? (
                    <button type="button" className="table-sort" onClick={() => onSort(field)}>
                      {t(`view.column.${column}`)}
                      <span aria-hidden="true">{active ? (sort.dir === 'ASC' ? ' ↑' : ' ↓') : ''}</span>
                    </button>
                  ) : (
                    t(`view.column.${column}`)
                  )}
                </th>
              )
            })}
          </tr>
        </thead>
        {sections.map((section) => (
          <tbody key={section.id}>
            {section.title ? (
              <tr className="table-group">
                <th colSpan={span} scope="colgroup">
                  {section.tickets.some(canEdit) ? (
                    <input
                      type="checkbox"
                      aria-label={t('work.table.selectGroup', { title: section.title })}
                      checked={section.tickets.filter(canEdit).every((ticket) => selection.has(ticket.id))}
                      onChange={(event) => onSelect(section.tickets.filter(canEdit).map((ticket) => ticket.id), event.target.checked)}
                    />
                  ) : null}{' '}
                  {section.title}
                  {section.hint ? <small className="hint"> · {section.hint}</small> : null}
                  <span className="list-count">{section.tickets.length}</span>
                </th>
              </tr>
            ) : null}
            {section.tickets.map((ticket) => (
              <tr key={ticket.id} data-ticket-id={ticket.id} className={[ticket.id === selectedId ? 'selected' : '', selection.has(ticket.id) ? 'checked' : ''].filter(Boolean).join(' ')} aria-busy={busy.has(ticket.id)}>
                <td className="table-check">
                  {canEdit(ticket) ? <input type="checkbox" aria-label={t('work.table.select', { key: ticket.key })} checked={selection.has(ticket.id)} onChange={(event) => onSelect([ticket.id], event.target.checked)} /> : null}
                </td>
                {columns.map((column) => (
                  <td key={column} className={`col-${column.toLowerCase()}`}>{cell(column, ticket)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        ))}
      </table>
    </div>
  )
}
