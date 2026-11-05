import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { DocComment } from '@penfii/contracts'
import { api } from '../../api'
import { Avatar, Button } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { RichTextView, plainText } from '../../richtext'
import { commentToRichText, type PickedMention } from './commentModel'
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
  const [draft, setDraft] = useState('')
  const [picked, setPicked] = useState<PickedMention[]>([])
  const [failed, setFailed] = useState(false)
  const [posting, setPosting] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy: posting })
  const post = useMutation({
    mutationFn: (text: string) => api.createDocComment(docId, commentToRichText(text, picked), readyIds),
    onSuccess: async () => {
      setDraft('')
      setPicked([])
      settle()
      await refresh()
    },
    onError: () => setFailed(true),
  })
  const busy = post.isPending
  useEffect(() => setPosting(busy), [busy])
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null)
  const [suggest, setSuggest] = useState<{ query: string; items: Array<{ id: string; name: string }> } | null>(null)
  const areaRef = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    if (!autoFocus) return
    areaRef.current?.focus()
  }, [autoFocus, docId])
  const caretAfter = useRef<number | null>(null)
  // The caret goes after a picked name in the same frame as the text that holds it, so quick typing
  // right after the pick lands in the right place.
  useLayoutEffect(() => {
    const area = areaRef.current
    if (area && caretAfter.current !== null) {
      area.focus()
      area.setSelectionRange(caretAfter.current, caretAfter.current)
      caretAfter.current = null
    }
  }, [draft])

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
    const text = draft.trim()
    if ((!text && readyIds.length === 0) || busy || blocking) return
    setFailed(false)
    post.mutate(text)
  }

  async function saveEdit() {
    if (!editing || !editing.text.trim()) return
    try {
      await api.editDocComment(docId, editing.id, commentToRichText(editing.text.trim(), []))
      setEditing(null)
      await refresh()
    } catch {
      setFailed(true)
    }
  }

  async function lookup(text: string, caret: number) {
    const before = text.slice(0, caret)
    const match = /(?:^|\s)@([^\s@]{0,20})$/.exec(before)
    if (!match) {
      setSuggest(null)
      return
    }
    const query = match[1] ?? ''
    try {
      const found = await api.searchDirectory(workspaceId, { q: query, types: 'person', limit: 6 })
      const items = found.results.flatMap((hit) => (hit.type === 'person' && hit.person ? [{ id: hit.person.id, name: hit.person.displayName }] : []))
      setSuggest({ query, items })
    } catch {
      setSuggest(null)
    }
  }

  function pick(item: { id: string; name: string }) {
    const area = areaRef.current
    if (!area || !suggest) return
    const caret = area.selectionStart
    const before = draft.slice(0, caret)
    const start = before.lastIndexOf('@')
    const next = `${draft.slice(0, start)}@${item.name} ${draft.slice(caret)}`
    setDraft(next)
    setPicked((current) => (current.some((p) => p.userId === item.id) ? current : [...current, { userId: item.id, name: item.name }]))
    setSuggest(null)
    caretAfter.current = start + item.name.length + 2
  }

  return (
    <section className="doc-comments" aria-label={t('docs.comments.title')}>
      <h2>{t('docs.comments.title')}{comments.isSuccess ? <span className="count-pill">{comments.data.items.length}</span> : null}</h2>
      {comments.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {comments.isError ? <p className="board-error" role="alert">{t('docs.comments.loadError')}</p> : null}
      {comments.isSuccess && comments.data.items.length === 0 ? <p className="hint">{t('docs.comments.empty')}</p> : null}
      {comments.isSuccess ? (
        <ol className="comment-list">
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
                  <>
                    <textarea rows={3} value={editing.text} aria-label={t('docs.comments.edit')} onChange={(event) => setEditing({ id: comment.id, text: event.target.value })} />
                    <div className="dialog-actions">
                      <Button variant="ghost" onClick={() => setEditing(null)}>{t('docs.cancel')}</Button>
                      <Button onClick={() => void saveEdit()}>{t('docs.comments.save')}</Button>
                    </div>
                  </>
                ) : (
                  <div className="comment-text"><RichTextView doc={comment.body} names={names.data} /></div>
                )}
                <AttachmentList items={comment.attachments ?? []} />
                {editing?.id !== comment.id && (comment.authorId === userId || canManage) ? (
                  <p className="comment-actions">
                    {comment.authorId === userId ? (
                      <button type="button" className="link-button" onClick={() => setEditing({ id: comment.id, text: plainText(comment.body) })}>{t('docs.comments.edit')}</button>
                    ) : null}
                    <button
                      type="button"
                      className="link-button"
                      onClick={() => void api.deleteDocComment(docId, comment.id).then(refresh).catch(() => setFailed(true))}
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
        <div className="doc-mention-box">
          <textarea
            ref={areaRef}
            rows={3}
            aria-label={t('docs.comments.placeholder')}
            placeholder={t('docs.comments.placeholder')}
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value)
              void lookup(event.target.value, event.target.selectionStart)
            }}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing) return
              if (event.key === 'Escape') setSuggest(null)
              if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
                event.preventDefault()
                submit()
              }
            }}
          />
          {suggest && suggest.items.length > 0 ? (
            <ul className="doc-mention-list" role="listbox" aria-label={t('docs.comments.mention')}>
              {suggest.items.map((item) => (
                <li key={item.id} role="option" aria-selected={false}>
                  <button type="button" onMouseDown={(event) => event.preventDefault()} onClick={() => pick(item)}>{item.name}</button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {uploadFailed ? <p className="board-error" role="alert">{t('composer.failedBlocks')}</p> : null}
        {failed ? <p className="board-error" role="alert">{t('docs.comments.error')}</p> : null}
        <div className="dialog-actions">
          <AttachButton onFiles={addFiles} disabled={busy} />
          <Button type="submit" disabled={(!draft.trim() && readyIds.length === 0) || busy || blocking}>{t('docs.comments.submit')}</Button>
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
