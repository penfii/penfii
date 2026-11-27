import { useCallback, useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { DocComment, RichTextDoc } from '@penfii/contracts'
import { api } from '../../api'
import { Avatar, Button, useConfirm } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { EMPTY_RICH_TEXT, RichTextEditor, RichTextView, isBlankDoc, plainText, type MentionCandidate, type RichTextEditorHandle } from '../../richtext'
import { CommentEditBox, focusEditButton } from '../comments/CommentEditBox'
import { editableBody, withoutBlankEdges } from '../comments/commentBody'
import { AttachButton } from '../files/AttachButton'
import { AttachmentList, UploadCards } from '../files/AttachmentList'
import { FileDropZone } from '../files/FileDropZone'
import { useFileUploads } from '../files/useFileUploads'

/**
 * Comments of a page (`doc_comments`). They are a flat list with no anchor in the text. Anyone who can
 * read the page may comment; typing `@` offers members, and a picked name mentions that person.
 */
export function DocComments({
  workspaceId,
  docId,
  userId,
  canManage,
  refreshTick,
  autoFocus = false,
}: {
  workspaceId: string
  docId: string
  userId: string
  canManage: boolean
  /** Changes when the co-editing socket says comments changed. */
  refreshTick: number
  /** The comment box is the writing surface while the page is not being edited. */
  autoFocus?: boolean
}) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const comments = useQuery(queries.docComments(docId))
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [removing, setRemoving] = useState<string | null>(null)
  const [draft, setDraft] = useState<RichTextDoc>(EMPTY_RICH_TEXT)
  const [failed, setFailed] = useState(false)
  const [posting, setPosting] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy: posting })
  const blank = isBlankDoc(draft)
  const post = useMutation({
    // A comment of attachments alone has no text: one empty paragraph, as before.
    mutationFn: () => api.createDocComment(docId, blank ? { type: 'doc', content: [{ type: 'paragraph', content: [] }] } : withoutBlankEdges(draft), readyIds),
    onSuccess: async () => {
      setDraft(EMPTY_RICH_TEXT)
      settle()
      await refresh()
    },
    // The text stays in the box for another try.
    onError: () => setFailed(true),
  })
  const busy = post.isPending
  useEffect(() => setPosting(busy), [busy])
  const [editing, setEditing] = useState<{ id: string; initial: RichTextDoc } | null>(null)
  const editorRef = useRef<RichTextEditorHandle>(null)
  const listRef = useRef<HTMLOListElement>(null)
  useEffect(() => {
    if (!autoFocus) return
    if (editorRef.current?.focus()) return
    // The editor draws a moment after this effect on the first draw.
    const frame = window.requestAnimationFrame(() => editorRef.current?.focus())
    return () => window.cancelAnimationFrame(frame)
  }, [autoFocus, docId])
  // `@` lists the people of the Workspace by name; what is picked is stored as the person's id, and the server decides who is told.
  const searchMentions = useCallback(
    async (query: string): Promise<MentionCandidate[]> => {
      const found = await api.searchDirectory(workspaceId, { q: query, types: 'person', limit: 8 })
      return found.results.flatMap((hit) => (hit.type === 'person' && hit.person ? [{ id: hit.person.id, name: hit.person.displayName, member: true }] : []))
    },
    [workspaceId],
  )

  const mentionIds = Array.from(new Set((comments.data?.items ?? []).flatMap((c) => mentionedIds(c.body)))).sort()
  const names = useQuery({
    queryKey: queryKeys.docCommentNames(workspaceId, mentionIds),
    queryFn: async () => {
      const resolved = await api.resolveReferences(workspaceId, mentionIds.map((id) => ({ kind: 'USER' as const, id })))
      return Object.fromEntries(resolved.filter((r) => r.available && r.title).map((r) => [r.id, r.title!]))
    },
    enabled: mentionIds.length > 0,
  })
  useEffect(() => {
    if (refreshTick > 0) void queryClient.invalidateQueries({ queryKey: queryKeys.docComments(docId) })
  }, [refreshTick, docId, queryClient])

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.docComments(docId) })
  }

  function submit() {
    if ((blank && readyIds.length === 0) || busy || blocking) return
    setFailed(false)
    post.mutate()
  }

  /** Back from the editor: the keyboard returns to the comment's Edit link, where it was. */
  function finishEditing(commentId: string) {
    setEditing(null)
    focusEditButton(listRef.current, commentId)
  }

  async function saveEdit(commentId: string, body: RichTextDoc) {
    setFailed(false)
    try {
      await api.editDocComment(docId, commentId, body)
      finishEditing(commentId)
      await refresh()
    } catch {
      // The box stays, with what was typed in it.
      setFailed(true)
    }
  }

  async function removeComment(commentId: string) {
    if (removing) return
    const ok = await confirm({ title: t('docs.comments.deleteTitle'), body: t('docs.comments.deleteConfirm'), confirmLabel: t('docs.delete') })
    if (!ok) return
    setFailed(false)
    setRemoving(commentId)
    try {
      await api.deleteDocComment(docId, commentId)
      await refresh()
    } catch {
      setFailed(true)
    } finally {
      setRemoving(null)
    }
  }

  return (
    <section className="doc-comments" aria-label={t('docs.comments.title')}>
      {confirmDialog}
      <h2>{t('docs.comments.title')}{comments.isSuccess ? <span className="count-pill">{comments.data.items.length}</span> : null}</h2>
      {comments.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {comments.isError ? <p className="board-error" role="alert">{t('docs.comments.loadError')}</p> : null}
      {comments.isSuccess && comments.data.items.length === 0 ? <p className="hint">{t('docs.comments.empty')}</p> : null}
      {comments.isSuccess ? (
        <ol className="comment-list" ref={listRef}>
          {comments.data.items.map((comment: DocComment) => (
            <li key={comment.id} className="comment-item">
              <Avatar name={comment.authorName} />
              <div>
                <p className="comment-head">
                  <strong>{comment.authorName}</strong>
                  <time dateTime={comment.createdAt}>{formatMessageTime(comment.createdAt, locale)}</time>
                  {comment.editedAt ? <small> · {t('docs.comments.edited')}</small> : null}
                </p>
                {editing?.id === comment.id ? (
                  <CommentEditBox
                    initial={editing.initial}
                    label={t('docs.comments.edit')}
                    saveLabel={t('docs.comments.save')}
                    cancelLabel={t('docs.cancel')}
                    onSave={(body) => saveEdit(comment.id, body)}
                    onCancel={() => finishEditing(comment.id)}
                    mentions={{ search: searchMentions }}
                  />
                ) : (
                  <div className="comment-text"><RichTextView doc={comment.body} names={names.data} /></div>
                )}
                <AttachmentList items={comment.attachments ?? []} />
                {editing?.id !== comment.id && (comment.authorId === userId || canManage) ? (
                  <p className="comment-actions">
                    {comment.authorId === userId ? (
                      <button type="button" className="link-button" data-edit-comment={comment.id} onClick={() => setEditing({ id: comment.id, initial: editableBody(comment.body, (body) => plainText(body as RichTextDoc)) })}>{t('docs.comments.edit')}</button>
                    ) : null}
                    <button
                      type="button"
                      className="link-button"
                      disabled={removing !== null}
                      onClick={() => void removeComment(comment.id)}
                    >
                      {t('docs.delete')}
                    </button>
                  </p>
                ) : null}
              </div>
            </li>
          ))}
        </ol>
      ) : null}
      <form
        className="comment-form"
        onSubmit={(event) => {
          event.preventDefault()
          submit()
        }}
      >
        <FileDropZone onFiles={addFiles} disabled={busy} className="comment-surface">
        <RichTextEditor
          ref={editorRef}
          value={draft}
          onChange={setDraft}
          onSend={submit}
          sendWith="mod-enter"
          onFiles={addFiles}
          mentions={{ search: searchMentions }}
          label={t('docs.comments.placeholder')}
          placeholder={t('docs.comments.placeholder')}
        />
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {uploadFailed ? <p className="board-error" role="alert">{t('composer.failedBlocks')}</p> : null}
        {failed ? <p className="board-error" role="alert">{t('docs.comments.error')}</p> : null}
        <div className="dialog-actions">
          <AttachButton onFiles={addFiles} disabled={busy} />
          <Button type="submit" disabled={(blank && readyIds.length === 0) || busy || blocking}>{t('docs.comments.submit')}</Button>
        </div>
        </FileDropZone>
      </form>
    </section>
  )
}

function mentionedIds(body: unknown): string[] {
  const out: string[] = []
  const walk = (node: unknown) => {
    if (typeof node !== 'object' || node === null) return
    const record = node as { type?: string; attrs?: { userId?: string }; content?: unknown[] }
    if (record.type === 'mention' && record.attrs?.userId) out.push(record.attrs.userId)
    for (const child of record.content ?? []) walk(child)
  }
  walk(body)
  return out
}
