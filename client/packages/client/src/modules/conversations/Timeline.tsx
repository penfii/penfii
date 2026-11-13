import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { ConversationChange, Message } from '@penfii/contracts'
import { isApiError, isNotFound, newIdempotencyKey } from '@penfii/api-client'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { dayLabel } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import type { DisplayNames } from '../../richtext'
import { errorText } from '../../ui/errorText'
import { useConversationLive } from './live'
import { MessageRow } from './MessageRow'
import { dedupeMessages, mergeMessages, presentMessage } from './messages'
import { loadNewestOverlap, planTimelineSync, refreshOlderLoaded } from './realtime'
import { OutgoingRow } from './OutgoingRow'
import {
  applyAutoReadSuccess,
  applyManualUnread,
  applyStaleRead,
  clearManualUnread,
  initialReadState,
  type ReadState,
} from './readState'
import { useAutoRead } from './useAutoRead'
import type { OutboundAttempt } from './useOutboundMessage'
import { continuesGroup } from './group'
import { useFollowScroll } from './scroll'
import { useVisibleSeqs } from './useVisibleSeqs'
import { useShown } from './useShown'

/** Loads the previous page when scrolling brings it into view; re-keyed per page so a still-visible top retriggers. */
function OlderSentinel({ root, onReach }: { root: HTMLElement | null; onReach: () => void }) {
  const ref = useShown(root, 0, (_seq, shown) => {
    if (shown) onReach()
  })
  return <div ref={ref} className="older-sentinel" aria-hidden="true" />
}

export type TimelineHandle = {
  upsert: (message: Message) => void
  noteOwnSeq: (seq: number) => void
  noteReply: (rootId: string) => void
}

