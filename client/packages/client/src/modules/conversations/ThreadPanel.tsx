import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import type { Message } from '@penfii/contracts'
import { isApiError, isNotFound, newIdempotencyKey } from '@penfii/api-client'
import { Link } from 'react-router'
import { useLeaveOverlay } from '../../shell/overlayTrail'
import { api } from '../../api'
import { WatchButton } from '../attention/WatchButton'
import { dayLabel, formatStamp } from '../../format'
import { displayedMinute } from './group'
import { useI18n } from '../../i18n/I18nProvider'
import type { DisplayNames } from '../../richtext'
import { errorText } from '../../ui/errorText'
import { useConversationLive, useRegisteredMessage } from './live'
import { MessageBody } from './MessageBody'
import { MessageRow } from './MessageRow'
import { AuthorCard } from '../people/AuthorCard'
import { loadThreadPages, mergeMessages, presentMessage } from './messages'
import { OutgoingRow } from './OutgoingRow'
import { MessageReference } from './ReplyQuote'
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
import { revealMessage, useFollowScroll } from './scroll'
import { useVisibleSeqs } from './useVisibleSeqs'
import { CloseIcon } from '../../design/icons'

export type ThreadHandle = {
  upsert: (message: Message) => void
  noteOwnSeq: (seq: number) => void
}

