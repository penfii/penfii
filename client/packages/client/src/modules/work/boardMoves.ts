import type { MovePlacement, ProjectBoard, SprintRef, Ticket } from '@penfii/contracts'

/*
 * Where a dragged card goes. A drop is described by a neighbour (before or after another
 * card), never by an index, so the server places it correctly even if the column changed
 * while the card was in the air.
 */

/** The slot a pointer at `y` points to: 0 is before the first card, `rects.length` after the last. */
export function slotFromY(rects: Array<{ top: number; height: number }>, y: number): number {
  const index = rects.findIndex((rect) => y < rect.top + rect.height / 2)
  return index === -1 ? rects.length : index
}

/** Whether dropping `draggedId` at `slot` of the column `ids` leaves it where it is. */
export function isSamePlace(ids: string[], slot: number, draggedId: string): boolean {
  const at = ids.indexOf(draggedId)
  return at !== -1 && (slot === at || slot === at + 1)
}

/** The neighbour-based placement for dropping at `slot`; the dragged card is not its own neighbour. */
export function placementAt(ids: string[], slot: number, draggedId: string): MovePlacement {
  const others = ids.filter((id) => id !== draggedId)
  const at = ids.indexOf(draggedId)
  const index = at !== -1 && at < slot ? slot - 1 : slot
  if (others.length === 0) return { position: 'last' }
  if (index < others.length) return { beforeId: others[index] }
  return { afterId: others[others.length - 1] }
}

/** The placement for moving a card one step up (-1) or down (+1) in its column, or null at the edge. */
export function placementForStep(ids: string[], ticketId: string, step: -1 | 1): MovePlacement | null {
  const at = ids.indexOf(ticketId)
  if (at === -1) return null
  const neighbour = ids[at + step]
  if (neighbour === undefined) return null
  return step === -1 ? { beforeId: neighbour } : { afterId: neighbour }
}

/**
 * The board as it will look once the server accepts the move, so the card lands at once.
 * The next fetch replaces it with what the server holds.
 */
export function placeInBoard(
  board: ProjectBoard,
  ticketId: string,
  change: { statusId?: string; sprint?: SprintRef | null; placement?: MovePlacement },
): ProjectBoard {
  const moving = board.columns.flatMap((column) => column.tickets).find((item) => item.id === ticketId)
  if (!moving) return board
  const moved: Ticket = {
    ...moving,
    statusId: change.statusId ?? moving.statusId,
    sprint: change.sprint === undefined ? moving.sprint : change.sprint,
    // Putting a card first leaves a time, as the server does.
    movedToTopAt: change.placement?.position === 'first' ? new Date().toISOString() : moving.movedToTopAt,
  }
  const placement = change.placement
  return {
    ...board,
    columns: board.columns.map((column) => {
      const without = column.tickets.filter((item) => item.id !== ticketId)
      if (!column.stateIds.includes(moved.statusId)) return { ...column, tickets: without }
      let index = without.length
      if (placement?.position === 'first') index = 0
      else if (placement?.afterId) {
        const anchor = without.findIndex((item) => item.id === placement.afterId)
        if (anchor !== -1) index = anchor + 1
      } else if (placement?.beforeId) {
        const anchor = without.findIndex((item) => item.id === placement.beforeId)
        if (anchor !== -1) index = anchor
      }
      const tickets = [...without.slice(0, index), moved, ...without.slice(index)]
      return { ...column, tickets }
    }),
  }
}
