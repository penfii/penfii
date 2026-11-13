import { useEffect, useMemo, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useParams } from 'react-router'
import { newIdempotencyKey } from '@penfii/api-client'
import type { MovePlacement, ProjectBoard, ProjectMember, ProjectPlanning, Ticket, ViewDefinition } from '@penfii/contracts'
import { api } from '../../api'
import { EmptyState, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { isSamePlace, placeInBoard, placementAt, placementForStep } from './boardMoves'
import { BoardColumn } from './BoardColumn'
import { BulkLifecycleBar } from './BulkLifecycleBar'
import { finishedSelection, restoreFromTrash, useBulkLifecycle, type LifecycleAction } from './bulkLifecycle'
import { notify } from '../../design/notices'
import { useIsMobile } from '../../design/useIsMobile'
import { useTicketPanel } from './panelParam'
import { holdBoardRefresh, noteOwnChange, refreshTicket } from './refresh'
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
  const { workspaceId = '' } = useParams()
  // Completed and Canceled cards can be chosen for a bulk archive or trash, by the Project admin.
  const [chosen, setChosen] = useState<Set<string>>(() => new Set())
  const lifecycle = useBulkLifecycle(workspaceId, (outcome) => {
    // What was not processed stays chosen; what was processed has left the Board.
    setChosen(new Set(outcome.items.filter((item) => !item.ok).map((item) => item.ticketId)))
    void queryClient.invalidateQueries({ queryKey: boardKey })
    void queryClient.invalidateQueries({ queryKey: queryKeys.projectArchive(projectId) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) })
  })
  const selectedIds = useMemo(() => finishedSelection(chosen, data), [chosen, data])
  const { confirm, dialog: confirmDialog } = useConfirm()
  const mobile = useIsMobile()
  const refreshLifecycle = () => {
    void queryClient.invalidateQueries({ queryKey: boardKey })
    void queryClient.invalidateQueries({ queryKey: queryKeys.projectArchive(projectId) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) })
  }
  /** Trash is asked about first and can be taken back; the result also stays in the bar until it is dismissed. */
  async function runLifecycle(action: LifecycleAction) {
    // On a phone the result carries Undo instead of a blocking question (the design's choice for the small screen).
    if (action === 'TRASH' && !mobile && !(await confirm({ title: t('bulk.lifecycle.trashConfirmTitle'), body: t('bulk.lifecycle.trashConfirm', { count: String(selectedIds.length) }), confirmLabel: t('bulk.lifecycle.trash') }))) return
    const outcome = await lifecycle.run(action, selectedIds)
    if (!outcome || action !== 'TRASH') return
    const moved = outcome.items.filter((item) => item.ok).map((item) => item.ticketId)
    if (outcome.failed > 0) {
      notify({ kind: 'error', message: t('bulk.lifecycle.trashPartial', { total: String(outcome.total), ok: String(outcome.succeeded), failed: String(outcome.failed) }) })
    } else if (moved.length > 0) {
      notify({
        message: t('bulk.lifecycle.trashDone', { total: String(outcome.total) }),
        action: {
          label: t('notice.undo'),
          run: () => void restoreFromTrash(moved).then(({ ok, failed }) => {
            refreshLifecycle()
            // The bar's "moved to the trash" is no longer true once they are back.
            if (ok > 0) lifecycle.clear()
            if (failed === 0) notify({ message: t('bulk.lifecycle.undone', { count: String(ok) }) })
            else notify({ kind: 'error', message: t('bulk.lifecycle.undonePartial', { total: String(moved.length), ok: String(ok), failed: String(failed) }) })
          }),
        },
      })
    }
  }
  function togglePick(ticketId: string) {
    setChosen((current) => {
      const next = new Set(current)
      if (!next.delete(ticketId)) next.add(ticketId)
      return next
    })
  }

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
    // No read of the Board may land on the card's way, and what is on screen now is what to go back to if the move fails.
    const release = holdBoardRefresh(queryClient, projectId)
    await queryClient.cancelQueries({ queryKey: boardKey })
    const before = queryClient.getQueryData<ProjectBoard>(boardKey) ?? current
    queryClient.setQueryData(boardKey, placeInBoard(before, ticketId, { statusId: toStatus, sprint, placement: placement ?? undefined }))
    try {
      if (toSprint) noteOwnChange(ticketId, (await api.assignSprint(ticketId, column.sprintId ?? null, newIdempotencyKey())).revisionNo)
      if (toStatus !== undefined || placement) {
        const moved = await api.moveTicket(
          ticketId,
          { ...(toStatus !== undefined ? { expectedRevision: ticket.revisionNo, statusId: toStatus } : {}), ...(placement ?? {}) },
          newIdempotencyKey(),
        )
        noteOwnChange(ticketId, moved.revisionNo)
      }
      if (placement) setFocusId(ticketId)
    } catch (cause) {
      // Back to what was there. When the first step (the Sprint) worked and the second did not, the read below shows the truth.
      queryClient.setQueryData(boardKey, before)
      explain(cause)
    } finally {
      setMoving((items) => {
        const next = new Set(items)
        next.delete(ticketId)
        return next
      })
      release(false)
      // Read in the background: the card is free to move again as soon as the server has answered.
      void refreshTicket(queryClient, ticketId, projectId)
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

  /** A phone cannot drag: a card goes to a State column chosen by name (the same move as Alt+←/→, to any column). */
  function moveTo(ticket: Ticket, columnId: string) {
    const target = columns.find((column) => column.id === columnId)
    if (!target || target.stateId === null) return
    const at = siblingIds(target, ticket)
    void drop(ticket.id, target, byOrder && (at.length > 0 || !target.hierarchical) ? placementAt(at, at.length, ticket.id) : null)
  }
  const moveChoices = columns.filter((column) => column.stateId !== null).map((column) => ({ id: column.id, name: column.name }))

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
      {data.canManageWorkflow && (selectedIds.length > 0 || lifecycle.result) ? (
        <BulkLifecycleBar
          count={selectedIds.length}
          running={lifecycle.running}
          result={lifecycle.result}
          last={lifecycle.last}
          names={new Map(allTickets(data).map((ticket) => [ticket.id, { key: ticket.key, title: ticket.title }]))}
          onRun={(action) => void runLifecycle(action)}
          onClear={() => {
            setChosen(new Set())
            lifecycle.clear()
          }}
        />
      ) : null}
      {confirmDialog}
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
              moveChoices={moveChoices}
              onMoveTo={moveTo}
              onReorder={reorderBy}
              moveToTopFor={(ticket) => moveToTopFor(column, ticket)}
              onSetWip={setWip}
              picked={data.canManageWorkflow ? new Set(selectedIds) : undefined}
              onPick={togglePick}
            />
          ))}
        </div>
      </div>
    </>
  )
}
