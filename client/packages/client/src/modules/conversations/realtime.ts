import type { ConversationChange, Message } from '@penfii/contracts'
import { api } from '../../api'
import { dedupeMessages } from './messages'

const CHANGE_PAGE = 50
const HISTORY_PAGE = 50

export type TimelineSyncPlan = {
  /** Already on screen, and absent from the newest pages, so the newest fetch did not refresh them. */
  refreshLoadedIds: string[]
  /** A create never appeared on the root timeline: a thread reply, whose root may be an older loaded message. */
  refreshOlderReplyCounts: boolean
}

/**
 * Every change in the batch is considered. A newest-page fetch covers messages inside that window;
 * loaded messages outside it, and thread replies that only affect an older root, are named here.
 */
export function planTimelineSync(
  loaded: readonly { id: string }[],
  fetched: readonly { id: string }[],
  changes: readonly { messageId: string; changeKind: string }[],
): TimelineSyncPlan {
  const fetchedIds = new Set(fetched.map((message) => message.id))
  const loadedIds = new Set(loaded.map((message) => message.id))
  const refreshLoadedIds: string[] = []
  const seen = new Set<string>()
  let refreshOlderReplyCounts = false
  for (const change of changes) {
    if (loadedIds.has(change.messageId) && !fetchedIds.has(change.messageId)) {
      if (!seen.has(change.messageId)) {
        seen.add(change.messageId)
        refreshLoadedIds.push(change.messageId)
      }
      continue
    }
    if (
      !loadedIds.has(change.messageId) &&
      !fetchedIds.has(change.messageId) &&
      change.changeKind === 'message.created'
    ) {
      refreshOlderReplyCounts = true
    }
  }
  return { refreshLoadedIds, refreshOlderReplyCounts }
}

/** Pages the change log after `afterSeq` until a short page. The cursor is the last seq, and every row is kept. */
export async function readAllChanges(conversationId: string, afterSeq: number): Promise<ConversationChange[]> {
  const all: ConversationChange[] = []
  let cursor = afterSeq
  for (let pageIndex = 0; pageIndex < 100; pageIndex += 1) {
    const page = await api.listChanges(conversationId, cursor)
    if (page.length === 0) break
    all.push(...page)
    const next = page[page.length - 1]?.changeSeq ?? cursor
    if (next <= cursor || page.length < CHANGE_PAGE) break
    cursor = next
  }
  return all
}

/**
 * Newest root messages, walking one page older only while the page does not overlap what is already loaded.
 * One new message is a single page. A burst longer than a page is followed until it meets the loaded window.
 */
export async function loadNewestOverlap(conversationId: string, loaded: readonly Message[]): Promise<Message[]> {
  const loadedIds = new Set(loaded.map((message) => message.id))
  const out: Message[] = []
  let beforeSeq: number | undefined
  for (let pageIndex = 0; pageIndex < 20; pageIndex += 1) {
    const page = dedupeMessages(await api.listMessages(conversationId, { beforeSeq, limit: HISTORY_PAGE }))
    out.push(...page)
    if (page.length < HISTORY_PAGE || loaded.length === 0) break
    if (page.some((message) => loadedIds.has(message.id))) break
    const oldest = page[0]
    if (!oldest) break
    beforeSeq = oldest.streamSeq
  }
  return dedupeMessages(out)
}

/** Re-reads loaded root messages older than `fetched`, so a thread reply can update their reply counts. */
export async function refreshOlderLoaded(
  conversationId: string,
  loaded: readonly Message[],
  fetched: readonly Message[],
): Promise<Message[]> {
  if (loaded.length === 0 || fetched.length === 0) return []
  const oldestFetched = fetched.reduce((min, message) => Math.min(min, message.streamSeq), Number.POSITIVE_INFINITY)
  const older = loaded.filter((message) => message.streamSeq < oldestFetched)
  if (older.length === 0) return []
  const oldestLoaded = older.reduce((min, message) => Math.min(min, message.streamSeq), Number.POSITIVE_INFINITY)
  const out: Message[] = []
  let beforeSeq = oldestFetched
  for (let pageIndex = 0; pageIndex < 20; pageIndex += 1) {
    const page = dedupeMessages(await api.listMessages(conversationId, { beforeSeq, limit: HISTORY_PAGE }))
    out.push(...page)
    const pageOldest = page[0]
    if (!pageOldest || page.length < HISTORY_PAGE || pageOldest.streamSeq <= oldestLoaded) break
    beforeSeq = pageOldest.streamSeq
  }
  return dedupeMessages(out)
}
