import { useEffect, useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { MovePlacement, ProjectBoard, ProjectMember, ProjectPlanning, Ticket, ViewDefinition } from '@penfii/contracts'
import { api } from '../../api'
import { EmptyState } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { isSamePlace, placeInBoard, placementAt, placementForStep } from './boardMoves'
import { BoardColumn } from './BoardColumn'
import { useTicketPanel } from './panelParam'
import { refreshTicket } from './refresh'
import { moveErrorKey } from './moveErrors'
import { allTickets } from './ticketModel'
import { buildColumns, canReorder, siblingIds } from './viewModel'

/**
 * The Board layout of a Project's View. Cards are Tickets; columns come from the View's grouping
 * (Workflow States by default). Creating, moving, reordering and opening all keep the Board in
 * place, and the shared order is kept on the server. Which Tickets show, and in what grouping, is
 * decided by the View; this component only draws and moves them.
 */
export function BoardLayout({
  projectId,
  data,
  definition,
  planning,
  members,
  onMessage,
  message,
}: {
  projectId: string
  data: ProjectBoard
  definition: ViewDefinition
  planning: ProjectPlanning | undefined
  members: ProjectMember[]
  message: string
  onMessage: (message: string) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const boardKey = queryKeys.projectBoard(projectId)
  const { selectedId, open, openCreate } = useTicketPanel()
  const [moving, setMoving] = useState<Set<string>>(() => new Set())
  const [draggingId, setDraggingId] = useState<string | null>(null)
  const [focusId, setFocusId] = useState<string | null>(null)
  const setMessage = onMessage

  useEffect(() => {
    if (!focusId) return
    const target = document.querySelector<HTMLElement>(`[data-ticket-id="${focusId}"] .ticket-card-open`)
    target?.focus()
    setFocusId(null)
  }, [focusId, data])

  const columns = useMemo(
    () =>
      buildColumns(
        data,
        definition,
        planning?.sprints ?? [],
        members,
        { unassigned: t('view.group.none'), noPriority: t('view.group.noPriority'), backlog: t('view.group.backlog'), priority: (value) => t(`work.priority.${value as 'LOW'}`) },
      ),
    [data, definition, planning, members, t],
  )

  const states = new Map(data.states.map((state) => [state.id, state]))
  const initialId = data.states.find((state) => state.isInitial)?.id
  const byOrder = canReorder(definition)

  function explain(cause: unknown) {
    setMessage(t(moveErrorKey(cause)))
  }

  /**
   * Puts a card in a column at a place. A State column changes the State and/or the shared
   * order in one command; a Sprint column assigns the Sprint, then orders.
   */
  async function drop(ticketId: string, column: (typeof columns)[number], placement: MovePlacement | null) {
    if (moving.has(ticketId)) return
    const current = queryClient.getQueryData<ProjectBoard>(boardKey)
    const ticket = current ? allTickets(current).find((item) => item.id === ticketId) : undefined
    if (!current || !ticket) return
    setMessage('')
    const toStatus = column.stateId !== null && !column.stateIds.includes(ticket.statusId) ? column.stateIds[0] : undefined
    const toSprint = column.sprintId !== undefined && (ticket.sprint?.id ?? null) !== column.sprintId
    if (column.sprintId !== undefined && toSprint && ticket.hierarchyKind !== 'TASK') {
      setMessage(t('work.move.sprintTask'))
      return
    }
    if (toStatus === undefined && !toSprint && !placement) return
    const sprint = toSprint
      ? column.sprintId === null
        ? null
        : { id: column.sprintId ?? '', name: column.name, state: planning?.sprints.find((item) => item.id === column.sprintId)?.state ?? 'PLANNED', inherited: false }
      : undefined
    setMoving((items) => new Set(items).add(ticketId))
    queryClient.setQueryData(boardKey, placeInBoard(current, ticketId, { statusId: toStatus, sprint, placement: placement ?? undefined }))
    try {
      if (toSprint) await api.assignSprint(ticketId, column.sprintId ?? null, newIdempotencyKey())
      if (toStatus !== undefined || placement) {
        await api.moveTicket(
          ticketId,
          { ...(toStatus !== undefined ? { expectedRevision: ticket.revisionNo, statusId: toStatus } : {}), ...(placement ?? {}) },
          newIdempotencyKey(),
        )
      }
      if (placement) setFocusId(ticketId)
    } catch (cause) {
      explain(cause)
    } finally {
      setMoving((items) => {
        const next = new Set(items)
        next.delete(ticketId)
        return next
      })
      await refreshTicket(queryClient, ticketId, projectId)
    }
  }

  function onDropTicket(column: (typeof columns)[number], ticketId: string, slot: number | null, ids: string[]) {
    // In a grouped column only siblings of the same parent are ordered by hand; a parent or a lone
    // card changes State but never the shared order.
    if (slot === null || !byOrder || (column.hierarchical && ids.length === 0)) {
      void drop(ticketId, column, null)
      return
    }
    if (isSamePlace(ids, slot, ticketId)) return
    void drop(ticketId, column, placementAt(ids, slot, ticketId))
  }

  function moveBy(ticket: Ticket, step: -1 | 1) {
    const index = columns.findIndex((column) => column.tickets.some((item) => item.id === ticket.id))
    const target = columns[index + step]
    if (!target || target.stateId === null) return
    const at = siblingIds(target, ticket)
    void drop(ticket.id, target, byOrder && (at.length > 0 || !target.hierarchical) ? placementAt(at, at.length, ticket.id) : null)
  }

  function reorderBy(ticket: Ticket, step: -1 | 1) {
    if (!byOrder) return
    const column = columns.find((item) => item.tickets.some((entry) => entry.id === ticket.id))
    if (!column) return
    const placement = placementForStep(siblingIds(column, ticket), ticket.id, step)
    if (placement) void drop(ticket.id, column, placement)
  }

  /**
   * "Move to top": a child goes first among the children of its parent; a parent or a card on its
   * own goes first in the shared order. The server keeps the order and a time, so the card stays on
   * top of the newest-first parent groups until something more recent happens. Not offered where the
   * View orders by something else.
   */
  function moveToTopFor(column: (typeof columns)[number], ticket: Ticket): (() => void) | null {
    if (!byOrder) return null
    const own = column.rows.find((row) => row.kind === 'ticket' && row.ticket.id === ticket.id)
    if (column.hierarchical && own?.kind === 'ticket' && own.nested) {
      const siblings = siblingIds(column, ticket)
      const first = siblings[0]
      return first === undefined || first === ticket.id ? null : () => void drop(ticket.id, column, { beforeId: first })
    }
    if (column.hierarchical && column.rows.findIndex((row) => row.kind === 'ticket' && row.ticket.id === ticket.id) === 0 && !ticket.movedToTopAt) return null
    if (!column.hierarchical && column.tickets[0]?.id === ticket.id) return null
    return () => void drop(ticket.id, column, { position: 'first' })
  }

  async function setWip(stateId: string, limit: number | null): Promise<boolean> {
    try {
      await api.updateWorkflowState(projectId, stateId, { expectedWorkflowVersion: data.workflowVersion, wipLimit: limit })
      await queryClient.invalidateQueries({ queryKey: boardKey })
      return true
    } catch {
      await queryClient.invalidateQueries({ queryKey: boardKey })
      return false
    }
  }

  if (data.columns.length === 0) return <EmptyState title={t('shell.project.work')} body={t('projects.workflowEmpty')} />
  const total = allTickets(data).length
  const stateGrouping = definition.group === 'STATUS'
  const sprintGrouping = definition.group === 'SPRINT'
  const hint = !data.canEdit || total === 0
    ? null
    : definition.group === 'ASSIGNEE' || definition.group === 'PRIORITY'
      ? t('view.hint.fixedGroup')
      : !byOrder
        ? t('view.hint.fixedSort')
        : columns.some((column) => column.hierarchical)
          ? t('work.board.hierarchyHint')
          : t('work.board.reorderHint')

  return (
    <>
      {hint ? <small className="field-hint board-hint">{hint}</small> : null}
      {message ? <p className="board-error" role="alert">{message}</p> : null}
      {total === 0 ? <p className="board-empty">{t('work.board.empty')}</p> : null}
      <div className="board-viewport">
        <div
          className="board"
          style={{ ['--board-cols' as string]: columns.length }}
          onDragStart={(event) => setDraggingId((event.target as HTMLElement).closest<HTMLElement>('[data-ticket-id]')?.dataset.ticketId ?? null)}
          onDragEnd={() => setDraggingId(null)}
          onDrop={() => setDraggingId(null)}
        >
          {columns.map((column) => (
            <BoardColumn
              key={column.id}
              column={column}
              states={states}
              selectedId={selectedId}
              moving={moving}
              canEdit={data.canEdit}
              canDrop={stateGrouping || (sprintGrouping && column.sprintId !== undefined)}
              canReorder={byOrder}
              canSetWip={data.canManageWorkflow && stateGrouping}
              showCreate={data.canEdit && stateGrouping && column.stateId !== null && column.stateIds.includes(initialId ?? '')}
              dragging={draggingId !== null}
              draggingId={draggingId}
              groupIds={(ticketId) => {
                const dragged = allTickets(data).find((item) => item.id === ticketId)
                return dragged ? siblingIds(column, dragged) : []
              }}
              showSprint={!sprintGrouping}
              showState={!stateGrouping}
              onCreate={() => openCreate({ projectId })}
              onDropTicket={(ticketId, slot, ids) => onDropTicket(column, ticketId, slot, ids)}
              onOpen={(ticket) => {
                setMessage('')
                open(ticket.id)
              }}
              onOpenRef={(id) => {
                setMessage('')
                open(id)
              }}
              onMoveBy={moveBy}
              onReorder={reorderBy}
              moveToTopFor={(ticket) => moveToTopFor(column, ticket)}
              onSetWip={setWip}
            />
          ))}
        </div>
      </div>
    </>
  )
}
