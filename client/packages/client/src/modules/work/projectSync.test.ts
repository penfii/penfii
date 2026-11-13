// @vitest-environment jsdom
import { QueryClient } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../../queryKeys'
import { FakeWebSocket, installFakeWebSocket } from '../../test/fakeWebSocket'
import { acquireProjectLive, closeAllProjectLive } from './live'
import { noteOwnChange } from './refresh'
import { BATCH_MS, handleProjectFrame, helloFor, newSyncState } from './projectSync'

function spy() {
  const client = new QueryClient()
  const invalidate = vi.spyOn(client, 'invalidateQueries')
  const keys = () => invalidate.mock.calls.map(([filter]) => JSON.stringify(filter?.queryKey))
  return { client, invalidate, keys, reset: () => invalidate.mockClear() }
}
const key = (value: readonly unknown[]) => JSON.stringify(value)

describe('applying the frames of a Project socket', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('reads what the Project screens show once when the first connection is ready (a Sprint change in between has no record), and keeps the head as its position', () => {
    const { client, keys } = spy()
    const state = newSyncState()
    expect(helloFor(state)).toEqual({ v: 1, type: 'hello', afterChangeSeq: 0 })
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 7 })
    expect(keys()).toEqual([
      key(queryKeys.projectBoard('p')),
      key(queryKeys.projectPlanning('p')),
      key(queryKeys.projectSchedule('p')),
      key(queryKeys.projectCalendar('p')),
      key(queryKeys.projectWorkflow('p')),
    ])
    expect(state.lastSeq).toBe(7)
  })

  it('refreshes the Ticket a change names, moves its position forward, and ignores a change it already holds', () => {
    const { client, keys, reset } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 7 })
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 8, changeKind: 'ticket.updated', ticketRevisionNo: 3 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toContain(key(queryKeys.ticket('t1')))
    expect(state.lastSeq).toBe(8)
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 8 })
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 5 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toEqual([])
    expect(state.lastSeq).toBe(8)
  })

  it('reads the whole Project again when a number is missing, and goes on from the new one', () => {
    const { client, keys, reset } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 3 })
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't9', changeSeq: 9 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toContain(key(queryKeys.projectCalendar('p')))
    expect(state.lastSeq).toBe(9)
    // The next number in line is an ordinary change again: only its Ticket is read.
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't9', changeSeq: 10 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).not.toContain(key(queryKeys.projectCalendar('p')))
  })

  it('reads the whole Project again on resync_required and takes the head of the next ready as its position', () => {
    const { client, keys, reset } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 7 })
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.resync_required', projectId: 'p' })
    const all = keys()
    for (const expected of [
      queryKeys.projectBoard('p'), queryKeys.projectPlanning('p'), queryKeys.projectSchedule('p'), queryKeys.all.ticket,
      queryKeys.all.ticketComments, queryKeys.all.ticketAttachments, queryKeys.all.ticketHistory, queryKeys.all.ticketDependencies,
      queryKeys.all.sprintResults, queryKeys.all.sprintReport, queryKeys.all.scopeDataset,
    ]) expect(all, key(expected)).toContain(key(expected))
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 3 })
    expect(state.lastSeq).toBe(3)
  })

  it('reads the Project again when it reconnects without a position, or when the server is behind it', () => {
    const { client, keys, reset } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 0 })
    helloFor(state)
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 4 })
    expect(keys()).toContain(key(queryKeys.projectCalendar('p')))

    const behind = newSyncState()
    handleProjectFrame(client, 'p', behind, { type: 'project.ready', projectId: 'p', headSeq: 10 })
    helloFor(behind)
    reset()
    handleProjectFrame(client, 'p', behind, { type: 'project.ready', projectId: 'p', headSeq: 6 })
    expect(keys()).toContain(key(queryKeys.projectCalendar('p')))
    expect(behind.lastSeq).toBe(6)
  })

  it('ignores frames of another Project and frames it does not understand', () => {
    const { client, invalidate } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.resync_required', projectId: 'other' })
    handleProjectFrame(client, 'p', state, null)
    handleProjectFrame(client, 'p', state, 'text')
    handleProjectFrame(client, 'p', state, { type: 'project.unknown', projectId: 'p' })
    expect(invalidate).not.toHaveBeenCalled()
  })
})

