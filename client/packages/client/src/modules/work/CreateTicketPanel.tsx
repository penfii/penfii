import { useContext, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MutableRefObject } from 'react'
import { UNSAFE_DataRouterContext, useBlocker, useNavigate, useParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, newIdempotencyKey } from '@penfii/api-client'
import type { HierarchyKind, ProjectBoard, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { notify } from '../../design/notices'
import { useSession } from '../identity'
import { formatStamp } from '../../format'
import { ResizeHandle, type PaneControls } from '../../shell/ResizeHandle'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { AssigneeField, DueDateField, LabelField, ParentField, PriorityField, PropRow, TypeField } from './fields'
import { CREATE_PANEL, type CreateContext } from './panelParam'
import { clearTicketDraft, readTicketDraft, restorable, ticketDraftKey, writeTicketDraft, type TicketDraft } from './ticketDraft'
import { isEstimateText, normalizeEstimateText } from './planValues'
import { recallPrefill } from './fromChat'
import { refreshTicket } from './refresh'
import { allTickets, parentCandidates, parentKind, textToDescription } from './ticketModel'
import { useProjectContext } from './useProjectContext'
import { CloseIcon } from '../../design/icons'
import { AttachButton } from '../files/AttachButton'
import { UploadCards } from '../files/AttachmentList'
import { FileDropZone } from '../files/FileDropZone'
import { useFileUploads } from '../files/useFileUploads'

type Form = {
  title: string
  description: string
  kind: HierarchyKind
  parentId: string | null
  channelId: string | null
  assigneeId: string | null
  priority: string | null
  dueOn: string | null
  typeId: string | null
  labelIds: string[]
  /** Task only, in the Project's unit; empty is unestimated. */
  estimate: string
}

function initialForm(context: CreateContext, board: ProjectBoard | undefined): Form {
  const parent = context.parentId && board ? allTickets(board).find((item) => item.id === context.parentId) : undefined
  const kind: HierarchyKind = context.kind ?? (parent ? (parent.hierarchyKind === 'EPIC' ? 'TASK' : 'SUBTASK') : 'TASK')
  const prefill = context.source ? recallPrefill(context.source.messageId) : null
  return {
    title: prefill?.title ?? '',
    description: prefill?.description ?? '',
    kind,
    parentId: parent?.id ?? null,
    // A Subtask stays in its parent's channel; otherwise the channel comes from where Create was opened.
    channelId: kind === 'EPIC' ? null : kind === 'SUBTASK' && parent ? parent.channelId : (context.channelId ?? null),
    assigneeId: context.assigneeId ?? null,
    priority: null,
    dueOn: null,
    typeId: null,
    labelIds: [],
    estimate: '',
  }
}

function formFromDraft(draft: TicketDraft): Form {
  return {
    title: draft.title,
    description: draft.description,
    kind: draft.kind,
    parentId: draft.parentId,
    channelId: draft.channelId,
    assigneeId: draft.assigneeId,
    priority: draft.priority,
    dueOn: draft.dueOn,
    typeId: draft.typeId,
    labelIds: draft.labelIds,
    estimate: draft.estimate,
  }
}

/** Equal forms give equal text; used to tell an edit from no edit. */
function formSignature(form: Form, attachments: number): string {
  return JSON.stringify([form.title.trim(), form.description.trim(), form.kind, form.parentId, form.channelId, form.assigneeId, form.priority, form.dueOn, form.typeId, [...form.labelIds].sort(), form.estimate.trim(), attachments > 0])
}

function withBoardAdded(board: ProjectBoard, ticket: Ticket): ProjectBoard {
  return {
    ...board,
    columns: board.columns.map((column) => (column.stateIds.includes(ticket.statusId) ? { ...column, tickets: [...column.tickets, ticket] } : column)),
  }
}

/**
 * The one way to make a Ticket. Everything that can be set at creation is set here, so a new
 * Ticket is complete when it appears. The Project (and often the parent or channel) comes from
 * where Create was opened. A new Ticket always starts in the Project's initial state.
 */
export function CreateTicketPanel({
  context,
  onClose,
  onCreated,
  onOpenTicket,
  pane,
}: {
  context: CreateContext
  onClose: () => void
  onCreated: (ticket: Ticket) => void
  /** Shows a Ticket made here ("Open" beside the result of Create another). */
  onOpenTicket?: (ticket: Ticket) => void
  pane?: PaneControls
}) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const queryClient = useQueryClient()
  // Every way of closing (Esc, ×, Cancel, ← Back) goes through the form, which knows about attachments and the draft.
  const closeGuard = useRef<(() => void) | null>(null)
  const requestClose = () => (closeGuard.current ?? onClose)()
  const { board, context: project, ready } = useProjectContext(context.projectId)
  // The kept draft is looked up under the signed-in person's id (the shell has read it long before a panel opens).
  const session = useSession()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const channels = useMemo(() => (space.data?.channels ?? []).filter((channel) => channel.projectId === context.projectId), [space.data, context.projectId])

  const header = (
    <div className="ticket-panel-head">
      <div className="head-cluster">
        <button type="button" className="btn btn-ghost panel-back" onClick={requestClose}>← {t('work.panel.back')}</button>
        <strong className="create-heading">{t('work.create.title')}</strong>
      </div>
      <button type="button" className="icon-button panel-close" aria-label={t('work.panel.close')} onClick={requestClose}><CloseIcon /></button>
    </div>
  )
  let body
  if (board.isError) {
    body = (
      <p className="hint" role="alert">
        {isApiError(board.error) && board.error.status === 403 ? t('projects.boardPermissionError') : t('work.panel.error')}{' '}
        <Button variant="secondary" onClick={() => void board.refetch()}>{t('work.panel.retry')}</Button>
      </p>
    )
  } else if (!project || !ready || space.isPending) {
    body = <p className="hint" role="status">{t('work.panel.loading')}</p>
  } else if (!project.board.canEdit) {
    body = <p className="board-note" role="note">{t('work.create.noAccess')}</p>
  } else {
    body = (
      <CreateForm
        projectId={context.projectId}
        context={context}
        ctx={project}
        channels={channels.map((channel) => ({ id: channel.id, name: channel.name }))}
        onCancel={onClose}
        closeGuard={closeGuard}
        userId={session.data?.id}
        onOpenTicket={onOpenTicket}
        onCreated={(ticket, another) => {
          const current = queryClient.getQueryData<ProjectBoard>(queryKeys.projectBoard(context.projectId))
          if (current) queryClient.setQueryData(queryKeys.projectBoard(context.projectId), withBoardAdded(current, ticket))
          void refreshTicket(queryClient, ticket.id, context.projectId)
          if (!another) onCreated(ticket)
        }}
      />
    )
  }
  return (
    <aside className="detail-panel ticket-panel create-panel" aria-label={t('work.create.title')} onKeyDown={(event) => {
      // A question asked inside the panel (its own dialog) answers its own Esc.
      if (event.key === 'Escape' && !event.defaultPrevented && !(event.target instanceof Element && event.target.closest('dialog'))) {
        // The close may open a question; the browser's own Esc handling must not answer it in the same keypress.
        event.preventDefault()
        requestClose()
      }
    }}>
      {pane ? <ResizeHandle pane={pane} label={t('work.create.resize')} resetLabel={t('layout.resetWidth')} /> : null}
      {header}
      <div className="create-body">{body}</div>
    </aside>
  )
}

