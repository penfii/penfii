import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { RichTextDoc } from '@penfii/contracts'
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
import { EMPTY_RICH_TEXT, isBlankDoc, RichTextEditor, RichTextView, type RichTextEditorHandle } from '../../richtext'
import { CommentEditBox, focusEditButton } from '../comments/CommentEditBox'
import { editableBody, showsRich, withoutBlankEdges } from '../comments/commentBody'
import { descriptionText } from './ticketModel'

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
  const [editing, setEditing] = useState<{ id: string; initial: RichTextDoc } | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
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

  /** Back from the editor: the keyboard returns to the comment's Edit link, where it was. */
  function finishEditing(commentId: string) {
    setEditing(null)
    focusEditButton(listRef.current, commentId)
  }

  async function saveEdit(commentId: string, body: RichTextDoc) {
    setActionFailed(false)
    try {
      await api.editTicketComment(ticketId, commentId, body)
      finishEditing(commentId)
      await refresh()
    } catch {
      // The box stays, with what was typed in it.
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
    <div className="comments" ref={listRef}>
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
                  <CommentEditBox
                    initial={editing.initial}
                    label={t('work.comments.edit')}
                    saveLabel={t('work.comments.save')}
                    cancelLabel={t('work.comments.cancel')}
                    onSave={(body) => saveEdit(comment.id, body)}
                    onCancel={() => finishEditing(comment.id)}
                  />
                ) : (
                  // A comment written with formatting (here or by MCP) is shown with it; a plain one keeps its line breaks as before.
                  showsRich(comment.body) ? <div className="comment-text"><RichTextView doc={comment.body} /></div> : <p className="comment-text">{descriptionText(comment.body)}</p>
                )}
                <AttachmentList items={comment.attachments} />
                {editing?.id !== comment.id && (comment.canEdit || comment.canDelete) ? (
                  <p className="comment-actions">
                    {comment.canEdit ? (
                      <button type="button" className="link-button" data-edit-comment={comment.id} onClick={() => setEditing({ id: comment.id, initial: editableBody(comment.body, descriptionText) })}>{t('work.comments.edit')}</button>
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
  const [draft, setDraft] = useState<RichTextDoc>(EMPTY_RICH_TEXT)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy })
  const editorRef = useRef<RichTextEditorHandle>(null)
  useEffect(() => {
    if (editorRef.current?.focus()) return
    // The editor draws a moment after this effect on the first draw.
    const frame = window.requestAnimationFrame(() => editorRef.current?.focus())
    return () => window.cancelAnimationFrame(frame)
  }, [ticketId])
  const blank = isBlankDoc(draft)

  async function submit() {
    if ((blank && readyIds.length === 0) || busy || blocking) return
    setBusy(true)
    setFailed(false)
    try {
      // A comment of attachments alone has no text: an empty document, as before.
      await api.createTicketComment(ticketId, blank ? { type: 'doc', content: [] } : withoutBlankEdges(draft), readyIds)
      setDraft(EMPTY_RICH_TEXT)
      settle()
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.ticketComments(ticketId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.ticketActivity(ticketId) }),
      ])
    } catch {
      // The text stays in the box for another try.
      setFailed(true)
    } finally {
      setBusy(false)
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
        <RichTextEditor
          ref={editorRef}
          value={draft}
          onChange={setDraft}
          onSend={() => void submit()}
          sendWith="mod-enter"
          onFiles={addFiles}
          label={t('work.comments.placeholder')}
          placeholder={t('work.comments.placeholder')}
        />
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {uploadFailed ? <p className="board-error" role="alert">{t('composer.failedBlocks')}</p> : null}
        {failed ? <p className="board-error" role="alert">{t('work.comments.error')}</p> : null}
        <div className="dialog-actions">
          <AttachButton onFiles={addFiles} disabled={busy} />
          <Button type="submit" disabled={(blank && readyIds.length === 0) || busy || blocking}>{t('work.comments.submit')}</Button>
        </div>
      </FileDropZone>
    </form>
  )
}
