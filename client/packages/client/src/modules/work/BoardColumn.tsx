import { useRef, useState, type DragEvent } from 'react'
import type { Ticket, WorkflowState } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { slotFromY } from './boardMoves'
import { Popover } from './Popover'
import { TICKET_DRAG_TYPE, TicketCard } from './TicketCard'
import type { ColumnModel } from './viewModel'
import { GearIcon } from '../../design/icons'

/**
 * One Board column. The whole column, header to bottom edge, is the drop target, so an empty
 * column is as easy to drop on as a full one. Its width comes from the Board, never from its tickets.
 * While a card is over the column a line shows the slot it will take.
 */
export function BoardColumn({
  column,
  states,
  selectedId,
  moving,
  canEdit,
  canDrop,
  canReorder,
  canSetWip,
  showCreate,
  dragging,
  draggingId,
  groupIds,
  showSprint,
  showState,
  onCreate,
  onDropTicket,
  onOpen,
  onOpenRef,
  onMoveBy,
  moveChoices,
  onMoveTo,
  onReorder,
  moveToTopFor,
  onSetWip,
  picked,
  onPick,
}: {
  column: ColumnModel
  states: Map<string, WorkflowState>
  selectedId: string | null
  moving: Set<string>
  canEdit: boolean
  /** Whether a card may be dropped here at all (State and Sprint columns). */
  canDrop: boolean
  /** Whether the place inside the column is meaningful (the View shows the shared order). */
  canReorder: boolean
  canSetWip: boolean
  showCreate: boolean
  dragging: boolean
  draggingId: string | null
  /** The cards a drop of this Ticket is positioned against in this column. */
  groupIds: (ticketId: string) => string[]
  showSprint: boolean
  showState: boolean
  onCreate: () => void
  /** `slot` is where in the column the pointer was, or null when the View does not order by hand. */
  onDropTicket: (ticketId: string, slot: number | null, ids: string[]) => void
  onOpen: (ticket: Ticket) => void
  /** Opens a parent shown as context. */
  onOpenRef: (ticketId: string) => void
  onMoveBy: (ticket: Ticket, step: -1 | 1) => void
  moveChoices: Array<{ id: string; name: string }>
  onMoveTo: (ticket: Ticket, columnId: string) => void
  onReorder: (ticket: Ticket, step: -1 | 1) => void
  /** Returns null when moving this card to the top would change nothing. */
  moveToTopFor: (ticket: Ticket) => (() => void) | null
  onSetWip: (stateId: string, limit: number | null) => Promise<boolean>
  /** The Tickets chosen for a bulk archive/trash. Only Completed and Canceled cards can be chosen. */
  picked?: Set<string>
  onPick?: (ticketId: string) => void
}) {
  const { t } = useI18n()
  const [over, setOver] = useState(false)
  const [slot, setSlot] = useState<number | null>(null)
  const dropRef = useRef<HTMLDivElement>(null)
  const isTicketDrag = (event: DragEvent) => event.dataTransfer.types.includes(TICKET_DRAG_TYPE)
  const draggedIds = draggingId ? groupIds(draggingId) : []
  const interactive = canEdit && canDrop
  const warn = column.wipLimit !== null && column.total > column.wipLimit

  function slotAt(event: DragEvent, ids: string[]): number {
    const cards = Array.from(dropRef.current?.querySelectorAll<HTMLElement>('[data-ticket-id]') ?? []).filter((card) => ids.includes(card.dataset.ticketId ?? ''))
    return slotFromY(cards.map((card) => card.getBoundingClientRect()), event.clientY)
  }

  function leave() {
    setOver(false)
    setSlot(null)
  }

  return (
    <section
      className={over ? 'board-col drop-over' : 'board-col'}
      aria-label={column.name}
      data-column-id={column.id}
      onDragEnter={(event) => {
        if (interactive && isTicketDrag(event)) setOver(true)
      }}
      onDragOver={(event) => {
        if (!interactive || !isTicketDrag(event)) return
        event.preventDefault()
        event.dataTransfer.dropEffect = 'move'
        if (!over) setOver(true)
        const next = canReorder && draggedIds.length > 0 ? slotAt(event, draggedIds) : null
        if (next !== slot) setSlot(next)
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) leave()
      }}
      onDrop={(event) => {
        event.preventDefault()
        const [ticketId] = event.dataTransfer.getData(TICKET_DRAG_TYPE).split(':')
        const ids = ticketId ? groupIds(ticketId) : []
        const target = canReorder && (ids.length > 0 || !column.hierarchical) ? slotAt(event, ids) : null
        leave()
        if (!interactive || !ticketId) return
        onDropTicket(ticketId, target, ids)
      }}
    >
      <h3>
        <span className={`state-dot ${(column.category ?? 'none').toLowerCase()}`} aria-hidden="true" />
        <span className="board-col-name">{column.name}</span>
        <span className="board-count" aria-label={String(column.tickets.length)}>{column.tickets.length}</span>
        {column.wipLimit !== null ? (
          <span className={warn ? 'wip-badge over' : 'wip-badge'} title={t('work.wip.help')}>
            {warn ? `${t('work.wip.over')} · ` : ''}{t('work.wip.label', { count: String(column.total), limit: String(column.wipLimit) })}
          </span>
        ) : null}
        {canSetWip && column.stateId ? (
          <WipEditor name={column.name} limit={column.wipLimit} onSave={(limit) => onSetWip(column.stateId ?? '', limit)} />
        ) : null}
      </h3>
      {showCreate ? (
        <button type="button" className="quick-add-trigger" onClick={onCreate}>
          <span aria-hidden="true">+</span> <span className="quick-add-label">{t('work.addTicket')}</span>
        </button>
      ) : null}
      <div className="column-drop" ref={dropRef}>
        {column.tickets.length === 0 ? (
          <p className={dragging && interactive ? 'column-empty is-target' : 'column-empty'}>
            {dragging && interactive ? t('work.board.dropHere') : t('work.board.columnEmpty')}
          </p>
        ) : null}
        {column.rows.map((row) => {
          if (row.kind === 'context') {
            const parent = row.parent
            return (
              <div key={`context-${row.id}`} className="card-slot context-slot">
                <button type="button" className="context-header" data-context-id={row.id} title={parent.title} onClick={() => onOpenRef(parent.id)}>
                  <span className="context-label">{t('work.board.parentLabel')}</span>
                  <small className="ticket-key">{parent.key}</small>
                  <span className="context-title">{parent.title}</span>
                  {row.stateName ? <span className="context-state">{row.stateName}</span> : null}
                </button>
              </div>
            )
          }
          const ticket = row.ticket
          const at = draggedIds.indexOf(ticket.id)
          return (
            <div
              key={ticket.id}
              className={['card-slot', row.nested ? 'nested' : '', slot !== null && at !== -1 && slot === at ? 'slot-before' : '', slot !== null && at !== -1 && slot === draggedIds.length && at === draggedIds.length - 1 ? 'slot-after' : ''].filter(Boolean).join(' ')}
            >
              <TicketCard
                ticket={ticket}
                state={states.get(ticket.statusId)}
                selected={selectedId === ticket.id}
                moving={moving.has(ticket.id)}
                draggable={canEdit && canDrop}
                showSprint={showSprint}
                showState={showState}
                onOpen={() => onOpen(ticket)}
                onMoveBy={(step) => onMoveBy(ticket, step)}
                moveChoices={canEdit && canDrop ? moveChoices : undefined}
                currentColumnId={column.id}
                onMoveTo={(columnId) => onMoveTo(ticket, columnId)}
                onReorder={(step) => onReorder(ticket, step)}
                onMoveToTop={canEdit && canDrop ? (moveToTopFor(ticket) ?? undefined) : undefined}
                pick={picked && onPick && (states.get(ticket.statusId)?.category === 'DONE' || states.get(ticket.statusId)?.category === 'CANCELED') ? { checked: picked.has(ticket.id), onToggle: () => onPick(ticket.id) } : undefined}
              />
            </div>
          )
        })}
      </div>
    </section>
  )
}