describe('the shared Project socket', () => {
  let sockets: FakeWebSocket[]
  beforeEach(() => {
    vi.useFakeTimers()
    sockets = installFakeWebSocket()
  })
  afterEach(() => {
    closeAllProjectLive()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('is one socket for every screen of the Project, kept for a moment after the last one leaves', () => {
    const { client } = spy()
    const board = acquireProjectLive(client, 'p')
    const timeline = acquireProjectLive(client, 'p')
    vi.advanceTimersByTime(1)
    expect(sockets).toHaveLength(1)
    board()
    timeline()
    // Moving to another screen of the Project: the next one takes it over before it closes.
    const planning = acquireProjectLive(client, 'p')
    vi.advanceTimersByTime(6000)
    expect(sockets).toHaveLength(1)
    expect(sockets[0]?.readyState).toBe(FakeWebSocket.CONNECTING)
    planning()
    vi.advanceTimersByTime(6000)
    expect(sockets[0]?.readyState).toBe(FakeWebSocket.CLOSED)
  })

  it('asks to continue from the last position it holds when it reconnects, and the replayed change reaches the Ticket', () => {
    const { client, keys, reset } = spy()
    const release = acquireProjectLive(client, 'p')
    vi.advanceTimersByTime(1)
    const first = sockets[0] as FakeWebSocket
    first.open()
    expect(first.sent).toEqual([{ v: 1, type: 'hello', afterChangeSeq: 0 }])
    first.receive({ type: 'project.ready', projectId: 'p', headSeq: 12 })
    first.drop()
    vi.advanceTimersByTime(1600)
    const second = sockets[1] as FakeWebSocket
    second.open()
    expect(second.sent).toEqual([{ v: 1, type: 'hello', afterChangeSeq: 12 }])
    reset()
    second.receive({ type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 13, ticketRevisionNo: 2, changeKind: 'ticket.updated' })
    second.receive({ type: 'project.ready', projectId: 'p', headSeq: 13 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toContain(key(queryKeys.ticket('t1')))
    // Caught up from the feed: the whole Project was not read again.
    expect(keys()).not.toContain(key(queryKeys.projectCalendar('p')))
    release()
  })

  it('stops reconnecting when it is closed for good', () => {
    const { client } = spy()
    acquireProjectLive(client, 'p')
    vi.advanceTimersByTime(1)
    closeAllProjectLive()
    vi.advanceTimersByTime(10000)
    expect(sockets).toHaveLength(1)
  })
})

describe('reading changes together', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('reads the Board once for a bulk change of a hundred Tickets', () => {
    const { client, invalidate, keys } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 0 })
    invalidate.mockClear()
    for (let n = 1; n <= 100; n += 1) {
      handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: `t${String(n)}`, changeSeq: n, changeKind: 'ticket.trashed', ticketRevisionNo: 2 })
    }
    expect(invalidate).not.toHaveBeenCalled()
    vi.advanceTimersByTime(BATCH_MS)
    const board = keys().filter((candidate) => candidate === key(queryKeys.projectBoard('p')))
    expect(board).toHaveLength(1)
    expect(keys().filter((candidate) => candidate === key(queryKeys.projectSchedule('p')))).toHaveLength(1)
  })

  it('touches only the Ticket\'s own tabs for a comment, and not the Board', () => {
    const { client, keys, reset } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 0 })
    reset()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 1, changeKind: 'ticket.commented', ticketRevisionNo: 1 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toContain(key(queryKeys.ticketComments('t1')))
    expect(keys()).not.toContain(key(queryKeys.projectBoard('p')))
    expect(keys()).not.toContain(key(queryKeys.all.ticket))
  })

  it('does not read again what this tab changed itself, but does read a comment of the same revision', () => {
    const { client, invalidate, keys } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 0 })
    invalidate.mockClear()
    noteOwnChange('t1', 4)
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 1, changeKind: 'ticket.status_changed', ticketRevisionNo: 4 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(invalidate).not.toHaveBeenCalled()
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 2, changeKind: 'ticket.commented', ticketRevisionNo: 4 })
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 3, changeKind: 'ticket.status_changed', ticketRevisionNo: 5 })
    vi.advanceTimersByTime(BATCH_MS)
    expect(keys()).toContain(key(queryKeys.ticketComments('t1')))
    expect(keys()).toContain(key(queryKeys.projectBoard('p')))
  })

  it('drops what it collected when the whole Project is read instead', () => {
    const { client, invalidate } = spy()
    const state = newSyncState()
    handleProjectFrame(client, 'p', state, { type: 'project.ready', projectId: 'p', headSeq: 0 })
    handleProjectFrame(client, 'p', state, { type: 'project.ticket.change', projectId: 'p', ticketId: 't1', changeSeq: 1, changeKind: 'ticket.updated', ticketRevisionNo: 2 })
    handleProjectFrame(client, 'p', state, { type: 'project.resync_required', projectId: 'p' })
    invalidate.mockClear()
    vi.advanceTimersByTime(BATCH_MS)
    expect(invalidate).not.toHaveBeenCalled()
  })
})
