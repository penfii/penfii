import { ResizeHandle } from '../shell/ResizeHandle'
import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useLocation, useNavigate } from 'react-router'
import { api } from '../api'
import { useI18n } from '../i18n/I18nProvider'
import { queryKeys } from '../queryKeys'
import { isArchivedTeam } from '../modules/spaces/teamState'
import { queries } from '../queries'
import { EMPTY_RICH_TEXT, isBlankDoc, messageBodyBytes, sameRichText, type DisplayNames } from '../richtext'
import { usePaneWidth } from '../shell/usePaneWidth'
import { Button } from '../design/ui'
import { shouldOfferJoin } from '../modules/spaces/joinPolicy'
import { NotFoundState } from '../ui/NotFoundState'
import {
  Composer,
  ConversationLiveProvider,
  ThreadPanel,
  Timeline,
  useOutboundMessage,
  type ThreadHandle,
  type TimelineHandle,
} from '../modules/conversations'
import { useServerDraft } from '../modules/conversations/useServerDraft'
import { useMentionSearch, type MentionScope } from '../modules/conversations/useMentionSearch'

export function ConversationPane({
  workspaceId,
  conversationId,
  threadRootId,
  currentUserId,
  names,
  showDmWarning,
  heading,
  threadBase,
}: {
  workspaceId: string
  conversationId: string
  threadRootId: string | null
  currentUserId: string
  names: DisplayNames
  showDmWarning: boolean
  heading: string
  threadBase: string
}) {
  const { t } = useI18n()
  const location = useLocation()
  const navigate = useNavigate()
  const threadPane = usePaneWidth('thread', {
    threadOpen: true,
    detailOpen: new URLSearchParams(location.search).has('panel'),
    onTooNarrow: () => {
      if (threadRootId) void navigate(threadBase)
    },
  })
  const queryClient = useQueryClient()
  const serverInfo = useQuery(queries.serverInfo())
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const channel = space.data?.channels.find((item) => item.conversationId === conversationId)
  const dm = space.data?.dms.find((item) => item.conversationId === conversationId)
  const mentionScope: MentionScope | null = channel ? { kind: 'channel', channelId: channel.id } : dm ? { kind: 'dm', peers: dm.peers } : null
  const searchMentions = useMentionSearch(workspaceId, mentionScope, currentUserId)
  const mustJoin = channel ? shouldOfferJoin(channel) : false
  // A Team's channels are read-only while the Team is archived (derived by the server from the Team).
  const readOnly = Boolean(channel?.teamId && isArchivedTeam(space.data?.teams.find((item) => item.id === channel.teamId)))
  const readOnlyBar = <div className="read-only-note" role="status">{t('teams.channelReadOnly')}</div>
  const channelRole = channel?.role ?? null
  const [joinFailed, setJoinFailed] = useState(false)
  async function join() {
    if (!channel) return
    setJoinFailed(false)
    try {
      await api.joinChannel(channel.id)
      await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
    } catch {
      setJoinFailed(true)
    }
  }
  const joinBar = (
    <div className="join-bar">
      <p>{t('channel.joinPrompt')}</p>
      <Button type="button" onClick={() => void join()}>{t('channel.join')}</Button>
      {joinFailed ? <p className="form-error" role="alert">{t('error.generic')}</p> : null}
    </div>
  )
  const [channelDoc, setChannelDoc] = useState(EMPTY_RICH_TEXT)
  const [threadDoc, setThreadDoc] = useState(EMPTY_RICH_TEXT)
  const channelOutbound = useOutboundMessage()
  const threadOutbound = useOutboundMessage()
  const timelineRef = useRef<TimelineHandle>(null)
  const threadRef = useRef<ThreadHandle>(null)
  const [commentOnId, setCommentOnId] = useState<string | null>(null)
  const [notFound, setNotFound] = useState(false)

  useEffect(() => {
    setChannelDoc(EMPTY_RICH_TEXT)
    setCommentOnId(null)
    setNotFound(false)
  }, [conversationId])

  useEffect(() => {
    setThreadDoc(EMPTY_RICH_TEXT)
    setCommentOnId(null)
  }, [threadRootId])

  // No draft is read for a channel nobody can write to (the server has none to give), nor before the Space says which it is.
  // Nor where the composer is a Join action: the server answers a draft request for a conversation the caller cannot write with 404.
  const draftsOpen = space.isSuccess && !readOnly && !mustJoin
  useServerDraft('CHANNEL_COMPOSE', draftsOpen ? conversationId : null, channelDoc, setChannelDoc)
  useServerDraft('THREAD_COMPOSE', draftsOpen ? (threadRootId ?? null) : null, threadDoc, setThreadDoc)

  const maxBytes: number | null | undefined = serverInfo.data
    ? serverInfo.data.limits.messageBodyMaxBytes
    : undefined

  const channelFiles = useRef<string[]>([])
  const threadFiles = useRef<string[]>([])

  async function send(which: 'channel' | 'thread', fileIds?: string[]): Promise<boolean> {
    const outbound = which === 'channel' ? channelOutbound : threadOutbound
    const doc = which === 'channel' ? channelDoc : threadDoc
    const setDoc = which === 'channel' ? setChannelDoc : setThreadDoc
    const files = which === 'channel' ? channelFiles : threadFiles
    if (fileIds) files.current = fileIds
    if (outbound.attempt?.phase === 'sending') return false
    if (typeof maxBytes !== 'number') return false
    const docAtSend =
      isBlankDoc(doc) && outbound.attempt?.phase === 'failed' ? outbound.attempt.doc : doc
    if (!sameRichText(docAtSend, doc)) setDoc(docAtSend)
    if ((isBlankDoc(docAtSend) && files.current.length === 0) || messageBodyBytes(docAtSend) > maxBytes) return false
    return outbound.submit(docAtSend, async (idempotencyKey) => {
      const message = await api.sendMessage(
        conversationId,
        {
          bodySchemaVersion: 1,
          body: docAtSend,
          threadRootId: which === 'thread' ? threadRootId : null,
          replyToId: which === 'thread' ? commentOnId : null,
          fileIds: files.current,
        },
        idempotencyKey,
      )
      setDoc((current) => (sameRichText(current, docAtSend) ? EMPTY_RICH_TEXT : current))
      void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
      if (which === 'thread') {
        threadRef.current?.noteOwnSeq(message.streamSeq)
        threadRef.current?.upsert(message)
        if (threadRootId) timelineRef.current?.noteReply(threadRootId)
        setCommentOnId(null)
      } else {
        timelineRef.current?.noteOwnSeq(message.streamSeq)
        timelineRef.current?.upsert(message)
      }
    })
  }

  if (notFound) return <NotFoundState />

  return (
    <ConversationLiveProvider conversationId={conversationId}>
      <div
        className={threadRootId ? 'conversation' : 'conversation no-thread'}
        style={{ ['--thread-width' as string]: `${String(threadPane.width)}px` }}
      >
        <div className="column">
          <Timeline
            ref={timelineRef}
            workspaceId={workspaceId}
            conversationId={conversationId}
            currentUserId={currentUserId}
            channelRole={channelRole}
            names={names}
            showDmWarning={showDmWarning}
            heading={heading}
            threadBase={threadBase}
            activeThreadId={threadRootId}
            attempt={channelOutbound.attempt}
            onRetry={() => void send('channel')}
            onNotFound={() => setNotFound(true)}
          />
          {serverInfo.isError ? <p className="form-error">{t('error.generic')}</p> : null}
          {readOnly ? readOnlyBar : mustJoin ? joinBar : (
            <Composer
              value={channelDoc}
              onChange={setChannelDoc}
              workspaceId={workspaceId}
              onSend={(fileIds) => send('channel', fileIds)}
              messageBodyMaxBytes={maxBytes}
              busy={channelOutbound.attempt?.phase === 'sending'}
              label={t('composer.placeholder')}
              placeholder={heading ? t('composer.placeholderIn', { name: heading }) : undefined}
              focusToken={threadRootId ? null : conversationId}
              searchMentions={searchMentions}
            />
          )}
        </div>
        {threadRootId ? (
          <div className="column thread-column">
            <ResizeHandle pane={threadPane} label={t('layout.resizeThread')} resetLabel={t('layout.resetWidth')} />
            <ThreadPanel
              ref={threadRef}
              workspaceId={workspaceId}
              conversationId={conversationId}
              rootId={threadRootId}
              currentUserId={currentUserId}
              channelRole={channelRole}
              names={names}
              conversationHref={threadBase}
              attempt={threadOutbound.attempt}
              onRetry={() => void send('thread')}
              onNotFound={() => setNotFound(true)}
              commentOnId={commentOnId}
              onComment={setCommentOnId}
              onClearComment={() => setCommentOnId(null)}
            />
            {readOnly ? readOnlyBar : mustJoin ? joinBar : (
              <Composer
                value={threadDoc}
                onChange={setThreadDoc}
                workspaceId={workspaceId}
                onSend={(fileIds) => send('thread', fileIds)}
                messageBodyMaxBytes={maxBytes}
                busy={threadOutbound.attempt?.phase === 'sending'}
                label={t('thread.compose')}
                placeholder={t('thread.compose')}
                sendLabel={t('thread.send')}
                focusToken={threadRootId}
                searchMentions={searchMentions}
              />
            )}
          </div>
        ) : null}
      </div>
    </ConversationLiveProvider>
  )
}