function CreateForm({
  projectId,
  context,
  ctx,
  channels,
  onCancel,
  onCreated,
  closeGuard,
  userId,
  onOpenTicket,
}: {
  projectId: string
  context: CreateContext
  ctx: NonNullable<ReturnType<typeof useProjectContext>['context']>
  channels: Array<{ id: string; name: string }>
  onCancel: () => void
  onCreated: (ticket: Ticket, another: boolean) => void
  closeGuard: MutableRefObject<(() => void) | null>
  /** Whose draft this is; without it (no session) nothing is kept. */
  userId: string | undefined
  onOpenTicket?: (ticket: Ticket) => void
}) {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const navigate = useNavigate()
  const draftKey = userId ? ticketDraftKey(userId, workspaceId, projectId, { messageId: context.source?.messageId, parentId: context.parentId }) : null
  // What the form starts with when nothing is kept; the draft is compared against it and Discard returns to it.
  const baseline = useRef<Form>(initialForm(context, ctx.board))
  // The kept draft of this person and place, read once when the form opens.
  const [restored, setRestored] = useState(() => {
    const kept = draftKey ? readTicketDraft(draftKey) : null
    if (!kept) return null
    const valid = {
      labelIds: new Set(ctx.labels.map((item) => item.id)),
      memberIds: new Set(ctx.members.map((item) => item.userId)),
      typeIds: new Set(ctx.types.map((item) => item.id)),
      ticketIds: new Set(allTickets(ctx.board).map((item) => item.id)),
      channelIds: new Set(channels.map((item) => item.id)),
    }
    return restorable(kept, valid)
  })
  const [form, setForm] = useState<Form>(() => (restored ? formFromDraft(restored.draft) : baseline.current))
  const [another, setAnother] = useState(false)
  const [busy, setBusy] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy })
  const [error, setError] = useState('')
  const [created, setCreated] = useState<Ticket | null>(null)
  const [titleTouched, setTitleTouched] = useState(false)
  const attachments = uploads.filter((item) => item.phase !== 'failed').length
  // Draft keeping: what is on disk now (so a restored draft that is not edited keeps its age), whether storage accepted it,
  // and whether the form is done with it (created, or navigating on after the person chose to leave).
  const savedSignature = useRef(formSignature(restored ? formFromDraft(restored.draft) : baseline.current, 0))
  const [storageFailed, setStorageFailed] = useState(false)
  const finished = useRef(false)
  const bypass = useRef(false)
  const [confirm, setConfirm] = useState<'files' | 'noStorage' | null>(null)
  const [discarding, setDiscarding] = useState(false)
  const changed = formSignature(form, attachments) !== formSignature(baseline.current, 0)
  const latest = useRef({ form, attachments })
  latest.current = { form, attachments }

  /** Writes the draft (or removes it when the form is back at its start). Returns false when storage refused it. */
  function persist(): boolean {
    if (!draftKey || finished.current) return true
    const { form: current, attachments: files } = latest.current
    const signature = formSignature(current, files)
    if (signature === savedSignature.current) return true
    if (signature === formSignature(baseline.current, 0)) {
      clearTicketDraft(draftKey)
      savedSignature.current = signature
      return true
    }
    const ok = writeTicketDraft(draftKey, { ...current, hadAttachments: files > 0, updatedAt: Date.now() })
    if (ok) savedSignature.current = signature
    return ok
  }
  const persistRef = useRef(persist)
  persistRef.current = persist
  useEffect(() => {
    const timer = setTimeout(() => setStorageFailed(!persistRef.current()), 300)
    return () => clearTimeout(timer)
  }, [form, attachments])
  // Whatever closes the form (a link, the browser's or the phone's Back, a reload), the last edit is kept, and a panel that goes away
  // without `leave` still says so. The notice waits a moment: React's development check unmounts and mounts once on purpose.
  const mounted = useRef(false)
  const announceRef = useRef<(stored: boolean) => void>(() => undefined)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      const stored = persistRef.current()
      setTimeout(() => {
        if (!mounted.current) announceRef.current(stored)
      })
    }
  }, [])
  useEffect(() => {
    if (restored) titleRef.current?.setSelectionRange(form.title.length, form.title.length)
    // Only when the form opens: the caret goes to the end of the restored title.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  // Leaving the page with files attached asks the browser's own question: the files would be deleted.
  useEffect(() => {
    if (attachments === 0) return
    const ask = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', ask)
    return () => window.removeEventListener('beforeunload', ask)
  }, [attachments])

  /** Says the draft was kept (when there was one and storage took it), with a way back to it. Once per form. */
  function announceKept(stored: boolean) {
    if (finished.current || !draftKey) return
    finished.current = true
    const { form: current, attachments: files } = latest.current
    if (!stored || formSignature(current, files) === formSignature(baseline.current, 0)) return
    notify({ message: files > 0 ? t('work.draft.keptNoFiles') : t('work.draft.kept'), action: { label: t('work.draft.reopen'), run: reopen } })
  }
  announceRef.current = announceKept

  /** Opens this same form again (the "Reopen" action of the notice shown after closing). */
  function reopen() {
    const next = new URLSearchParams(window.location.search)
    next.set('panel', CREATE_PANEL)
    next.set('newProject', projectId)
    if (context.parentId) next.set('newParent', context.parentId)
    if (context.kind) next.set('newKind', context.kind)
    if (context.channelId) next.set('newChannel', context.channelId)
    if (context.assigneeId) next.set('newAssignee', context.assigneeId)
    if (context.source) {
      next.set('newFromConv', context.source.conversationId)
      next.set('newFromMsg', context.source.messageId)
      next.set('newFromRoot', context.source.threadRootId)
    }
    void navigate(`${window.location.pathname}?${next.toString()}`)
  }

  /** Closes the panel for good: the draft is kept and, when there was something to keep, the person is told how to get it back. */
  function leave(throughBlocker?: () => void) {
    bypass.current = true
    announceKept(persist())
    if (throughBlocker) throughBlocker()
    else onCancel()
  }

  function requestClose() {
    if (busy) return
    if (attachments > 0) setConfirm('files')
    else if (changed && storageFailed) setConfirm('noStorage')
    else leave()
  }
  closeGuard.current = requestClose

  function discard() {
    if (draftKey) clearTicketDraft(draftKey)
    savedSignature.current = formSignature(baseline.current, attachments)
    setRestored(null)
    setForm(baseline.current)
    setTitleTouched(false)
    setDiscarding(false)
    setStorageFailed(false)
    setTimeout(() => titleRef.current?.focus())
  }
  const keyRef = useRef<{ signature: string; key: string } | null>(null)
  const titleRef = useRef<HTMLInputElement>(null)
  const initialState = ctx.board.states.find((state) => state.isInitial)
  const parentWanted = parentKind(form.kind)
  const parents = parentCandidates(form.kind, form.channelId, null, ctx.board)
  const parent = form.parentId ? allTickets(ctx.board).find((item) => item.id === form.parentId) : undefined
  const subtaskNeedsParent = form.kind === 'SUBTASK' && !form.parentId
  const titleMissing = !form.title.trim()
  const estimateWanted = form.kind === 'TASK' && (ctx.board.estimateUnit ?? 'NONE') !== 'NONE'
  const estimateInvalid = estimateWanted && form.estimate.trim() !== '' && !isEstimateText(form.estimate)
  const canSubmit = !titleMissing && !subtaskNeedsParent && !estimateInvalid && !busy && !blocking && initialState !== undefined
  const optionsOpen = Boolean(context.parentId || context.kind || context.channelId)

  function patch(next: Partial<Form>) {
    setForm((current) => ({ ...current, ...next }))
  }

  function changeKind(kind: HierarchyKind) {
    // Changing the kind can make the parent or channel invalid; start those fresh.
    setForm((current) => ({ ...current, kind, parentId: null, channelId: kind === 'EPIC' ? null : current.channelId }))
  }

  function changeParent(parentId: string | null) {
    const chosen = parentId ? allTickets(ctx.board).find((item) => item.id === parentId) : undefined
    setForm((current) => ({ ...current, parentId, channelId: current.kind === 'SUBTASK' && chosen ? chosen.channelId : current.channelId }))
  }

  async function submit() {
    setTitleTouched(true)
    if (!canSubmit || !initialState) return
    const body = {
      title: form.title.trim(),
      description: form.description.trim() ? textToDescription(form.description) : undefined,
      hierarchyKind: form.kind,
      parentId: form.parentId,
      channelId: form.kind === 'EPIC' ? null : form.channelId,
      statusId: initialState.id,
      assigneeId: form.assigneeId,
      priority: form.priority,
      dueOn: form.dueOn,
      labelIds: form.labelIds,
      typeId: form.typeId,
      estimate: estimateWanted && form.estimate.trim() !== '' ? { value: normalizeEstimateText(form.estimate) } : undefined,
      // The files are created with the Ticket, in the same request.
      fileIds: readyIds.length > 0 ? readyIds : undefined,
    }
    // A retry of the same form reuses its key, so a lost response cannot create twice.
    const signature = JSON.stringify(body)
    if (keyRef.current?.signature !== signature) keyRef.current = { signature, key: newIdempotencyKey() }
    setBusy(true)
    setError('')
    setCreated(null)
    try {
      const ticket = await api.createTicket(projectId, body, keyRef.current.key)
      keyRef.current = null
      settle()
      // The Ticket exists: its draft is done with, wherever the form goes next.
      if (draftKey) clearTicketDraft(draftKey)
      setRestored(null)
      if (another) {
        const next = { ...initialForm(context, ctx.board), kind: form.kind, parentId: form.parentId, channelId: form.channelId, assigneeId: form.assigneeId, priority: form.priority, typeId: form.typeId, labelIds: form.labelIds }
        // What is kept for the next Ticket is the new start, not an unsaved edit.
        baseline.current = next
        savedSignature.current = formSignature(next, 0)
        setForm(next)
        setCreated(ticket)
        setTitleTouched(false)
        titleRef.current?.focus()
      } else {
        finished.current = true
        bypass.current = true
      }
      onCreated(ticket, another)
    } catch {
      // The server's own message (English, technical) is not shown; what was typed stays and the draft keeps it.
      setError(t('work.create.error'))
    } finally {
      setBusy(false)
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) {
      event.preventDefault()
      void submit()
    }
  }

  return (
    <form
      className="create-form"
      noValidate
      onKeyDown={onKeyDown}
      onSubmit={(event) => {
        event.preventDefault()
        void submit()
      }}
    >
      <div className="create-fields">
      <div className="kind-switch" role="radiogroup" aria-label={t('work.create.kind')}>
        {(['EPIC', 'TASK', 'SUBTASK'] as const).map((kind) => (
          <label key={kind} className={form.kind === kind ? 'is-selected' : undefined}>
            <input type="radio" name="ticket-kind" value={kind} checked={form.kind === kind} disabled={busy} onChange={() => changeKind(kind)} />
            {t(`work.kind.${kind}`)}
          </label>
        ))}
      </div>
      <small className="field-hint">{t('work.create.kindHint')}</small>
      {restored ? (
        <div className="draft-restored" role="status">
          <span>
            {t('work.draft.restored', { time: formatStamp(new Date(restored.draft.updatedAt).toISOString(), locale) })}
            {restored.draft.hadAttachments ? ` ${t('work.draft.attachmentsLost')}` : ''}
            {restored.dropped ? ` ${t('work.draft.partial')}` : ''}
          </span>
          <button type="button" className="link-button" disabled={busy} onClick={() => setDiscarding(true)}>{t('work.draft.discard')}</button>
        </div>
      ) : null}
      {storageFailed ? <p className="board-error" role="alert">{t('work.draft.storageFailed')}</p> : null}
      {context.source ? <p className="create-for" role="note">{t('work.create.fromChat')}</p> : null}
      {parent ? <p className="create-for" role="note">{t('work.create.forParent', { key: parent.key, title: parent.title })}</p> : null}
      <label className="field">
        <span>{t('work.field.title')} *</span>
        <input
          ref={titleRef}
          autoFocus
          required
          maxLength={300}
          aria-invalid={titleTouched && titleMissing}
          placeholder={t('work.quick.placeholder')}
          value={form.title}
          onChange={(event) => patch({ title: event.target.value })}
          onBlur={() => setTitleTouched(true)}
        />
        {titleTouched && titleMissing ? <small className="field-error" role="alert">{t('work.create.titleRequired')}</small> : null}
      </label>
      <FileDropZone onFiles={addFiles} disabled={busy} className="create-drop">
        <label className="field">
        <span>{t('work.field.description')}</span>
        <textarea className="create-description" rows={10} placeholder={t('work.description.placeholder')} value={form.description} onChange={(event) => patch({ description: event.target.value })} />
      </label>
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {uploadFailed ? <p className="board-error" role="alert">{t('composer.failedBlocks')}</p> : null}
        <div className="attachment-actions"><AttachButton onFiles={addFiles} disabled={busy} /></div>
      </FileDropZone>

      <div className="prop-grid">
        <PropRow label={t('work.field.assignee')}>
          <AssigneeField value={form.assigneeId} members={ctx.members} disabled={busy} onChange={(assigneeId) => patch({ assigneeId })} />
        </PropRow>
        <PropRow label={t('work.field.priority')}>
          <PriorityField value={form.priority} disabled={busy} onChange={(priority) => patch({ priority })} />
        </PropRow>
      </div>

      <details className="more-properties" open={optionsOpen}>
        <summary>{t('work.create.more')}</summary>
        <div className="prop-grid">
          <PropRow label={t('work.field.type')}>
            <TypeField value={form.typeId} types={ctx.types} disabled={busy} onChange={(typeId) => patch({ typeId })} />
          </PropRow>
          <PropRow label={t('work.field.status')}>
            <div>
              <select aria-label={t('work.field.status')} value={initialState?.id ?? ''} disabled>
                {initialState ? <option value={initialState.id}>{initialState.name}</option> : null}
              </select>
              <small className="field-hint">{t('work.create.stateHint')}</small>
            </div>
          </PropRow>
          <PropRow label={t('work.field.labels')}>
            <LabelField projectId={projectId} labels={ctx.labels} selected={form.labelIds} disabled={busy} canCreate={ctx.board.canEdit} onChange={(labelIds) => patch({ labelIds })} />
          </PropRow>
          <PropRow label={t('work.field.dueOn')}>
            <DueDateField value={form.dueOn} disabled={busy} onChange={(dueOn) => patch({ dueOn })} />
          </PropRow>
          {estimateWanted ? (
            <PropRow label={t('work.field.estimate')}>
              <div className="prop-inline estimate-field">
                <input
                  type="text"
                  inputMode="decimal"
                  aria-label={t('work.field.estimate')}
                  aria-invalid={estimateInvalid}
                  placeholder={t('work.estimate.placeholder')}
                  value={form.estimate}
                  disabled={busy}
                  onChange={(event) => patch({ estimate: event.target.value })}
                />
                <span className="estimate-unit" aria-hidden="true">{ctx.board.estimateUnit === 'HOURS' ? 'h' : 'pt'}</span>
                {estimateInvalid ? <small className="field-error" role="alert">{t('work.estimate.invalid')}</small> : null}
              </div>
            </PropRow>
          ) : null}
          {parentWanted ? (
            <PropRow label={t('work.field.parent')}>
              <div>
                <ParentField
                  label={t('work.field.parent')}
                  value={form.parentId}
                  options={parents}
                  allowNone={form.kind === 'TASK'}
                  disabled={busy}
                  onChange={changeParent}
                />
                {subtaskNeedsParent ? <small className="field-hint">{t('work.parent.subtaskRequired')}</small> : null}
              </div>
            </PropRow>
          ) : null}
          {form.kind !== 'EPIC' ? (
            <PropRow label={t('work.field.channel')}>
              <div>
                <select
                  aria-label={t('work.field.channel')}
                  value={form.channelId ?? ''}
                  disabled={busy || (form.kind === 'SUBTASK' && parent !== undefined)}
                  onChange={(event) => setForm((current) => ({ ...current, channelId: event.target.value || null, parentId: current.kind === 'SUBTASK' ? null : current.parentId }))}
                >
                  <option value="">{t('work.channel.project')}</option>
                  {channels.map((channel) => (
                    <option key={channel.id} value={channel.id}>#{channel.name}</option>
                  ))}
                </select>
                {form.kind === 'SUBTASK' && parent ? <small className="field-hint">{t('work.create.channelFromParent')}</small> : null}
              </div>
            </PropRow>
          ) : null}
        </div>
      </details>

      </div>
      <div className="create-footer">
      {created ? (
        <p className="create-result" role="status">
          {t('work.create.created', { key: created.key })}
          {onOpenTicket ? <> · <button type="button" className="link-button" onClick={() => onOpenTicket(created)}>{t('work.create.open')}</button></> : null}
        </p>
      ) : null}
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      <div className="create-actions">
        <label className="board-toggle">
          <input type="checkbox" checked={another} onChange={(event) => setAnother(event.target.checked)} />
          {t('work.create.another')}
        </label>
        <small className="field-hint create-shortcut">{t('work.create.hint')}</small>
        <span className="create-buttons">
          <Button type="button" variant="secondary" onClick={requestClose}>{t('work.description.cancel')}</Button>
          <Button type="submit" disabled={!canSubmit}>{busy ? t('loading') : t('work.create.submit')}</Button>
        </span>
      </div>
      </div>
      <CloseDialog open={confirm !== null} kind={confirm ?? 'files'} count={attachments} onKeep={() => setConfirm(null)} onLeave={() => { setConfirm(null); leave() }} />
      <LeaveBlocker
        active={!busy && (attachments > 0 || (changed && storageFailed))}
        kind={attachments > 0 ? 'files' : 'noStorage'}
        bypass={bypass}
        count={attachments}
        onLeave={(proceed) => leave(proceed)}
      />
      <Dialog open={discarding} title={t('work.draft.discard.title')} onClose={() => setDiscarding(false)} closeWithBack={false}>
        <p>{t('work.draft.discard.body')}</p>
        <div className="dialog-actions">
          <Button type="button" variant="secondary" data-autofocus onClick={() => setDiscarding(false)}>{t('work.draft.keep')}</Button>
          <Button type="button" variant="danger" onClick={discard}>{t('work.draft.discard.confirm')}</Button>
        </div>
      </Dialog>
    </form>
  )
}

/** The question asked before a close that would lose something: attached files, or text when the browser cannot keep a draft. */
function CloseDialog({ open, kind, count, onKeep, onLeave }: { open: boolean; kind: 'files' | 'noStorage'; count: number; onKeep: () => void; onLeave: () => void }) {
  const { t } = useI18n()
  return (
    <Dialog open={open} title={kind === 'files' ? t('work.draft.leave.title') : t('work.draft.noStorage.title')} onClose={onKeep} closeWithBack={false}>
      <p>{kind === 'files' ? t('work.draft.leave.body', { count: String(count) }) : t('work.draft.noStorage.body')}</p>
      <div className="dialog-actions">
        <Button type="button" variant="secondary" data-autofocus onClick={onKeep}>{t('work.draft.keep')}</Button>
        <Button type="button" variant="danger" onClick={onLeave}>{t('work.draft.leave.confirm')}</Button>
      </div>
    </Dialog>
  )
}

type LeaveBlockerProps = { active: boolean; kind: 'files' | 'noStorage'; bypass: MutableRefObject<boolean>; count: number; onLeave: (proceed: () => void) => void }

/**
 * Asks before the app navigates away (a link, the browser's Back, another screen) when that would lose something; a close
 * from inside the panel is asked about by the form itself. It needs a data router and renders nothing without one.
 */
function LeaveBlocker(props: LeaveBlockerProps) {
  return useContext(UNSAFE_DataRouterContext) ? <Blocking {...props} /> : null
}

function Blocking({ active, kind, bypass, count, onLeave }: LeaveBlockerProps) {
  const blocker = useBlocker(({ currentLocation, nextLocation }) => active && !bypass.current && (currentLocation.pathname !== nextLocation.pathname || currentLocation.search !== nextLocation.search))
  return <CloseDialog open={blocker.state === 'blocked'} kind={kind} count={count} onKeep={() => blocker.reset?.()} onLeave={() => onLeave(() => blocker.proceed?.())} />
}