export function Timeline({
  ref,
  workspaceId,
  conversationId,
  currentUserId,
  channelRole,
  names,
  showDmWarning,
  heading,
  threadBase,
  activeThreadId = null,
  attempt,
  onRetry,
  onNotFound,
}: {
  ref?: Ref<TimelineHandle>
  workspaceId: string
  conversationId: string
  currentUserId: string
  /** The person's role in the channel; a MANAGER may delete anyone's message. */
  channelRole?: string | null
  names: DisplayNames
  showDmWarning: boolean
  heading?: string
  threadBase?: string
  activeThreadId?: string | null
  attempt: OutboundAttempt | null
  onRetry: () => void
  onNotFound: () => void
}) {
  const { locale, t } = useI18n()
  const queryClient = useQueryClient()
  const live = useConversationLive()
  const publishRef = useRef(live.publish)
  publishRef.current = live.publish
  const onNotFoundRef = useRef(onNotFound)
  onNotFoundRef.current = onNotFound
  const [messages, setMessages] = useState<Message[]>([])
  const messagesRef = useRef<Message[]>([])
  messagesRef.current = messages
  const [hasOlder, setHasOlder] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [scrollRoot, setScrollRoot] = useState<HTMLDivElement | null>(null)
  const [ownSeqs, setOwnSeqs] = useState<number[]>([])
  const [read, setRead] = useState<ReadState | null>(null)
  const [generation, setGeneration] = useState<number | null>(null)
  const [title, setTitle] = useState<string | null>(null)
  const snapshotEpoch = useRef(0)
  const loadGeneration = useRef(0)
  const historyReady = useRef(false)
  const olderRequested = useRef<number | null>(null)
  const appliedCount = useRef(0)
  const applyChain = useRef(Promise.resolve())
  const visible = useVisibleSeqs()
  const follow = useFollowScroll(scrollRoot, messages.length, conversationId)

  const upsert = useCallback((message: Message) => {
    snapshotEpoch.current += 1
    setMessages((current) => mergeMessages(current, [presentMessage(message)]))
  }, [])
  const noteOwnSeq = useCallback((seq: number) => {
    follow.pin()
    setOwnSeqs((current) => (current.includes(seq) ? current : [...current, seq]))
  }, [follow])
  const noteReply = useCallback((rootId: string) => {
    snapshotEpoch.current += 1
    setMessages((current) => current.map((message) => (
      message.id === rootId ? { ...message, replyCount: message.replyCount + 1 } : message
    )))
  }, [])
  useImperativeHandle(ref, () => ({ upsert, noteOwnSeq, noteReply }), [noteOwnSeq, noteReply, upsert])

  const changesRef = useRef(live.changes)
  changesRef.current = live.changes

  useEffect(() => {
    publishRef.current(messages)
  }, [messages])

  const applyBatch = useCallback((batch: readonly ConversationChange[], gen: number) => {
    if (batch.length === 0 || gen !== loadGeneration.current) return Promise.resolve()
    return (async () => {
      try {
        const fetched = await loadNewestOverlap(conversationId, messagesRef.current)
        if (gen !== loadGeneration.current) return
        const loaded = messagesRef.current
        const plan = planTimelineSync(loaded, fetched, batch)
        const extra: Message[] = []
        for (const id of plan.refreshLoadedIds) {
          const current = loaded.find((message) => message.id === id)
          if (!current) continue
          const page = dedupeMessages(await api.listMessages(conversationId, { beforeSeq: current.streamSeq + 1, limit: 1 }))
          const updated = page.find((message) => message.id === id)
          if (updated) extra.push(updated)
        }
        if (plan.refreshOlderReplyCounts) extra.push(...await refreshOlderLoaded(conversationId, loaded, fetched))
        if (gen !== loadGeneration.current) return
        const incoming = dedupeMessages([...fetched, ...extra])
        setMessages((current) => mergeMessages(current, incoming))
      } catch (caught) {
        if (isNotFound(caught)) onNotFoundRef.current()
      }
    })()
  }, [conversationId])

  const schedule = useCallback((batch: readonly ConversationChange[]) => {
    const gen = loadGeneration.current
    applyChain.current = applyChain.current.then(() => applyBatch(batch, gen)).then(() => undefined, () => undefined)
  }, [applyBatch])

  useEffect(() => {
    const gen = ++loadGeneration.current
    historyReady.current = false
    olderRequested.current = null
    appliedCount.current = 0
    setMessages([])
    setHasOlder(false)
    setLoading(true)
    setError(null)
    const conversation = live.snapshot
    if (live.snapshotError) {
      if (isNotFound(live.snapshotError)) onNotFoundRef.current()
      else setError(errorText(live.snapshotError, t('error.generic'), t('error.dbUnavailable')))
      setLoading(false)
      return
    }
    if (!conversation || conversation.id !== conversationId) return
    let cancelled = false
    const requestEpoch = snapshotEpoch.current
    setTitle(conversation.title)
    setGeneration(conversation.generation)
    setRead(initialReadState({
      generation: conversation.generation,
      readThroughSeq: conversation.readThroughSeq,
      manualUnreadFromSeq: conversation.manualUnreadFromSeq,
    }))
    // History is read after the snapshot, so it includes everything up to lastChangeSeq.
    // Changes that arrive while this read is in flight stay queued and are applied on top.
    void api.listMessages(conversationId, { limit: 50 })
      .then((page) => {
        if (cancelled || gen !== loadGeneration.current) return
        const newest = dedupeMessages(page)
        setMessages((current) => (requestEpoch === snapshotEpoch.current ? newest : mergeMessages(current, newest)))
        setHasOlder(page.length >= 50)
        setLoading(false)
        const queued = changesRef.current.slice(appliedCount.current)
        appliedCount.current = changesRef.current.length
        historyReady.current = true
        if (queued.length > 0) schedule(queued)
      })
      .catch((caught: unknown) => {
        if (cancelled || gen !== loadGeneration.current) return
        if (isNotFound(caught)) onNotFoundRef.current()
        else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [conversationId, live.snapshot, live.snapshotError, schedule, t])

  useEffect(() => {
    if (!historyReady.current) return
    const fresh = live.changes.slice(appliedCount.current)
    if (fresh.length === 0) return
    appliedCount.current = live.changes.length
    schedule(fresh)
  }, [live.changes, schedule])

  const seenResync = useRef(0)
  useEffect(() => {
    if (live.resyncCount === seenResync.current) return
    seenResync.current = live.resyncCount
    if (live.resyncCount === 0 || !historyReady.current) return
    const gen = loadGeneration.current
    void api
      .listMessages(conversationId, { limit: 50 })
      .then((page) => {
        if (gen !== loadGeneration.current) return
        setMessages((current) => mergeMessages(current, page))
      })
      .catch((caught: unknown) => {
        if (isNotFound(caught)) onNotFoundRef.current()
      })
  }, [conversationId, live.resyncCount])

  const postRead = useCallback(
    (body: Parameters<typeof api.markConversationRead>[1], key: string) => api.markConversationRead(conversationId, body, key),
    [conversationId],
  )
  const refreshGeneration = useCallback(async () => {
    try {
      const conversation = await api.getConversation(conversationId)
      setGeneration(conversation.generation)
      return conversation.generation
    } catch (caught) {
      if (isNotFound(caught)) onNotFoundRef.current()
      return null
    }
  }, [conversationId])

  const { bumpEpoch } = useAutoRead({
    active: read != null,
    generation: read?.generation ?? generation,
    readThroughSeq: read?.readThroughSeq ?? 0,
    visibleSeqs: visible.seqs,
    ownSeqs,
    loadedSeqs: messages.map((message) => message.streamSeq),
    hasOlder,
    manualUnread: read?.manualUnread ?? false,
    post: postRead,
    refreshGeneration,
    onSuccess: (result) => {
      if (result.readThroughSeq == null) return
      const readThroughSeq = result.readThroughSeq
      setRead((current) => (current ? applyAutoReadSuccess(current, { generation: result.generation, readThroughSeq }) : current))
      void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
    },
    onStale: (nextGeneration) => {
      setRead((current) => (current ? applyStaleRead(current, nextGeneration) : current))
    },
  })

  async function loadOlder() {
    const oldest = messages[0]
    // One request per page boundary: a still-visible sentinel must not refetch the page being loaded.
    if (!oldest || olderRequested.current === oldest.streamSeq) return
    olderRequested.current = oldest.streamSeq
    const requestEpoch = snapshotEpoch.current
    follow.holdScroll()
    try {
      const page = await api.listMessages(conversationId, { beforeSeq: oldest.streamSeq, limit: 50 })
      if (requestEpoch !== snapshotEpoch.current) {
        olderRequested.current = null
        follow.releaseHold()
        return
      }
      setMessages((current) => mergeMessages(current, page))
      setHasOlder(page.length >= 50)
    } catch (caught) {
      olderRequested.current = null
      follow.releaseHold()
      if (isNotFound(caught)) onNotFound()
      else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    }
  }
  const loadOlderRef = useRef(loadOlder)
  loadOlderRef.current = loadOlder
  const reachTop = useCallback(() => void loadOlderRef.current(), [])

  function markUnread(fromSeq: number) {
    bumpEpoch()
    setRead((current) => (current ? applyManualUnread(current, fromSeq) : current))
    void api.markConversationUnread(conversationId, { fromSeq }, newIdempotencyKey()).catch((caught: unknown) => {
      if (isNotFound(caught)) {
        setRead((current) => (current ? clearManualUnread(current) : current))
        onNotFound()
        return
      }
      if (isApiError(caught)) setError(caught.message)
    })
  }

  const own = new Set(ownSeqs)
  const unread =
    (read?.manualUnread ?? false) ||
    messages.some((message) => message.streamSeq > (read?.readThroughSeq ?? 0) && !own.has(message.streamSeq))
  let previousDay = ''

  return (
    <section className="transcript">
      {heading && !unread ? null : (
        <header className="transcript-head">
          {heading ? null : <h2>{title || t('conversation.empty')}</h2>}
          {unread ? <span className="unread-flag">{t('conversation.unread')}</span> : null}
        </header>
      )}
      {showDmWarning ? <p className="dm-warning">{t('dm.history')}</p> : null}
      <div className="scroller" ref={setScrollRoot}>
        {hasOlder && !loading && messages.length > 0 ? (
          <OlderSentinel key={messages[0]?.id} root={scrollRoot} onReach={reachTop} />
        ) : null}
        {loading ? <p>{t('loading')}</p> : null}
        {!loading && messages.length === 0 ? <p>{t('timeline.empty')}</p> : null}
        {messages.map((message, index) => {
          const day = dayLabel(message.createdAt, locale)
          const separator = day && day !== previousDay
          if (separator) previousDay = day
          const previous = index > 0 ? messages[index - 1] : undefined
          const grouped = !separator && continuesGroup(previous, message)
          const base = threadBase ?? `/w/${workspaceId}/c/${conversationId}`
          return (
            <div key={message.id}>
              {separator ? <div className="day-sep">{day}</div> : null}
              <MessageRow
                message={message}
                compact={grouped}
                currentUserId={currentUserId}
                channelRole={channelRole}
                names={names}
                scrollRoot={scrollRoot}
                onVisible={visible.setVisible}
                onMessage={upsert}
                onNotFound={onNotFound}
                onManualUnread={markUnread}
                threadHref={`${base}/t/${message.id}`}
                threadActive={message.id === activeThreadId}
              />
            </div>
          )
        })}
        {attempt ? <OutgoingRow attempt={attempt} userId={currentUserId} names={names} onRetry={onRetry} /> : null}
      </div>
      {follow.pending > 0 ? (
        <button type="button" className="jump-latest" onClick={follow.jump}>
          {t('timeline.newMessages', { count: String(follow.pending) })}
        </button>
      ) : null}
      {error ? <p className="form-error">{error}</p> : null}
    </section>
  )
}
