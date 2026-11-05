import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, isNotFound } from '@penfii/api-client'
import type { DocContent, DocPage, ReferenceKind } from '@penfii/contracts'
import { formatMessageTime } from '../../format'
import { api } from '../../api'
import { Button, Dialog, EmptyState } from '../../design/ui'
import { MoreIcon } from '../../design/icons'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useSession } from '../identity/useSession'
import { DocAccessDialog } from './DocAccessDialog'
import { DocReferencedBy } from './ReferencedBy'
import { DocComments } from './DocComments'
import { DocHistoryDialog } from './DocHistoryDialog'
import { UnsavedBlocker } from './UnsavedBlocker'
import { Popover } from '../work/Popover'
import { useScopeLabel } from './useScopeLabel'
import type { EditorController } from './DocEditor'

// BlockNote is large: it loads only when a page is opened.
const DocEditor = lazy(() => import('./DocEditor'))

/** The draft of one editing session. It lives in the browser until Save. */
type Editing = {
  /** Remounts the editor when the draft starts over from other content. */
  seed: number
  title: string
  content: DocContent
  /** The page's `revisionNo` when the draft started; Save sends it back. */
  baseRevision: number
  /** The Version the draft was restored from, if any. */
  restoredFrom: number | null
}

type Problem = 'conflict' | 'invalid' | 'error' | null

/**
 * One page. Read: the saved document with Edit, Create Version and History. Edit: a draft with Save
 * and Cancel. Save replaces the saved document and creates no Version; Create Version freezes the
 * saved document into History; Restore only loads a Version into a new draft.
 */
/** A page with nothing in it: no blocks, or only paragraphs without text. */
function isEmptyDoc(blocks: unknown[]): boolean {
  return blocks.every((block) => {
    if (typeof block !== 'object' || block === null) return false
    const { type, content, children } = block as { type?: unknown; content?: unknown; children?: unknown }
    const noText = content === undefined || (Array.isArray(content) && content.every((item) => (item as { text?: unknown }).text === ''))
    return type === 'paragraph' && noText && (children === undefined || (Array.isArray(children) && children.length === 0))
  })
}

