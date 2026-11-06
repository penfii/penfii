import { useState, type DragEvent, type KeyboardEvent } from 'react'
import type { Ticket, WorkflowState } from '@penfii/contracts'
import { Avatar } from '../../design/ui'
import { Sheet, SheetRow } from '../../design/Sheet'
import { useIsMobile } from '../../design/useIsMobile'
import { useI18n } from '../../i18n/I18nProvider'
import { formatDue, isOverdue, isPriority } from './ticketModel'
import { formatEstimate } from './planValues'

export const TICKET_DRAG_TYPE = 'application/x-penfii-ticket'

/**
 * A Board card is a quick read of the Ticket: key, title, type, priority, who, when and
 * child progress. The description and the rest of the detail live in the side panel.
 */
export function TicketCard({
  ticket,
  state,
  selected,
  moving,
  draggable,
  showSprint,
  showState = false,
  onOpen,
  onMoveBy,
  moveChoices,
  currentColumnId,
  onMoveTo,
  onReorder,
  onMoveToTop,
}: {
  ticket: Ticket
  state: WorkflowState | undefined
  selected: boolean
  moving: boolean
  draggable: boolean
  showSprint: boolean
  /** In a View that does not group by State, the card names its State itself. */
  showState?: boolean
  onOpen: () => void
  onMoveBy: (step: -1 | 1) => void
  /** On a phone, where a card cannot be dragged: the State columns it can be moved to. */
  moveChoices?: Array<{ id: string; name: string }>
  currentColumnId?: string
  onMoveTo?: (columnId: string) => void
  onReorder: (step: -1 | 1) => void
  /** Puts the card at the top of its place in the shared order; absent when that is not meaningful here. */
  onMoveToTop?: () => void
}) {
  const { t, locale } = useI18n()
  const mobile = useIsMobile()
  const [moveOpen, setMoveOpen] = useState(false)
  const done = state?.category === 'DONE' || state?.category === 'CANCELED'
  const overdue = isOverdue(ticket.dueOn, done)
  const kind = ticket.hierarchyKind
  const childKey = kind === 'EPIC' ? 'work.card.children.EPIC' : 'work.card.children.TASK'
  const visibleLabels = ticket.labels.slice(0, 2)
  const open = ticket.dependencies.openBlockers
  // An open predecessor keeps a ticket from being completed; once it is complete, the same
  // predecessor reopening is only a recheck.
  const waiting = state?.category === 'NOT_STARTED' || state?.category === 'IN_PROGRESS'

  function onDragStart(event: DragEvent<HTMLElement>) {
    event.dataTransfer.setData(TICKET_DRAG_TYPE, `${ticket.id}:${String(ticket.revisionNo)}`)
    event.dataTransfer.effectAllowed = 'move'
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (!draggable || !event.altKey) return
    if (event.key === 'ArrowLeft') {
      event.preventDefault()
      onMoveBy(-1)
    } else if (event.key === 'ArrowRight') {
      event.preventDefault()
      onMoveBy(1)
    } else if (event.key === 'Home' && onMoveToTop) {
      event.preventDefault()
      onMoveToTop()
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      onReorder(-1)
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      onReorder(1)
    }
  }

  return (
    <article
      className={['task-card', 'ticket-card', selected ? 'selected' : '', done ? 'is-done' : ''].filter(Boolean).join(' ')}
      aria-busy={moving}
      data-ticket-id={ticket.id}
      draggable={draggable && !moving}
      onDragStart={onDragStart}
    >
      <button
        type="button"
        className="task-card-open ticket-card-open"
        aria-current={selected ? 'true' : undefined}
        aria-keyshortcuts={draggable ? `Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown${onMoveToTop ? ' Alt+Home' : ''}` : undefined}
        aria-label={t('work.board.openTicket', { key: ticket.key })}
        onClick={onOpen}
        onKeyDown={onKeyDown}
      >
        <span className="ticket-card-top">
          <small className="ticket-key">{ticket.key}</small>
          {showState && state ? <span className={`state-chip state-${state.category.toLowerCase()}`}>{state.name}</span> : null}
          {kind !== 'TASK' ? <span className={`kind-chip kind-${kind.toLowerCase()}`}>{t(`work.kind.${kind}`)}</span> : null}
          {ticket.ticketType ? (
            <span className="type-chip" style={ticket.ticketType.color ? { ['--chip' as string]: ticket.ticketType.color } : undefined}>
              {ticket.ticketType.name}
            </span>
          ) : null}
          {isPriority(ticket.priority) ? <span className={`priority-chip priority-${ticket.priority.toLowerCase()}`}>{t(`work.priority.${ticket.priority}`)}</span> : null}
          {open > 0 ? (
            <span className={waiting ? 'blocked-chip' : 'blocked-chip recheck'} title={t(waiting ? 'work.card.blockedHelp' : 'work.card.recheckHelp', { count: String(open) })}>
              {t(waiting ? 'work.card.blocked' : 'work.card.recheck', { count: String(open) })}
            </span>
          ) : null}
        </span>
        <strong className="ticket-card-title">{ticket.title}</strong>
        {ticket.parent ? (
          <span className="ticket-card-parent" title={ticket.parent.title}>
            ↑ {ticket.parent.key} · {ticket.parent.title}
          </span>
        ) : null}
        <span className="ticket-card-foot">
          {ticket.assigneeName ? (
            <span className="ticket-assignee" title={ticket.assigneeName}>
              <Avatar name={ticket.assigneeName} />
              <span>{ticket.assigneeName}</span>
            </span>
          ) : null}
          {ticket.dueOn ? (
            <time dateTime={ticket.dueOn} className={overdue ? 'ticket-due overdue' : 'ticket-due'} title={overdue ? t('work.card.overdue') : undefined}>
              {formatDue(ticket.dueOn, locale)}
            </time>
          ) : null}
          {ticket.estimate ? <span className="estimate-chip" title={t('work.card.estimate', { estimate: formatEstimate(ticket.estimate) })}>{formatEstimate(ticket.estimate)}</span> : null}
          {ticket.childSummary.total > 0 && kind !== 'SUBTASK' ? (
            <span className="ticket-children" title={t('work.children.hiddenNote')}>
              <span className="progress-mini" aria-hidden="true">
                <span style={{ width: `${String(Math.round((ticket.childSummary.done / ticket.childSummary.total) * 100))}%` }} />
              </span>
              {t(childKey, { done: String(ticket.childSummary.done), total: String(ticket.childSummary.total) })}
            </span>
          ) : null}
          {showSprint && ticket.sprint ? <span className="sprint-chip" title={ticket.sprint.name}>{ticket.sprint.name}</span> : null}
          {visibleLabels.map((label) => (
            <span key={label.id} className="label-chip">{label.name}</span>
          ))}
          {ticket.labels.length > visibleLabels.length ? <span className="label-chip">+{ticket.labels.length - visibleLabels.length}</span> : null}
          {moving ? <span role="status">{t('work.card.moving')}</span> : null}
        </span>
      </button>
      {mobile && moveChoices && onMoveTo && !moving ? (
        <>
          <button type="button" className="card-move" aria-haspopup="dialog" aria-label={t('work.card.moveTo', { key: ticket.key })} onClick={() => setMoveOpen(true)}>⇄</button>
          <Sheet open={moveOpen} title={t('work.card.moveTo', { key: ticket.key })} onClose={() => setMoveOpen(false)}>
            {moveChoices.map((choice) => (
              <SheetRow key={choice.id} selected={choice.id === currentColumnId} onClick={() => { setMoveOpen(false); if (choice.id !== currentColumnId) onMoveTo(choice.id) }}>{choice.name}</SheetRow>
            ))}
          </Sheet>
        </>
      ) : null}
      {onMoveToTop && !moving ? (
        <button type="button" className="card-top" aria-label={t('work.card.moveTop', { key: ticket.key })} title={`${t('work.card.moveTop', { key: ticket.key })} (Alt+Home)`} onClick={onMoveToTop}>
          ⤒
        </button>
      ) : null}
    </article>
  )
}
