import { QueryClient, partialMatchKey } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

vi.mock('./api', () => ({ api: {} }))

import { queries } from './queries'
import { queryKeys } from './queryKeys'

describe('query keys', () => {
  it('builds every per-id key under the prefix used to invalidate it', () => {
    const { all } = queryKeys
    const pairs: Array<[readonly unknown[], readonly unknown[]]> = [
      [all.space, queryKeys.space('w')],
      [all.ticket, queryKeys.ticket('t')],
      [all.ticketChildren, queryKeys.ticketChildren('t')],
      [all.ticketDependencies, queryKeys.ticketDependencies('t')],
      [all.ticketRevisions('t'), queryKeys.ticketRevision('t', 3)],
      [all.scopeDataset, queryKeys.scopeDataset('w', { kind: 'WORKSPACE' }, 'ALL', [])],
      [all.sprintResults, queryKeys.sprintResults('s')],
      [all.sprintReport, queryKeys.sprintReport('s')],
      [all.favorites, queryKeys.favorites('w')],
      [all.later, queryKeys.later('w', 'ACTIVE')],
      [all.inbox, queryKeys.inbox('w', 'all', 'all')],
      [all.inboxUnread, queryKeys.inboxUnread('w')],
      [all.docTree('w'), queryKeys.docTree('w', 'p')],
      [all.adminAudit('w'), queryKeys.adminAudit('w', '')],
      [queryKeys.me, queryKeys.sessions],
    ]
    for (const [prefix, key] of pairs) expect(partialMatchKey(key, prefix)).toBe(true)
  })

  it('keeps kinds that share a word apart', () => {
    expect(partialMatchKey(queryKeys.ticketChildren('t'), queryKeys.all.ticket)).toBe(false)
    expect(partialMatchKey(queryKeys.inboxUnread('w'), queryKeys.all.inbox)).toBe(false)
    expect(partialMatchKey(queryKeys.docTree('other'), queryKeys.all.docTree('w'))).toBe(false)
  })

  it('gives the shared definitions the same keys the invalidations use', () => {
    expect(queries.space('w').queryKey).toEqual(queryKeys.space('w'))
    expect(queries.projectWorkflow('p').queryKey).toEqual(queryKeys.projectWorkflow('p'))
    expect(queries.scopeDataset('w', { kind: 'TEAM', teamId: 'tm' }, 'ASSIGNED', ['a', 'b']).queryKey).toEqual(queryKeys.scopeDataset('w', { kind: 'TEAM', teamId: 'tm' }, 'ASSIGNED', ['a', 'b']))
    expect(queries.docTree('w').queryKey).toEqual(queryKeys.docTree('w', null))
  })

  it('marks only the matching entries stale', async () => {
    const client = new QueryClient()
    for (const key of [queryKeys.ticket('a'), queryKeys.ticket('b'), queryKeys.ticketChildren('a'), queryKeys.space('w')]) client.setQueryData(key, {})
    await client.invalidateQueries({ queryKey: queryKeys.all.ticket })
    const stale = (key: readonly unknown[]) => client.getQueryState(key)?.isInvalidated
    expect(stale(queryKeys.ticket('a'))).toBe(true)
    expect(stale(queryKeys.ticket('b'))).toBe(true)
    expect(stale(queryKeys.ticketChildren('a'))).toBe(false)
    expect(stale(queryKeys.space('w'))).toBe(false)
  })
})