export function DocPageView({
  workspaceId,
  docId,
  startEditing = false,
  onEditStarted,
}: {
  workspaceId: string
  docId: string
  /** Open straight into editing (a page that was just created). */
  startEditing?: boolean
  /** Called once that request was carried out, so the address stops asking for it. */
  onEditStarted?: () => void
}) {
  const { t, locale } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const session = useSession()
  const page = useQuery({ ...queries.doc(docId), retry: false })
  const sharedWith = useScopeLabel(workspaceId, { visibility: page.data?.visibility ?? 'WORKSPACE', projectId: page.data?.scopeProjectId ?? null, teamId: page.data?.scopeTeamId ?? null })
  const [editing, setEditing] = useState<Editing | null>(null)
  const [titleDraft, setTitleDraft] = useState('')
  const [editorDirty, setEditorDirty] = useState(false)
  const [saving, setSaving] = useState(false)
  const [problem, setProblem] = useState<Problem>(null)
  const [discardOpen, setDiscardOpen] = useState(false)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [accessOpen, setAccessOpen] = useState(false)
  const [templateOpen, setTemplateOpen] = useState(false)
  const [templateName, setTemplateName] = useState('')
  const [notice, setNotice] = useState<string | null>(null)
  const [versioning, setVersioning] = useState(false)
  const controller = useRef<EditorController | null>(null)
  const seed = useRef(0)
  const started = useRef(false)

  const dirty = editing !== null && (editorDirty || titleDraft.trim() !== editing.title || editing.restoredFrom !== null)
  // Leaving the tab with a draft that was not saved would lose it: ask first.
  useEffect(() => {
    if (!dirty) return
    const guard = (event: BeforeUnloadEvent) => {
      event.preventDefault()
      event.returnValue = ''
    }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [dirty])

  const onController = useCallback((next: EditorController | null) => {
    controller.current = next
  }, [])

  const open = useCallback(
    (kind: ReferenceKind, id: string, hints: { projectId?: string }) => {
      const base = `/w/${workspaceId}`
      if (kind === 'TICKET' && hints.projectId) void navigate(`${base}/projects/${hints.projectId}/work?panel=ticket:${id}`)
      else if (kind === 'DOC') void navigate(`${base}/docs?doc=${id}`)
    },
    [navigate, workspaceId],
  )

  /** Starts a draft from the page as it is now on the server, or from a Version's content. */
  const beginEdit = useCallback(
    async (from?: { versionNo: number; title: string; content: DocContent }) => {
      setProblem(null)
      setNotice(null)
      try {
        const fresh = await queryClient.fetchQuery({ ...queries.doc(docId), staleTime: 0 })
        seed.current += 1
        const next: Editing = {
          seed: seed.current,
          title: from ? from.title : fresh.title,
          content: from ? from.content : fresh.content,
          baseRevision: fresh.revisionNo,
          restoredFrom: from?.versionNo ?? null,
        }
        setEditorDirty(false)
        setTitleDraft(next.title)
        setEditing(next)
        if (from) setNotice(t('docs.restored', { no: String(from.versionNo) }))
      } catch {
        setNotice(t('docs.loadError'))
      }
    },
    [docId, queryClient, t],
  )

  useEffect(() => {
    if (startEditing && !started.current && page.data?.rights.edit) {
      started.current = true
      void beginEdit()
      onEditStarted?.()
    }
  }, [startEditing, page.data, beginEdit, onEditStarted])

  async function restore(versionNo: number) {
    try {
      const version = await api.getDocVersion(docId, versionNo)
      setHistoryOpen(false)
      await beginEdit({ versionNo, title: version.title, content: version.content })
    } catch {
      setNotice(t('docs.history.loadError'))
    }
  }

  async function save() {
    if (!editing || saving) return
    const title = titleDraft.trim()
    if (!title) return
    const content = controller.current?.getContent()
    if (!content) return
    setSaving(true)
    setProblem(null)
    try {
      const saved = await api.saveDoc(docId, { expectedRevisionNo: editing.baseRevision, title, content })
      queryClient.setQueryData(queryKeys.doc(docId), saved)
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.docVersions(docId) }),
        queryClient.invalidateQueries({ queryKey: ['doc-references'] }),
      ])
      setEditing(null)
      setEditorDirty(false)
      setNotice(t('docs.saved'))
    } catch (error) {
      if (isApiError(error) && error.status === 409) setProblem('conflict')
      else if (isApiError(error) && (error.status === 422 || error.status === 400)) setProblem('invalid')
      else setProblem('error')
    } finally {
      setSaving(false)
    }
  }

  function cancel() {
    if (dirty) setDiscardOpen(true)
    else leaveEditing()
  }
  function leaveEditing() {
    setDiscardOpen(false)
    setEditing(null)
    setEditorDirty(false)
    setProblem(null)
  }
  async function loadLatest() {
    leaveEditing()
    await queryClient.invalidateQueries({ queryKey: queryKeys.doc(docId) })
  }

  async function createVersion() {
    if (!page.data || versioning) return
    setNotice(null)
    setVersioning(true)
    try {
      const version = await api.createDocVersion(docId, page.data.revisionNo)
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.doc(docId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.docVersions(docId) }),
      ])
      setNotice(t('docs.version.created', { no: String(version.versionNo) }))
    } catch (error) {
      if (isApiError(error) && error.status === 422 && error.code === 'doc_version_unchanged') setNotice(t('docs.version.unchanged'))
      else if (isApiError(error) && error.status === 409) {
        setNotice(t('docs.version.conflict'))
        void queryClient.invalidateQueries({ queryKey: queryKeys.doc(docId) })
      } else setNotice(t('docs.version.error'))
    } finally {
      setVersioning(false)
    }
  }

  if (page.isPending) return <p className="hint doc-page" role="status">{t('loading')}</p>
  if (page.isError) {
    return (
      <div className="doc-page">
        <EmptyState title={t('docs.unavailable.title')} body={isNotFound(page.error) ? t('docs.unavailable.body') : t('docs.loadError')} />
      </div>
    )
  }
  const data = page.data
  const user = session.data
  const canEdit = data.rights.edit
  const editorContent = editing ? editing.content : data.content
  const versionLine =
    data.latestVersionNo === null
      ? t('docs.version.none')
      : `${t('docs.version.latest', { no: String(data.latestVersionNo) })} · ${data.hasUnversionedChanges ? t('docs.version.pending') : t('docs.version.upToDate')}`

  return (
    <article className="doc-page">
      <div className="doc-topbar">
        <nav className="doc-crumb" aria-label={t('nav.docs')}>
          <span>{t('nav.docs')}</span>
          <span aria-hidden="true">›</span>
          <b>{(editing ? titleDraft : data.title) || t('docs.untitled')}</b>
        </nav>
        {editing ? (
          <>
            <span id="doc-edit-status" role="status" className={['doc-status', titleDraft.trim() ? '' : 'is-warning'].filter(Boolean).join(' ')}>
              {!titleDraft.trim() ? t('docs.save.needsTitle') : dirty ? t('docs.edit.unsaved') : t('docs.edit.clean')}
            </span>
            <span className="doc-actions">
              <Button variant="ghost" onClick={cancel} disabled={saving}>{t('docs.cancel')}</Button>
              <Button onClick={() => void save()} disabled={saving || !titleDraft.trim()} title={titleDraft.trim() ? undefined : t('docs.save.needsTitle')}>{saving ? t('docs.saving') : t('docs.save')}</Button>
            </span>
          </>
        ) : (
          <span className="doc-actions">
            <Button variant="secondary" onClick={() => setAccessOpen(true)}>{t('docs.access.open')}</Button>
            <Button variant="secondary" onClick={() => setHistoryOpen(true)}>{t('docs.history')}</Button>
            {canEdit ? (
              <Button variant="secondary" disabled={!data.hasUnversionedChanges || !data.title.trim() || versioning} onClick={() => void createVersion()}>{t('docs.version.create')}</Button>
            ) : null}
            {canEdit ? <Button onClick={() => void beginEdit()}>{t('docs.edit')}</Button> : null}
            <Popover label={t('docs.more')} icon={<MoreIcon />} iconOnly align="end">
              {(close) => (
                <div className="menu-list">
                  <button type="button" onClick={() => { close(); setTemplateName(data.title); setTemplateOpen(true) }}>{t('docs.template.save')}</button>
                </div>
              )}
            </Popover>
          </span>
        )}
      </div>
      <div className="doc-body">
      <div className="doc-column">
      <div className="doc-head">
        {data.visibility === 'RESTRICTED' ? <span role="img" aria-label={t('docs.restricted')}>🔒</span> : null}
        {editing ? (
          <input
            className="doc-title-input"
            aria-label={t('docs.title')}
            aria-invalid={!titleDraft.trim()}
            aria-describedby="doc-edit-status"
            autoFocus={!editing.title}
            placeholder={t('docs.title.placeholder')}
            value={titleDraft}
            onChange={(event) => setTitleDraft(event.target.value)}
          />
        ) : (
          <h1 className={['doc-title', data.title ? '' : 'is-untitled'].filter(Boolean).join(' ')}>{data.title || t('docs.untitled')}</h1>
        )}
      </div>
      {editing ? null : (
        <div className="doc-head">
          <span className="doc-meta">
            <span className="doc-badge" title={t('docs.access.visibility')}>{t('docs.access.visibility')}: {sharedWith}</span>
            <span className="doc-status">{versionLine}</span>
          </span>
        </div>
      )}
      {problem === 'conflict' ? (
        <div className="doc-banner is-danger" role="alert">
          <p>{t('docs.save.conflict')}</p>
          <div className="doc-banner-actions">
            <Button variant="secondary" onClick={() => void loadLatest()}>{t('docs.save.reload')}</Button>
          </div>
        </div>
      ) : null}
      {problem === 'invalid' ? <p className="doc-banner is-danger" role="alert">{t('docs.save.invalid')}</p> : null}
      {problem === 'error' ? <p className="doc-banner is-danger" role="alert">{t('docs.save.error')}</p> : null}
      {notice ? <p className="doc-banner" role="status">{notice}</p> : null}
      {!data.title.trim() && !editing ? <p className="doc-banner is-warning" role="status">{t('docs.untitled.notice')}</p> : null}
      {!canEdit ? <p className="doc-banner">{t('docs.readOnly')}</p> : null}
      {!editing && canEdit && isEmptyDoc(data.content.blocks) ? (
        <EmptyState title={t('docs.empty.title')} body={t('docs.empty.body')}>
          <Button onClick={() => void beginEdit()}>{t('docs.empty.start')}</Button>
        </EmptyState>
      ) : null}
      <Suspense fallback={<p className="hint" role="status">{t('loading')}</p>}>
        <DocEditor
          key={`${data.id}:${editing ? `edit${editing.seed}` : `read${data.revisionNo}`}`}
          docId={data.id}
          projectId={data.projectId}
          workspaceId={workspaceId}
          content={editorContent}
          editable={editing !== null}
          autoFocus={editing !== null && editing.title.trim().length > 0}
          onController={onController}
          onDirtyChange={setEditorDirty}
          open={open}
        />
      </Suspense>
      {user ? (
        <DocComments workspaceId={workspaceId} docId={docId} userId={user.id} canManage={data.myRole === 'MANAGER'} refreshTick={0} autoFocus={editing === null} />
      ) : null}
      </div>
      {editing ? null : <DocAside page={data} workspaceId={workspaceId} sharedWith={sharedWith} locale={locale} />}
      </div>
      <DocAccessDialog open={accessOpen} page={data} workspaceId={workspaceId} onClose={() => setAccessOpen(false)} />
      <DocHistoryDialog
        open={historyOpen}
        page={data}
        workspaceId={workspaceId}
        onClose={() => setHistoryOpen(false)}
        onRestore={canEdit ? (versionNo) => void restore(versionNo) : undefined}
        openReference={open}
      />
      <UnsavedBlocker active={dirty} />
      <Dialog open={discardOpen} title={t('docs.edit.discard.title')} onClose={() => setDiscardOpen(false)}>
        <p>{t('docs.edit.discard.body')}</p>
        <div className="dialog-actions">
          <Button variant="ghost" onClick={() => setDiscardOpen(false)}>{t('docs.edit.keep')}</Button>
          <Button onClick={leaveEditing}>{t('docs.edit.discard.confirm')}</Button>
        </div>
      </Dialog>
      <Dialog open={templateOpen} title={t('docs.template.save')} onClose={() => setTemplateOpen(false)}>
        <label className="field">
          <span>{t('docs.template.name')}</span>
          <input value={templateName} onChange={(event) => setTemplateName(event.target.value)} />
        </label>
        <p className="hint">{t('docs.template.hint')}</p>
        {data.visibility === 'RESTRICTED' ? <p className="doc-banner is-warning" role="note">{t('docs.template.restricted')}</p> : null}
        <div className="dialog-actions">
          <Button variant="ghost" onClick={() => setTemplateOpen(false)}>{t('docs.cancel')}</Button>
          <Button
            disabled={!templateName.trim()}
            onClick={() => {
              void api
                .createDocTemplate(workspaceId, templateName.trim(), docId)
                .then(() => queryClient.invalidateQueries({ queryKey: queryKeys.docTemplates(workspaceId) }))
                .then(() => setTemplateOpen(false))
                .catch(() => setNotice(t('docs.template.error')))
            }}
          >
            {t('docs.template.create')}
          </Button>
        </div>
      </Dialog>
    </article>
  )
}