function WipEditor({ name, limit, onSave }: { name: string; limit: number | null; onSave: (limit: number | null) => Promise<boolean> }) {
  const { t } = useI18n()
  const [draft, setDraft] = useState(limit === null ? '' : String(limit))
  const [error, setError] = useState('')
  const parsed = draft.trim() === '' ? null : Number(draft)
  const valid = parsed === null || (Number.isInteger(parsed) && parsed >= 1 && parsed <= 1000)

  async function save(value: number | null, close: () => void) {
    if (!valid) {
      setError(t('work.wip.invalid'))
      return
    }
    const ok = await onSave(value)
    if (ok) close()
    else setError(t('work.wip.error'))
  }

  return (
    <span className="wip-editor">
      <Popover label={t('work.wip.settings', { name })} icon={<GearIcon />} iconOnly align="end">
        {(close) => (
          <form
            className="view-form"
            onSubmit={(event) => {
              event.preventDefault()
              void save(parsed, close)
            }}
          >
            <label className="field">
              <span>{t('work.wip.edit', { name })}</span>
              <input type="number" min={1} max={1000} inputMode="numeric" aria-label={t('work.wip.field')} value={draft} onChange={(event) => { setDraft(event.target.value); setError('') }} />
            </label>
            <small className="field-hint">{t('work.wip.help')}</small>
            {error ? <p className="board-error" role="alert">{error}</p> : null}
            <div className="dialog-actions">
              <Button type="button" variant="ghost" onClick={() => { setDraft(''); void save(null, close) }}>{t('work.wip.clear')}</Button>
              <Button type="submit">{t('work.wip.save')}</Button>
            </div>
          </form>
        )}
      </Popover>
    </span>
  )
}
