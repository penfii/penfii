import { useCallback, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { ConversationPeer } from '@penfii/contracts'
import { api } from '../../api'
import { queries } from '../../queries'
import type { MentionCandidate } from '../../richtext'

/** Who `@` may offer in a conversation: a channel (its members first, then the rest of the workspace) or a DM (the people in it). */
export type MentionScope = { kind: 'channel'; channelId: string } | { kind: 'dm'; peers: readonly ConversationPeer[] }

/** People `@` alone lists (no query yet): enough to pick from without a wall of names. */
const EMPTY_QUERY_LIMIT = 8

const lower = (value: string | null | undefined) => (value ?? '').toLowerCase()

/**
 * How well a person fits what was typed, lower is better; null when they do not fit. Only what a
 * person would type to find someone counts: the name, the username and the part of the email before
 * the `@`. The email's domain is shared by everybody at a company and says nothing about who this is.
 */
export function mentionRank(query: string, person: { name: string; username?: string | null; email?: string | null }): number | null {
  const q = lower(query).trim()
  if (!q) return 0
  const name = lower(person.name)
  const username = lower(person.username)
  const local = lower(person.email?.split('@')[0])
  if (name.startsWith(q)) return 0
  if (username.startsWith(q)) return 1
  if (local.startsWith(q)) return 2
  if (name.split(/[\s\-_.]+/).some((word) => word.startsWith(q))) return 3
  if (name.includes(q)) return 4
  if (username.includes(q) || local.includes(q)) return 5
  return null
}

function byName(a: MentionCandidate, b: MentionCandidate): number {
  return a.name.localeCompare(b.name)
}

/**
 * The search the Composer gives its editor. It only lists; it does not decide who is notified (the
 * server does that from what each person may read). Members of the channel come first; other active
 * people of the workspace follow and are marked as not members.
 */
export function useMentionSearch(workspaceId: string, scope: MentionScope | null, selfId: string | null): (query: string) => Promise<MentionCandidate[]> {
  const queryClient = useQueryClient()
  // The editor decides once whether it offers mentions, and the conversation (so the scope) is often
  // not known yet when it is made. The function therefore always exists and reads the scope when
  // it is called; it offers nobody until the scope is known.
  const scopeRef = useRef(scope)
  scopeRef.current = scope
  return useCallback(
    async (query: string): Promise<MentionCandidate[]> => {
      const current = scopeRef.current
      if (!current) return []
      const q = query.trim()
      if (current.kind === 'dm') {
        return current.peers
          .filter((peer) => peer.id !== selfId)
          .map((peer) => ({ peer, rank: mentionRank(q, { name: peer.displayName, username: peer.username }) }))
          .flatMap(({ peer, rank }) => (rank === null ? [] : [{ rank, c: { id: peer.id, name: peer.displayName, detail: peer.username ?? undefined, member: true } }]))
          .sort((a, b) => a.rank - b.rank || byName(a.c, b.c))
          .map(({ c }) => c)
      }
      const members = await queryClient.ensureQueryData(queries.channelMembers(current.channelId))
      const memberIds = new Set(members.map((m) => m.userId))
      if (q === '') {
        // `@` alone: the people of this channel, nobody from outside it.
        return members
          .filter((m) => m.userId !== selfId)
          .map<MentionCandidate>((m) => ({ id: m.userId, name: m.displayName, member: true }))
          .sort(byName)
          .slice(0, EMPTY_QUERY_LIMIT)
      }
      const found = await api.searchDirectory(workspaceId, { q, types: 'person', limit: 20 })
      const ranked = found.results
        .flatMap((hit) => (hit.person ? [hit.person] : []))
        .filter((person) => person.id !== selfId)
        .flatMap((person) => {
          const rank = mentionRank(q, { name: person.displayName, username: person.username, email: person.email })
          return rank === null ? [] : [{ rank, c: { id: person.id, name: person.displayName, detail: person.username ?? person.organizationName, member: memberIds.has(person.id) } satisfies MentionCandidate }]
        })
        .sort((a, b) => a.rank - b.rank || byName(a.c, b.c))
        .map(({ c }) => c)
      return [...ranked.filter((c) => c.member), ...ranked.filter((c) => !c.member)]
    },
    [workspaceId, selfId, queryClient],
  )
}
