import type { KeyboardEvent } from 'react'
import type { WorkflowState } from '@penfii/contracts'
import { Avatar } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { StateMenu } from './StateMenu'
import { formatDue, isOverdue, isPriority } from './ticketModel'
import { formatEstimate } from './planValues'
import { BlockedChip } from './WorkChips'
import type { Section, WorkTicket } from './viewModel'
import type { TableColumn } from '@penfii/contracts'

export type ListReorder = {
  /** Whether the row has a neighbour to swap with; a parent among parents has none. */
  canStep: (ticket: WorkTicket, section: Section) => boolean
  step: (ticket: WorkTicket, section: Section, step: -1 | 1) => void
  top: (ticket: WorkTicket, section: Section) => void
}

/**
 * The List layout: the View's Tickets as scannable rows, grouped as the View says. Children sit
 * under their parent in the shared order. In a Project the shared order can be changed from a row
 * (Alt+↑/↓, Alt+Home); in a Team or Workspace scope Projects are mixed, so there is no order to
 * change and the rows only show where each Ticket lives.
 */
export function ListLayout({
  sections,
  selectedId,
  compact,
  wide,
  states,
  busy,
  canEdit,
  reorder,
  onOpen,
  onState,
  fields,
  select,
}: {
  sections: Section[]
  selectedId: string | null
  compact: boolean
  /** A scope that mixes Projects: rows name their Project and Channel. */
  wide: boolean
  /** The Project's States when it is known; otherwise each row loads its own Project's. */
  states?: WorkflowState[]
  busy: Set<string>
  canEdit: (ticket: WorkTicket) => boolean
  reorder?: ListReorder
  onOpen: (id: string) => void
  onState: (ticket: WorkTicket, stateId: string) => void
  /** On a phone the Table's columns are the fields a row shows; without it every field shows. */
  fields?: ReadonlySet<TableColumn>
  /** Phone select mode: a row is chosen instead of opened, and its State and order controls rest. */
  select?: { chosen: ReadonlySet<string>; onToggle: (id: string) => void }
}) {
  const { t, locale } = useI18n()
  const show = (column: TableColumn) => !fields || fields.has(column)
  return (
    <div className={compact ? 'work-list compact' : 'work-list'}>
      {sections.map((section) => (
        <section key={section.id} className="list-section" aria-label={section.title || undefined}>
          {section.title ? (
            <h3 className="list-section-head">
              {section.title}
              {section.hint ? <small className="hint"> · {section.hint}</small> : null}
              <span className="list-count">{section.tickets.length}</span>
            </h3>
          ) : null}
          <ul className="list-rows">
            {section.rows.map((row) => {
              if (row.kind === 'context') {
                return (
                  <li key={`context:${row.id}`} className="list-row context">
                    <button type="button" className="list-context-open" onClick={() => onOpen(row.id)} aria-label={t('work.board.openTicket', { key: row.parent.key })}>
                      <span aria-hidden="true">↑</span> <small className="ticket-key">{row.parent.key}</small> <span>{row.parent.title}</span>
                      {row.stateName ? <small className="hint"> · {row.stateName}</small> : null}
                    </button>
                  </li>
                )
              }
              const ticket = row.ticket as WorkTicket
              const done = ticket.stateCategory === 'DONE' || ticket.stateCategory === 'CANCELED'
              const overdue = isOverdue(ticket.dueOn, done)
              const editable = canEdit(ticket) && !busy.has(ticket.id) && !select
              const pickable = select && canEdit(ticket) && !busy.has(ticket.id)
              const chosen = Boolean(select?.chosen.has(ticket.id))
              const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
                if (!reorder || !event.altKey || !canEdit(ticket)) return
                if (event.key === 'ArrowUp') {
                  event.preventDefault()
                  reorder.step(ticket, section, -1)
                } else if (event.key === 'ArrowDown') {
                  event.preventDefault()
                  reorder.step(ticket, section, 1)
                } else if (event.key === 'Home') {
                  event.preventDefault()
                  reorder.top(ticket, section)
                }
              }
              return (
                <li
                  key={ticket.id}
                  className={['list-row', row.nested ? 'nested' : '', ticket.id === selectedId ? 'selected' : '', chosen ? 'checked' : '', done ? 'is-done' : ''].filter(Boolean).join(' ')}
                  data-ticket-id={ticket.id}
                  aria-busy={busy.has(ticket.id)}
                >
                  {select ? (
                    <input type="checkbox" className="list-row-check" aria-label={t('bulk.rowSelect', { key: ticket.key })} checked={chosen} disabled={!pickable} onChange={() => select.onToggle(ticket.id)} />
                  ) : null}
                  <button
                    type="button"
                    className="list-row-open"
                    aria-current={ticket.id === selectedId ? 'true' : undefined}
                    aria-keyshortcuts={reorder && canEdit(ticket) ? 'Alt+ArrowUp Alt+ArrowDown Alt+Home' : undefined}
                    aria-label={t('work.board.openTicket', { key: ticket.key })}
                    onClick={() => (select ? (pickable ? select.onToggle(ticket.id) : undefined) : onOpen(ticket.id))}
                    onKeyDown={onKeyDown}
                  >
                    <small className="ticket-key">{ticket.key}</small>
                    {ticket.hierarchyKind !== 'TASK' ? <span className={`kind-chip kind-${ticket.hierarchyKind.toLowerCase()}`}>{t(`work.kind.${ticket.hierarchyKind}`)}</span> : null}
                    <strong className="list-row-title">{ticket.title}</strong>
                  </button>
                  <span className="list-row-meta">
                    {show('STATE') ? <StateMenu ticket={ticket} known={states} disabled={!editable} onPick={(id) => onState(ticket, id)} /> : null}
                    <BlockedChip ticket={ticket} />
                    {wide ? (
                      <span className="where-chip" title={ticket.channelName ? `${ticket.projectName} · # ${ticket.channelName}` : ticket.projectName}>
                        {ticket.projectName}
                        {ticket.channelName ? <small> # {ticket.channelName}</small> : null}
                      </span>
                    ) : ticket.channelName && show('CHANNEL') ? (
                      <span className="where-chip"># {ticket.channelName}</span>
                    ) : null}
                    {show('PRIORITY') && isPriority(ticket.priority) ? <span className={`priority-chip priority-${ticket.priority.toLowerCase()}`}>{t(`work.priority.${ticket.priority}`)}</span> : null}
                    {show('SPRINT') && ticket.sprint ? <span className="sprint-chip" title={ticket.sprint.name}>{ticket.sprint.name}</span> : null}
                    {(show('LABELS') ? ticket.labels.slice(0, 2) : []).map((label) => (
                      <span key={label.id} className="label-chip">{label.name}</span>
                    ))}
                    {show('DUE_ON') && ticket.dueOn ? (
                      <time dateTime={ticket.dueOn} className={overdue ? 'ticket-due overdue' : 'ticket-due'} title={overdue ? t('work.card.overdue') : undefined}>
                        {formatDue(ticket.dueOn, locale)}
                      </time>
                    ) : null}
                    {fields?.has('MILESTONE') ? ticket.milestones.map((milestone) => <span key={milestone.id} className="label-chip">{milestone.name}</span>) : null}
                    {fields?.has('PARENT') && ticket.parent ? <span className="ticket-parent" title={ticket.parent.title}><small className="ticket-key">{ticket.parent.key}</small></span> : null}
                    {show('ESTIMATE') && ticket.estimate ? <span className="estimate-chip" title={t('work.card.estimate', { estimate: formatEstimate(ticket.estimate) })}>{formatEstimate(ticket.estimate)}</span> : null}
                    {show('ASSIGNEE') && ticket.assigneeName ? (
                      <span className="ticket-assignee" title={ticket.assigneeName}>
                        <Avatar name={ticket.assigneeName} />
                        <span>{ticket.assigneeName}</span>
                      </span>
                    ) : null}
                  </span>
                  {reorder && canEdit(ticket) && !select ? (
                    <span className="list-row-order">
                      {reorder.canStep(ticket, section) ? (
                        <>
                          <button type="button" className="icon-button" aria-label={t('work.list.moveUp', { key: ticket.key })} title="Alt+↑" disabled={busy.has(ticket.id)} onClick={() => reorder.step(ticket, section, -1)}>↑</button>
                          <button type="button" className="icon-button" aria-label={t('work.list.moveDown', { key: ticket.key })} title="Alt+↓" disabled={busy.has(ticket.id)} onClick={() => reorder.step(ticket, section, 1)}>↓</button>
                        </>
                      ) : null}
                      <button type="button" className="icon-button" aria-label={t('work.card.moveTop', { key: ticket.key })} title="Alt+Home" disabled={busy.has(ticket.id)} onClick={() => reorder.top(ticket, section)}>⤒</button>
                    </span>
                  ) : null}
                </li>
              )
            })}
          </ul>
        </section>
      ))}
    </div>
  )
}
