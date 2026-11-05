import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Avatar, Button, useConfirm } from '../../design/ui'
import { AuthorCard } from '../people/AuthorCard'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useSession } from '../identity'
import { AttachButton } from '../files/AttachButton'
import { AttachmentList, UploadCards } from '../files/AttachmentList'
import { FileDropZone } from '../files/FileDropZone'
import { useFileUploads } from '../files/useFileUploads'
import { descriptionText, textToDescription } from './ticketModel'

const INITIAL_VISIBLE = 5
const SEEN_PREFIX = 'penfii.ticketSeen.'

function readSeen(ticketId: string): string | null {
  try {
    return window.localStorage.getItem(SEEN_PREFIX + ticketId)
  } catch {
    return null
  }
}

function writeSeen(ticketId: string, stamp: string) {
  try {
    window.localStorage.setItem(SEEN_PREFIX + ticketId, stamp)
  } catch {
    // Without storage the marker simply does not persist.
  }
}

/**
 * Comments are a flat list apart from the system's change records. Anyone who can read may comment.
 * The list shows the newest first-seen comments and folds older ones behind "earlier comments"; a divider
 * marks where comments the viewer has not seen begin (the last-seen stamp lives in this browser only).
 */
export function CommentList({ ticketId, workspaceId, onView }: { ticketId: string; workspaceId: string; onView?: (view: { total: number; shown: number }) => void }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const comments = useQuery(queries.ticketComments(ticketId))
  const session = useSession()
  const meId = session.data?.id
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null)
  const [actionFailed, setActionFailed] = useState(false)
  const [showAll, setShowAll] = useState(false)
  const { confirm, dialog: confirmDialog } = useConfirm()
  // What was seen before this visit; fixed for the visit so the divider does not vanish while reading.
  const seenAt = useRef<string | null | undefined>(undefined)

  const all = useMemo(() => comments.data ?? [], [comments.data])
  if (seenAt.current === undefined && comments.isSuccess) seenAt.current = readSeen(ticketId)
  const since = seenAt.current ?? null
  const isNew = (comment: { createdAt: string; authorId: string }) => since !== null && comment.createdAt > since && comment.authorId !== meId
  const newCount = all.filter(isNew).length
  const visibleCount = showAll ? all.length : Math.min(all.length, Math.max(INITIAL_VISIBLE, newCount))
  const shown = all.slice(all.length - visibleCount)
  const firstNewId = shown.find(isNew)?.id

  useEffect(() => {
    onView?.({ total: all.length, shown: visibleCount })
  }, [all.length, visibleCount, onView])

  useEffect(() => {
    const latest = all.reduce<string | null>((max, comment) => (max === null || comment.createdAt > max ? comment.createdAt : max), null)
    if (latest) writeSeen(ticketId, latest)
  }, [all, ticketId])

  async function refresh() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.ticketComments(ticketId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.ticketActivity(ticketId) }),
    ])
  }

  async function saveEdit() {
    const text = editing?.text.trim()
    if (!editing || !text) return
    setActionFailed(false)
    try {
      await api.editTicketComment(ticketId, editing.id, textToDescription(text))
      setEditing(null)
      await refresh()
    } catch {
      setActionFailed(true)
    }
  }

  async function removeComment(commentId: string) {
    const ok = await confirm({ title: t('work.comments.deleteTitle'), body: t('work.comments.deleteConfirm'), confirmLabel: t('work.comments.delete') })
    if (!ok) return
    setActionFailed(false)
    try {
      await api.deleteTicketComment(ticketId, commentId)
      await refresh()
    } catch {
      setActionFailed(true)
    }
  }

  return (
    <div className="comments">
      {comments.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {comments.isError ? <p className="hint" role="alert">{t('work.comments.loadError')}</p> : null}
      {comments.isSuccess && all.length === 0 ? <p className="hint">{t('work.comments.empty')}</p> : null}
      {comments.isSuccess && all.length > visibleCount ? (
        <button type="button" className="more-button" onClick={() => setShowAll(true)}>
          {t('work.comments.earlier', { count: String(all.length - visibleCount) })}
        </button>
      ) : null}
      {comments.isSuccess ? (
        <ol className="comment-list">
          {shown.map((comment) => (
            <li key={comment.id} className="comment-item-wrap">
              {comment.id === firstNewId ? <div className="new-divider" role="separator">{t('work.comments.newFromHere', { count: String(newCount) })}</div> : null}
              <div className="comment-item">
              <Avatar name={comment.authorName ?? '?'} />
              <div>
                <div className="comment-head">
                  <AuthorCard workspaceId={workspaceId} userId={comment.authorId} name={comment.authorName ?? t('work.actor.unknown')} />
                  <time dateTime={comment.createdAt} title={new Date(comment.createdAt).toLocaleString(locale)}>
                    {formatMessageTime(comment.createdAt, locale)}
                  </time>
                  {comment.editedAt ? <small> · {t('work.comments.edited')}</small> : null}
                </div>
                {editing?.id === comment.id ? (
                  <>
                    <textarea rows={3} value={editing.text} aria-label={t('work.comments.edit')} onChange={(event) => setEditing({ id: comment.id, text: event.target.value })} />
                    <div className="dialog-actions">
                      <Button variant="ghost" onClick={() => setEditing(null)}>{t('work.comments.cancel')}</Button>
                      <Button disabled={!editing.text.trim()} onClick={() => void saveEdit()}>{t('work.comments.save')}</Button>
                    </div>
                  </>
                ) : (
                  <p className="comment-text">{descriptionText(comment.body)}</p>
                )}
                <AttachmentList items={comment.attachments} />
                {editing?.id !== comment.id && (comment.canEdit || comment.canDelete) ? (
                  <p className="comment-actions">
                    {comment.canEdit ? (
                      <button type="button" className="link-button" onClick={() => setEditing({ id: comment.id, text: descriptionText(comment.body) })}>{t('work.comments.edit')}</button>
                    ) : null}
                    {comment.canDelete ? (
                      <button type="button" className="link-button" onClick={() => void removeComment(comment.id)}>{t('work.comments.delete')}</button>
                    ) : null}
                  </p>
                ) : null}
              </div>
              </div>
            </li>
          ))}
        </ol>
      ) : null}
      {actionFailed ? <p className="board-error" role="alert">{t('work.comments.actionError')}</p> : null}
      {confirmDialog}
    </div>
  )
}

