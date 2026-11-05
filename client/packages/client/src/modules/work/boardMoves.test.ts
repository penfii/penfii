import { describe, expect, it } from 'vitest'
import { fixtureBoard, fixtureTicket as ticket } from './testFixtures'
import { isSamePlace, placeInBoard, placementAt, placementForStep, slotFromY } from './boardMoves'

describe('slots', () => {
  const rects = [{ top: 0, height: 40 }, { top: 48, height: 40 }, { top: 96, height: 40 }]

  it('picks the slot by the middle of each card', () => {
    expect(slotFromY(rects, 5)).toBe(0)
    expect(slotFromY(rects, 25)).toBe(1)
    expect(slotFromY(rects, 70)).toBe(2)
    expect(slotFromY(rects, 200)).toBe(3)
    expect(slotFromY([], 10)).toBe(0)
  })

  it('knows when a drop leaves the card where it is', () => {
    const ids = ['a', 'b', 'c']
    expect(isSamePlace(ids, 1, 'b')).toBe(true)
    expect(isSamePlace(ids, 2, 'b')).toBe(true)
    expect(isSamePlace(ids, 0, 'b')).toBe(false)
    expect(isSamePlace(ids, 3, 'b')).toBe(false)
    expect(isSamePlace(ids, 0, 'x')).toBe(false)
  })
})

describe('placement', () => {
  it('uses a neighbour, and never the dragged card itself', () => {
    const ids = ['a', 'b', 'c']
    expect(placementAt(ids, 0, 'x')).toEqual({ beforeId: 'a' })
    expect(placementAt(ids, 3, 'x')).toEqual({ afterId: 'c' })
    expect(placementAt(ids, 1, 'x')).toEqual({ beforeId: 'b' })
    // Dragging a down past c: the slot after c is "after c".
    expect(placementAt(ids, 3, 'a')).toEqual({ afterId: 'c' })
    // Dragging c up before a.
    expect(placementAt(ids, 0, 'c')).toEqual({ beforeId: 'a' })
    // Dragging a to the slot before c: before c.
    expect(placementAt(ids, 2, 'a')).toEqual({ beforeId: 'c' })
  })

  it('drops into an empty column at the end', () => {
    expect(placementAt([], 0, 'x')).toEqual({ position: 'last' })
    expect(placementAt(['x'], 1, 'x')).toEqual({ position: 'last' })
  })

  it('steps to the neighbour with the keyboard and stops at the edges', () => {
    const ids = ['a', 'b', 'c']
    expect(placementForStep(ids, 'b', -1)).toEqual({ beforeId: 'a' })
    expect(placementForStep(ids, 'b', 1)).toEqual({ afterId: 'c' })
    expect(placementForStep(ids, 'a', -1)).toBeNull()
    expect(placementForStep(ids, 'c', 1)).toBeNull()
    expect(placementForStep(ids, 'x', 1)).toBeNull()
  })
})

describe('optimistic board', () => {
  const board = fixtureBoard([
    ticket({ id: 'a', statusId: 's-todo' }),
    ticket({ id: 'b', statusId: 's-todo' }),
    ticket({ id: 'c', statusId: 's-doing' }),
  ])
  const ids = (next: ReturnType<typeof fixtureBoard>) => next.columns.map((column) => column.tickets.map((item) => item.id))

  it('reorders inside a column', () => {
    expect(ids(placeInBoard(board, 'b', { placement: { beforeId: 'a' } }))).toEqual([['b', 'a'], ['c'], []])
    expect(ids(placeInBoard(board, 'a', { placement: { afterId: 'b' } }))).toEqual([['b', 'a'], ['c'], []])
  })

  it('moves to another column at the requested place', () => {
    expect(ids(placeInBoard(board, 'a', { statusId: 's-doing', placement: { beforeId: 'c' } }))).toEqual([['b'], ['a', 'c'], []])
    expect(ids(placeInBoard(board, 'a', { statusId: 's-done', placement: { position: 'last' } }))).toEqual([['b'], ['c'], ['a']])
  })

  it('leaves the board alone for an unknown card', () => {
    expect(placeInBoard(board, 'zzz', { placement: { position: 'first' } })).toBe(board)
  })
})