type OutlineItem = { id: string; level: number; text: string }

/** The page's headings, in order, from the saved content. */
function outline(blocks: unknown[]): OutlineItem[] {
  const found: OutlineItem[] = []
  const walk = (list: unknown[]) => {
    for (const raw of list) {
      const block = raw as { id?: string; type?: string; props?: { level?: number }; content?: unknown; children?: unknown[] }
      if (block.type === 'heading' && typeof block.id === 'string' && Array.isArray(block.content)) {
        const text = block.content.map((part) => (typeof (part as { text?: unknown }).text === 'string' ? (part as { text: string }).text : '')).join('').trim()
        if (text) found.push({ id: block.id, level: block.props?.level ?? 1, text })
      }
      if (Array.isArray(block.children)) walk(block.children)
    }
  }
  walk(blocks)
  return found
}

/** The right column of frame 3.1: the outline and what the page says about itself. */
function DocAside({ page, workspaceId, sharedWith, locale }: { page: DocPage; workspaceId: string; sharedWith: string; locale: string }) {
  const { t } = useI18n()
  const people = useQuery(queries.people(workspaceId))
  const items = outline(page.content.blocks)
  const author = people.data?.find((person) => person.userId === page.createdBy)?.displayName
  return (
    <aside className="doc-aside" aria-label={t('docs.aside.info')}>
      {items.length > 0 ? (
        <section>
          <h2>{t('docs.aside.outline')}</h2>
          <ul className="doc-outline">
            {items.map((item) => (
              <li key={item.id} style={{ paddingInlineStart: `${String((item.level - 1) * 12)}px` }}>
                <button type="button" onClick={() => document.querySelector(`[data-id="${item.id}"]`)?.scrollIntoView?.({ behavior: 'smooth', block: 'start' })}>{item.text}</button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <DocReferencedBy docId={page.id} workspaceId={workspaceId} />
      <section>
        <h2>{t('docs.aside.info')}</h2>
        <dl className="doc-facts">
          {author ? <div><dt>{t('docs.aside.author')}</dt><dd>{author}</dd></div> : null}
          <div><dt>{t('docs.aside.updated')}</dt><dd>{formatMessageTime(page.updatedAt, locale)}</dd></div>
          <div><dt>{t('docs.access.visibility')}</dt><dd>{sharedWith}</dd></div>
        </dl>
      </section>
    </aside>
  )
}
