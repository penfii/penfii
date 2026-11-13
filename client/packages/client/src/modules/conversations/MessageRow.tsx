import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import type { Message, MessageVersion, RichTextDoc } from '@penfii/contracts'
import { Link, useParams, useSearchParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, isNotFound, isVersionConflict, newIdempotencyKey } from '@penfii/api-client'
import { api } from '../../api'
import { canDeleteMessage } from '../spaces/capabilities'
import { queries } from '../../queries'
import { formatStamp } from '../../format'
import { displayedMinute } from './group'
import { useI18n } from '../../i18n/I18nProvider'
import { isBlankDoc, mentionedUserIds, plainText, RichTextEditor, RichTextView, type DisplayNames } from '../../richtext'
import { useMentionNameOf } from '../../richtext/env'
import { errorText } from '../../ui/errorText'
import { Avatar, Button, useConfirm } from '../../design/ui'
import { AuthorCard } from '../people/AuthorCard'
import { BookmarkIcon, MoreIcon, ThreadIcon } from '../../design/icons'
import { MessageBody } from './MessageBody'
import { prefillFromMessage, rememberPrefill } from '../work/fromChat'
import { CREATE_PANEL, CREATE_KEYS } from '../work/panelParam'
import { MessageReference } from './ReplyQuote'
import { presentMessage } from './messages'
import { REACTION_EMOJI, reactionCount, reactionPressed, withToggledReaction } from './reactions'
import { useShown } from './useShown'
import { Sheet } from '../../design/Sheet'
import { useIsMobile } from '../../design/useIsMobile'
import { useLongPress } from '../../design/useLongPress'
import { queryKeys } from '../../queryKeys'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function MessageRow({
  message,
  currentUserId,
  channelRole,
  names,
  scrollRoot,
  onVisible,
  onMessage,
  onNotFound,
  onManualUnread,
  threadHref,
  threadActive = false,
  onReply,
  replyActive,
  referenced,
  commentCount = 0,
  onShowComments,
  onOpenReference,
  onComment,
  commentActive,
  compact = false,
}: {
  message: Message
  currentUserId: string
  channelRole?: string | null
  names: DisplayNames
  scrollRoot: HTMLElement | null
  onVisible: (seq: number, shown: boolean) => void
  onMessage: (message: Message) => void
  onNotFound: () => void
  onManualUnread: (fromSeq: number) => void
  threadHref?: string
  threadActive?: boolean
  onReply?: () => void
  replyActive?: boolean
  referenced?: Message | null
  commentCount?: number
  onShowComments?: () => void
  onOpenReference?: (messageId: string) => void
  onComment?: () => void
  commentActive?: boolean
  compact?: boolean
}) {
  const { locale, t } = useI18n()
  const nameOf = useMentionNameOf(names)
  const ref = useShown(scrollRoot, message.streamSeq, onVisible)
  const [editDoc, setEditDoc] = useState<RichTextDoc | null>(null)
  const editBox = useRef<HTMLDivElement>(null)
  const editing = editDoc !== null
  // On a phone the composer sits right under the list: bring the whole edit form, with its buttons, into view.
  useEffect(() => {
    if (editing) editBox.current?.scrollIntoView?.({ block: 'nearest' })
  }, [editing])
  const [expectedVersion, setExpectedVersion] = useState(message.version)
  const [conflictBody, setConflictBody] = useState<RichTextDoc | null | undefined>(undefined)
  const [history, setHistory] = useState<MessageVersion[] | null>(null)
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [more, setMore] = useState(false)
  // On a phone a long press opens the message's actions as a bottom sheet (the hover toolbar needs a mouse).
  const [sheet, setSheet] = useState(false)
  const mobile = useIsMobile()
  const press = useLongPress(() => setSheet(true), mobile)
  const closeMenu = () => { setMore(false); setSheet(false) }
  const menuRef = useRef<HTMLDivElement>(null)
  const toggleRef = useRef<HTMLButtonElement>(null)
  // The menu hangs below the message; at the end of the list it is brought into view instead of left under the composer.
  useEffect(() => {
    if (!more) return
    menuRef.current?.scrollIntoView({ block: 'nearest' })
    menuRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus()
  }, [more])
  /** Arrow keys move between the items; Escape closes the menu and returns to its button. */
  function onMenuKey(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      setMore(false)
      toggleRef.current?.focus()
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="menuitem"]')]
    const at = items.indexOf(document.activeElement as HTMLElement)
    items[(at + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus()
    event.preventDefault()
  }
  const { workspaceId = '', conversationId = '' } = useParams()
  const [searchParams, setSearchParams] = useSearchParams()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 && conversationId.length > 0 })
  /** Opens the Ticket form beside the chat, starting from this message's own text. */
  function makeTicket() {
    const channel = space.data?.channels.find((item) => item.conversationId === conversationId)
    const known: Record<string, string> = { ...names }
    for (const id of message.body ? mentionedUserIds(message.body) : []) {
      const name = nameOf(id)
      if (name) known[id] = name
    }
    rememberPrefill(message.id, prefillFromMessage(message, known))
    const next = new URLSearchParams(searchParams)
    for (const key of CREATE_KEYS) next.delete(key)
    next.set('panel', CREATE_PANEL)
    if (channel?.projectId) {
      next.set('newProject', channel.projectId)
      next.set('newChannel', channel.id)
    }
    next.set('newAssignee', currentUserId)
    next.set('newFromConv', conversationId)
    next.set('newFromMsg', message.id)
    next.set('newFromRoot', message.threadRootId ?? message.id)
    setSearchParams(next)
  }
  const queryClient = useQueryClient()
  const [laterNote, setLaterNote] = useState('')
  async function saveForLater(messageId: string) {
    try {
      await api.addLater(workspaceId, messageId, null, 'MESSAGE')
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.later })
      setLaterNote(t('saved.later.addedMessage'))
      window.setTimeout(() => setLaterNote(''), 2500)
    } catch {
      setLaterNote(t('error.generic'))
    }
  }
  const [busyEmoji, setBusyEmoji] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const mine = message.authorId === currentUserId

  async function saveEdit() {
    setError(null)
    try {
      if (!editDoc || isBlankDoc(editDoc)) return
      const updated = await api.editMessage(
        message.id,
        { expectedVersion, body: editDoc },
        newIdempotencyKey(),
      )
      setEditDoc(null)
      setConflictBody(undefined)
      onMessage(presentMessage(updated))
    } catch (caught) {
      if (isVersionConflict(caught)) {
        const details = isApiError(caught) && isRecord(caught.details) ? caught.details : null
        const version = details && typeof details.version === 'number' ? details.version : null
        if (details && version !== null && 'body' in details) {
          setConflictBody(details.body as RichTextDoc | null)
          setExpectedVersion(version)
          return
        }
        setError(t('edit.conflict'))
        return
      }
      if (isNotFound(caught)) onNotFound()
      else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  async function remove() {
    if (!(await confirm({ title: t('message.delete'), body: t('message.deleteConfirm'), confirmLabel: t('message.delete') }))) return
    setError(null)
    try {
      await api.deleteMessage(message.id, newIdempotencyKey())
      onMessage({ ...message, deleted: true, body: null })
    } catch (caught) {
      if (isNotFound(caught)) onNotFound()
      else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  async function toggleReaction(emoji: string) {
    if (busyEmoji) return
    setBusyEmoji(emoji)
    const previous = message
    onMessage(withToggledReaction(message, emoji, currentUserId))
    const mineReaction = previous.reactions.some(
      (reaction) => reaction.emoji === emoji && reactionPressed(reaction, currentUserId),
    )
    try {
      const key = newIdempotencyKey()
      if (mineReaction) await api.deleteReaction(message.id, { emoji }, key)
      else await api.putReaction(message.id, { emoji }, key)
    } catch (caught) {
      onMessage(previous)
      if (isNotFound(caught)) onNotFound()
      else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    } finally {
      setBusyEmoji(null)
    }
  }

  async function copyText() {
    const text = message.body ? plainText(message.body, nameOf) : ''
    try {
      await navigator.clipboard.writeText(text)
      closeMenu()
    } catch {
      setError(t('message.copyFailed'))
    }
  }

  async function showHistory() {
    setError(null)
    try {
      const versions = await api.messageHistory(message.id)
      setHistory(versions.versions)
    } catch (caught) {
      if (isNotFound(caught)) onNotFound()
      else setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    }
  }

  // Earlier versions are readable by their author only (the server answers anyone else "not found",
  // which would blank the whole timeline), so only the author gets a link; others see that it was edited.
  const editedLink = message.editedAt && !message.deleted ? (
    mine ? (
      <button type="button" className="edited-link" onClick={() => void showHistory()}>{t('message.editedHistory')}</button>
    ) : (
      <span className="edited-link">{t('message.edited')}</span>
    )
  ) : null

  // In the phone's sheet a reaction is a one-step choice: it is sent and the sheet closes.
  const reactionBar = (after?: () => void) => (
  <div className="reaction-bar" aria-label={t('reactions.toggle')}>
    {REACTION_EMOJI.map((emoji) => {
      const reaction = (message.reactions ?? []).find((item) => item.emoji === emoji)
      const pressed = reactionPressed(reaction, currentUserId)
      return (
        <button
          key={emoji}
          type="button"
          aria-pressed={pressed}
          aria-label={`${t('reactions.toggle')} ${emoji}`}
          disabled={busyEmoji === emoji}
          onClick={() => { void toggleReaction(emoji); after?.() }}
        >
          {emoji}
        </button>
      )
    })}
  </div>
  )
  const menuItems = (
    <>
    {threadHref ? <Link role="menuitem" to={threadHref} onClick={() => closeMenu()}>{t('message.openThread')}</Link> : null}
    {onComment ? (
      <button type="button" role="menuitem" aria-pressed={commentActive ?? false} onClick={() => { closeMenu(); onComment() }}>
        {t('message.threadReply')}
      </button>
    ) : null}
    {onReply ? (
      <button type="button" role="menuitem" aria-pressed={replyActive ?? false} onClick={() => { closeMenu(); onReply() }}>
        {t('message.threadReply')}
      </button>
    ) : null}
    <button type="button" role="menuitem" onClick={() => void copyText()}>
      {t('message.copy')}
    </button>
    {conversationId && !message.deleted ? (
      <button type="button" role="menuitem" onClick={() => { closeMenu(); makeTicket() }}>
        {t('message.makeTicket')}
      </button>
    ) : null}
    <button type="button" role="menuitem" onClick={() => { closeMenu(); void saveForLater(message.id) }}>
      {t('saved.later.addMessage')}
    </button>
    <button type="button" role="menuitem" onClick={() => { closeMenu(); onManualUnread(message.streamSeq) }}>
      {t('message.markUnread')}
    </button>
    {mine ? (
      <>
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            if (!message.body) return
            setEditDoc(message.body)
            closeMenu()
            setExpectedVersion(message.version)
            setConflictBody(undefined)
          }}
        >
          {t('message.edit')}
        </button>
        <button type="button" role="menuitem" onClick={() => { closeMenu(); void showHistory() }}>
          {t('message.history')}
        </button>
      </>
    ) : null}
    {canDeleteMessage(message, currentUserId, channelRole) ? (
      <>
        <hr className="menu-separator" />
        <button type="button" role="menuitem" className="menu-danger" onClick={() => { closeMenu(); void remove() }}>
          {t('message.delete')}
        </button>
      </>
    ) : null}
    </>
  )

  return (
    <article
      ref={ref}
      className={[compact ? 'message compact' : 'message group-start', threadActive ? 'thread-active' : ''].filter(Boolean).join(' ')}
      data-stream-seq={message.streamSeq}
      data-message-id={message.id}
      onMouseLeave={() => setMore(false)}
      {...press}
    >
      <div className="message-gutter">
        {compact ? null : <Avatar name={names[message.authorId] || message.authorDisplayName || message.authorId} />}
      </div>
      <div className="message-main">
      {compact ? null : <header className="message-meta">
        <AuthorCard workspaceId={workspaceId} userId={message.authorId} name={names[message.authorId] || message.authorDisplayName || message.authorId} authorKind={message.authorKind} />
        <span aria-hidden="true">·</span>
        <time dateTime={message.createdAt} title={formatStamp(message.createdAt, locale)}>
          {displayedMinute(message.createdAt) ?? message.createdAt}
        </time>
        {editedLink}
      </header>}
      {message.replyToId ? (
        <MessageReference
          name={referenced ? names[referenced.authorId] || referenced.authorDisplayName || referenced.authorId : ''}
          message={referenced ?? null}
          onOpen={referenced && onOpenReference ? () => onOpenReference(referenced.id) : undefined}
        />
      ) : null}
      {editDoc ? null : <MessageBody message={message} names={names} />}
      {/* A grouped (compact) row has no header, so the marker follows the text; otherwise an edit would show nowhere. */}
      {compact && !editDoc ? editedLink : null}
      {conflictBody !== undefined ? (
        <div className="conflict">
          <p>{t('edit.conflict')}</p>
          <div>
            <h4>{t('edit.server')}</h4>
            <MessageBody message={{ deleted: conflictBody == null, body: conflictBody }} names={names} />
          </div>
          <div>
            <h4>{t('edit.local')}</h4>
            {editDoc ? <RichTextView doc={editDoc} names={names} /> : null}
          </div>
          <div className="conflict-actions">
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                // Take the server copy: the edit is dropped and the row shows what the server has.
                onMessage({ ...message, body: conflictBody, deleted: conflictBody == null, version: expectedVersion })
                setEditDoc(null)
                setConflictBody(undefined)
              }}
            >
              {t('edit.useServer')}
            </Button>
            <Button type="button" onClick={() => void saveEdit()}>{t('edit.keepMine')}</Button>
          </div>
        </div>
      ) : null}
      {editDoc ? (
        <div className="edit-box" ref={editBox}>
          <RichTextEditor value={editDoc} baseline={message.body} onChange={setEditDoc} label={t('edit.local')} autoFocus />
          <Button type="button" onClick={() => void saveEdit()}>
            {t('message.save')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            onClick={() => {
              setEditDoc(null)
              setConflictBody(undefined)
            }}
          >
            {t('message.cancel')}
          </Button>
        </div>
      ) : null}
      {message.replyCount > 0 && threadHref ? (
        <Link className={threadActive ? 'reply-count active' : 'reply-count'} to={threadHref} aria-current={threadActive ? 'page' : undefined}>
          <ThreadIcon />
          {t('message.replies', { count: String(message.replyCount) })}
        </Link>
      ) : null}
      {commentCount > 0 && onShowComments ? (
        <button type="button" className="reply-count" onClick={onShowComments}>
          <ThreadIcon />
          {t('message.replies', { count: String(commentCount) })}
        </button>
      ) : null}
      {!message.deleted && (message.reactions ?? []).some((reaction) => reactionCount(reaction) > 0) ? (
        <div className="reaction-pills">
          {(message.reactions ?? []).filter((reaction) => reactionCount(reaction) > 0).map((reaction) => (
            <button
              key={reaction.emoji}
              type="button"
              aria-pressed={reactionPressed(reaction, currentUserId)}
              onClick={() => void toggleReaction(reaction.emoji)}
            >
              {reaction.emoji} {reactionCount(reaction)}
            </button>
          ))}
        </div>
      ) : null}
      {!message.deleted ? (
        <div className="hover-actions">
          {reactionBar()}
          {threadHref ? (
            <Link
              className="thread-open"
              to={threadHref}
              aria-label={t('message.openThread')}
              title={t('message.openThread')}
              aria-current={threadActive ? 'page' : undefined}
            >
              <ThreadIcon />
            </Link>
          ) : null}
          <button type="button" className="hover-later" title={t('saved.later.addMessage')} onClick={() => void saveForLater(message.id)}>
            <BookmarkIcon />
            <span className="visually-hidden">{t('saved.later.addMessage')}</span>
          </button>
          <button type="button" className="more-toggle" ref={toggleRef} aria-haspopup="menu" aria-expanded={more} aria-label={t('message.more')} title={t('message.more')} onClick={() => setMore((current) => !current)}>
            <MoreIcon />
          </button>
          {more ? (
            <div className="hover-menu" role="menu" aria-label={t('message.more')} ref={menuRef} onKeyDown={onMenuKey}>
            {menuItems}
            </div>
          ) : null}
        </div>
      ) : null}
      {mobile && !message.deleted ? (
        // There is no hover on a phone: this is the visible way to the actions (a long press does the same).
        <button type="button" className="message-more" aria-haspopup="dialog" aria-label={t('message.more')} onClick={() => setSheet(true)}>
          <MoreIcon />
        </button>
      ) : null}
      {sheet ? (
        <Sheet open title={t('message.more')} onClose={closeMenu}>
          <div className="m-sheet-reactions">{reactionBar(closeMenu)}</div>
          <div className="m-sheet-menu" role="menu" aria-label={t('message.more')}>{menuItems}</div>
        </Sheet>
      ) : null}
      {laterNote ? <p className="hint" role="status">{laterNote}</p> : null}
      {history && !message.deleted ? (
        <div className="history">
          <h4>{t('history.title')}</h4>
          {history.map((version) => (
            <div key={version.version}>
              <span>v{version.version}</span>
              <MessageBody message={{ deleted: version.body == null, body: version.body }} names={names} />
            </div>
          ))}
        </div>
      ) : null}
      {error ? <p className="form-error">{error}</p> : null}
      </div>
      {confirmDialog}
    </article>
  )
}