/** The comment box lives outside the scrolling area so it is always at the bottom of the panel. */
export function CommentComposer({ ticketId, workspaceId }: { ticketId: string; workspaceId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState('')
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy })
  const areaRef = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    areaRef.current?.focus()
  }, [ticketId])

  async function submit() {
    const text = draft.trim()
    if ((!text && readyIds.length === 0) || busy || blocking) return
    setBusy(true)
    setFailed(false)
    try {
      await api.createTicketComment(ticketId, textToDescription(text), readyIds)
      setDraft('')
      settle()
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.ticketComments(ticketId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.ticketActivity(ticketId) }),
      ])
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) {
      event.preventDefault()
      void submit()
    }
  }

  return (
    <form
      className="comment-form comment-composer"
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <FileDropZone onFiles={addFiles} disabled={busy} className="comment-surface">
        <textarea
          ref={areaRef}
          rows={2}
          aria-label={t('work.comments.placeholder')}
          placeholder={t('work.comments.placeholder')}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
        />
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {uploadFailed ? <p className="board-error" role="alert">{t('composer.failedBlocks')}</p> : null}
        {failed ? <p className="board-error" role="alert">{t('work.comments.error')}</p> : null}
        <div className="dialog-actions">
          <AttachButton onFiles={addFiles} disabled={busy} />
          <Button type="submit" disabled={(!draft.trim() && readyIds.length === 0) || busy || blocking}>{t('work.comments.submit')}</Button>
        </div>
      </FileDropZone>
    </form>
  )
}