export function ThreadPanel({
  ref,
  workspaceId,
  conversationId,
  rootId,
  currentUserId,
  channelRole,
  names,
  conversationHref,
  attempt,
  onRetry,
  onNotFound,
  commentOnId,
  onComment,
  onClearComment,
}: {
  ref?: Ref<ThreadHandle>
  workspaceId: string
  conversationId: string
  rootId: string
  currentUserId: string
  /** The person's role in the channel; a MANAGER may delete anyone's message. */
  channelRole?: string | null
  names: DisplayNames
  conversationHref?: string
  attempt: OutboundAttempt | null
  onRetry: () => void
  onNotFound: () => void
  commentOnId: string | null
  onComment: (messageId: string) => void
  onClearComment: () => void
}) {
  const { locale, t } = useI18n()
  const leave = useLeaveOverlay()
  const closeTo = conversationHref ?? `/w/${workspaceId}/c/${conversationId}`
  const live = useConversationLive()
  const publishRef = useRef(live.publish)
  publishRef.current = live.publish
  const onNotFoundRef = useRef(onNotFound)
  onNotFoundRef.current = onNotFound
  const rootMessage = useRegisteredMessage(rootId)
  const [messages, setMessages] = useState<Message[]>([])
  const [read, setRead] = useState<ReadState | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [scrollRoot, setScrollRoot] = useState<HTMLDivElement | null>(null)
  const [ownSeqs, setOwnSeqs] = useState<number[]>([])
  const snapshotEpoch = useRef(0)
  const visible = useVisibleSeqs()
  const follow = useFollowScroll(scrollRoot, messages.length, rootId)

  const upsert = useCallback((message: Message) => {
    snapshotEpoch.current += 1
    setMessages((current) => mergeMessages(current, [presentMessage(message)]))
  }, [])
  const noteOwnSeq = useCallback((seq: number) => {
    follow.pin()
    setOwnSeqs((current) => (current.includes(seq) ? current : [...current, seq]))
  }, [follow])
  useImperativeHandle(ref, () => ({ upsert, noteOwnSeq }), [noteOwnSeq, upsert])

  useEffect(() => {
    publishRef.current(messages)
    if (rootMessage) publishRef.current([rootMessage])
  }, [messages, rootMessage])

  useEffect(() => {
    let cancelled = false
    const requestEpoch = ++snapshotEpoch.current
    setLoading(true)
    void Promise.all([loadThreadPages(rootId), api.getThreadRead(rootId).catch(() => null)])
      .then(([page, cursor]) => {
        if (cancelled) return
        if (requestEpoch === snapshotEpoch.current) setMessages(page.messages)
        setRead(initialReadState({
          generation: cursor?.generation ?? page.generation,
          readThroughSeq: cursor?.readThroughSeq ?? 0,
          manualUnreadFromSeq: cursor?.manualUnreadFromSeq,
        }))
        setLoading(false)
      })
      .catch((caught: unknown) => {
        if (cancelled) return
        if (isNotFound(caught)) onNotFoundRef.current()
        else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [rootId, t])

  const seenChange = useRef(0)
  useEffect(() => {
    if (live.changeCount === seenChange.current) return
    seenChange.current = live.changeCount
    const requestEpoch = ++snapshotEpoch.current
    void loadThreadPages(rootId)
      .then((page) => {
        if (requestEpoch !== snapshotEpoch.current) return
        setMessages((current) => mergeMessages(current, page.messages))
        if (page.generation != null) {
          setRead((current) => (current && current.generation == null ? { ...current, generation: page.generation } : current))
        }
      })
      .catch((caught: unknown) => {
        if (isNotFound(caught)) onNotFoundRef.current()
      })
  }, [live.changeCount, rootId])

  const seenResync = useRef(0)
  useEffect(() => {
    if (live.resyncCount === seenResync.current) return
    seenResync.current = live.resyncCount
    if (live.resyncCount === 0) return
    const requestEpoch = ++snapshotEpoch.current
    void loadThreadPages(rootId)
      .then((page) => {
        if (requestEpoch !== snapshotEpoch.current) return
        setMessages((current) => mergeMessages(current, page.messages))
      })
      .catch((caught: unknown) => {
        if (isNotFound(caught)) onNotFoundRef.current()
      })
  }, [live.resyncCount, rootId])

  const postRead = useCallback(
    (body: Parameters<typeof api.markThreadRead>[1], key: string) => api.markThreadRead(rootId, body, key),
    [rootId],
  )
  const refreshGeneration = useCallback(async () => {
    try {
      const cursor = await api.getThreadRead(rootId)
      return cursor.generation
    } catch (caught) {
      if (isNotFound(caught)) onNotFoundRef.current()
      return null
    }
  }, [rootId])

  const { bumpEpoch } = useAutoRead({
    active: read != null && read.generation != null,
    generation: read?.generation ?? null,
    readThroughSeq: read?.readThroughSeq ?? 0,
    visibleSeqs: visible.seqs,
    ownSeqs,
    loadedSeqs: messages.map((message) => message.streamSeq),
    hasOlder: false,
    manualUnread: read?.manualUnread ?? false,
    post: postRead,
    refreshGeneration,
    onSuccess: (result) => {
      if (result.readThroughSeq == null) return
      const readThroughSeq = result.readThroughSeq
      setRead((current) => (current ? applyAutoReadSuccess(current, { generation: result.generation, readThroughSeq }) : current))
    },
    onStale: (nextGeneration) => setRead((current) => (current ? applyStaleRead(current, nextGeneration) : current)),
  })

  function markUnread(fromSeq: number) {
    bumpEpoch()
    setRead((current) => (current ? applyManualUnread(current, fromSeq) : current))
    void api.markThreadUnread(rootId, { fromSeq }, newIdempotencyKey()).catch((caught: unknown) => {
      if (isNotFound(caught)) {
        setRead((current) => (current ? clearManualUnread(current) : current))
        onNotFound()
      } else if (isApiError(caught)) setError(caught.message)
    })
  }

  const origin = messages.find((message) => message.id === rootId) ?? rootMessage
  const comments = messages.filter((message) => message.threadRootId === rootId)
  const commentCount = comments.filter((message) => !message.deleted).length
  const referenceCounts = new Map<string, number>()
  for (const message of comments) {
    if (message.deleted || !message.replyToId) continue
    referenceCounts.set(message.replyToId, (referenceCounts.get(message.replyToId) ?? 0) + 1)
  }
  const commentTarget = commentOnId ? messages.find((message) => message.id === commentOnId) ?? null : null

  function focusMessages(ids: string[]) {
    if (!scrollRoot || ids.length === 0) return
    const [first, ...rest] = ids
    if (first) revealMessage(scrollRoot, first)
    for (const id of rest) {
      const node = scrollRoot.querySelector(`[data-message-id="${CSS.escape(id)}"]`)
      if (!(node instanceof HTMLElement)) continue
      node.classList.add('flash')
      window.setTimeout(() => node.classList.remove('flash'), 1400)
    }
  }
  let previousDay = ''
  const unread = read?.manualUnread ?? false
  const originName = origin
    ? names[origin.authorId] || origin.authorDisplayName || origin.authorId
    : ''

  return (
    <section className="transcript thread-panel">
      <header className="transcript-head">
        <div className="head-cluster">
          <h2>{t('thread.title')}</h2>
          <WatchButton kind="THREAD" targetId={rootId} />
          {unread ? <span className="unread-flag">{t('conversation.unread')}</span> : null}
        </div>
        <Link
          className="icon-button thread-close"
          to={closeTo}
          onClick={(event) => {
            // On a phone this steps back out of the thread instead of adding the channel after it (see overlayTrail).
            if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
            event.preventDefault()
            leave(closeTo)
          }}
          aria-label={t('thread.close')} title={t('thread.close')}><CloseIcon /></Link>
      </header>
      <div className="scroller" ref={setScrollRoot}>
        {origin ? (
          <article className="comment-origin" data-message-id={origin.id}>
            <header className="message-meta">
              <AuthorCard workspaceId={workspaceId} userId={origin.authorId} name={originName} authorKind={origin.authorKind} />
              <span aria-hidden="true">·</span>
              <time dateTime={origin.createdAt} title={formatStamp(origin.createdAt, locale)}>
                {displayedMinute(origin.createdAt) ?? origin.createdAt}
              </time>
            </header>
            <MessageBody message={origin} names={names} />
          </article>
        ) : null}
        <p className="comment-count">{t('message.replies', { count: String(commentCount) })}</p>
        {loading ? <p>{t('loading')}</p> : null}
        {comments.map((message, index) => {
          const day = dayLabel(message.createdAt, locale)
          const separator = index > 0 && Boolean(day) && day !== previousDay
          if (day) previousDay = day
          const previous = index > 0 ? comments[index - 1] : undefined
          const grouped = !separator && continuesGroup(previous, message)
          const referenced = message.replyToId
            ? messages.find((item) => item.id === message.replyToId) ?? null
            : undefined
          const replies = comments.filter((item) => !item.deleted && item.replyToId === message.id).map((item) => item.id)
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
                referenced={referenced}
                commentCount={referenceCounts.get(message.id) ?? 0}
                onShowComments={replies.length > 0 ? () => focusMessages(replies) : undefined}
                onOpenReference={(messageId) => focusMessages([messageId])}
                onComment={() => onComment(message.id)}
                commentActive={commentOnId === message.id}
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
      {commentOnId ? (
        <div className="comment-target">
          <div className="comment-target-head">
            <span>{t('thread.commenting')}</span>
            <button type="button" onClick={onClearComment}>{t('thread.clearComment')}</button>
          </div>
          <MessageReference
            name={commentTarget ? names[commentTarget.authorId] || commentTarget.authorDisplayName || commentTarget.authorId : ''}
            message={commentTarget}
            onOpen={commentTarget ? () => focusMessages([commentTarget.id]) : undefined}
          />
        </div>
      ) : null}
      {error ? <p className="form-error">{error}</p> : null}
    </section>
  )
}
