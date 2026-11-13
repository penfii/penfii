import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import { useQueryClient } from '@tanstack/react-query'
import type { DocContent, ReferenceKind } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity/useSession'
import { queryKeys } from '../../queryKeys'
import { UnsavedBlocker } from './UnsavedBlocker'
import { ShareScopePicker, type ShareScope } from './ShareScopePicker'
import { useScopeLabel } from './useScopeLabel'
import { clearNewDocDraft, isBlankDraft, readNewDocDraft, writeNewDocDraft } from './newDocDraft'
import type { EditorController } from './DocEditor'

const DocEditor = lazy(() => import('./DocEditor'))

const EMPTY: DocContent = { schema: 'penfii.doc', version: 1, blocks: [] }
const WORKSPACE: ShareScope = { visibility: 'WORKSPACE', projectId: null, teamId: null }

/**
 * A new page is edited before it exists. The first save asks who it is shared with, then creates it.
 * One draft per person and workspace stays in this browser until save, discard, or logout.
 * Files wait until the page exists. An edit of a page that is already saved is not kept here.
 */
export function DraftDoc({
  workspaceId,
  projectId = null,
  parentId = null,
  onCreated,
  onDiscard,
}: {
  workspaceId: string
  projectId?: string | null
  parentId?: string | null
  onCreated: (id: string) => void
  onDiscard: () => void
}) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const session = useSession()
  const userId = session.data?.id
  const open = useCallback((kind: ReferenceKind, id: string, hints: { projectId?: string }) => {
    const base = `/w/${workspaceId}`
    if (kind === 'TICKET' && hints.projectId) void navigate(`${base}/projects/${hints.projectId}/work?panel=ticket:${id}`)
    else if (kind === 'DOC') void navigate(`${base}/docs?doc=${id}`)
  }, [navigate, workspaceId])
  const [ready, setReady] = useState(false)
  const [title, setTitle] = useState('')
  const [initial, setInitial] = useState<DocContent>(EMPTY)
  const [stored, setStored] = useState(true)
  const [askScope, setAskScope] = useState(false)
  const [scope, setScope] = useState<ShareScope>(WORKSPACE)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const controller = useRef<EditorController | null>(null)
  const body = useRef<DocContent>(EMPTY)
  const shared = useScopeLabel(workspaceId, scope)

  useEffect(() => {
    if (!userId) return
    const saved = readNewDocDraft(userId, workspaceId)
    const content = saved?.content ?? EMPTY
    setTitle(saved?.title ?? '')
    setInitial(content)
    body.current = content
    setReady(true)
  }, [userId, workspaceId])

  useEffect(() => {
    if (!userId || !ready) return
    const handle = window.setTimeout(() => {
      setStored(writeNewDocDraft(userId, workspaceId, { title, content: body.current }))
    }, 200)
    return () => window.clearTimeout(handle)
  }, [title, initial, userId, workspaceId, ready])

  const onContentChange = useCallback((content: DocContent) => {
    body.current = content
    if (!userId) return
    setStored(writeNewDocDraft(userId, workspaceId, { title, content }))
  }, [title, userId, workspaceId])

  const dirty = title.trim().length > 0 || !isBlankDraft({ title: '', content: body.current })
  useEffect(() => {
    if (!dirty || stored) return
    const guard = (event: BeforeUnloadEvent) => {
      event.preventDefault()
      event.returnValue = ''
    }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [dirty, stored])

  const onController = useCallback((next: EditorController | null) => {
    controller.current = next
  }, [])

  function discard() {
    if (userId) clearNewDocDraft(userId, workspaceId)
    onDiscard()
  }

  async function create() {
    const trimmed = title.trim()
    if (!trimmed || busy) return
    setBusy(true)
    setFailed(false)
    try {
      const page = await api.createDoc(workspaceId, {
        title: trimmed,
        parentId,
        projectId,
        visibility: scope.visibility,
        scopeProjectId: scope.projectId,
        scopeTeamId: scope.teamId,
      })
      const content = controller.current?.getContent() ?? body.current
      if (content && !isBlankDraft({ title: '', content })) {
        await api.saveDoc(page.id, { expectedRevisionNo: page.revisionNo, title: trimmed, content })
      }
      if (userId) clearNewDocDraft(userId, workspaceId)
      // The new page is in the list before it opens, so it is there to see and to select.
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) })
      setAskScope(false)
      onCreated(page.id)
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  if (!ready) return <p className="hint" role="status">{t('loading')}</p>

  return (
    <div className="doc-page">
      <UnsavedBlocker active={dirty && !stored && !busy} />
      <header className="doc-draft-bar">
        <div>
          <strong>{t('docs.draft.badge')}</strong>
          <p className="hint">{stored ? t('docs.draft.hint') : t('docs.draft.hintMemory')}</p>
        </div>
        <div className="doc-actions">
          <Button type="button" variant="ghost" onClick={discard}>{t('docs.draft.discard')}</Button>
          <Button type="button" disabled={!title.trim() || busy} onClick={() => setAskScope(true)}>{t('docs.draft.save')}</Button>
        </div>
      </header>
      <label className="field">
        <span>{t('docs.draft.title')}</span>
        <input value={title} aria-label={t('docs.draft.title')} placeholder={t('docs.draft.title')} onChange={(event) => setTitle(event.target.value)} />
      </label>
      <Suspense fallback={<p className="hint">{t('loading')}</p>}>
        <DocEditor
          key={`${userId ?? 'anon'}:${workspaceId}`}
          docId="draft"
          projectId={projectId}
          workspaceId={workspaceId}
          content={initial}
          editable
          files={false}
          onController={onController}
          onContentChange={onContentChange}
          open={open}
        />
      </Suspense>
      <Dialog open={askScope} title={t('docs.draft.shareTitle')} onClose={() => setAskScope(false)}>
        <ShareScopePicker workspaceId={workspaceId} name="draft-scope" value={scope} onChange={setScope} suggestedProjectId={projectId} />
        <p className="hint">{t('docs.draft.willShare', { scope: shared })}</p>
        {failed ? <p className="board-error" role="alert">{t('docs.new.error')}</p> : null}
        <div className="dialog-actions">
          <Button type="button" variant="ghost" onClick={() => setAskScope(false)}>{t('docs.draft.keep')}</Button>
          <Button type="button" disabled={busy} onClick={() => void create()}>{t('docs.draft.create')}</Button>
        </div>
      </Dialog>
    </div>
  )
}
