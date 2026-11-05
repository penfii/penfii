import { useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { useParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, newIdempotencyKey } from '@penfii/api-client'
import type { HierarchyKind, ProjectBoard, Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { ResizeHandle, type PaneControls } from '../../shell/ResizeHandle'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { AssigneeField, DueDateField, LabelField, ParentField, PriorityField, PropRow, TypeField } from './fields'
import type { CreateContext } from './panelParam'
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
  pane,
}: {
  context: CreateContext
  onClose: () => void
  onCreated: (ticket: Ticket) => void
  pane?: PaneControls
}) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const queryClient = useQueryClient()
  const { board, context: project } = useProjectContext(context.projectId)
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const channels = useMemo(() => (space.data?.channels ?? []).filter((channel) => channel.projectId === context.projectId), [space.data, context.projectId])

  const header = (
    <div className="ticket-panel-head">
      <div className="head-cluster">
        <button type="button" className="btn btn-ghost panel-back" onClick={onClose}>← {t('work.panel.back')}</button>
        <strong className="create-heading">{t('work.create.title')}</strong>
      </div>
      <button type="button" className="icon-button panel-close" aria-label={t('work.panel.close')} onClick={onClose}><CloseIcon /></button>
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
  } else if (!project) {
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
      if (event.key === 'Escape' && !event.defaultPrevented) onClose()
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
}: {
  projectId: string
  context: CreateContext
  ctx: NonNullable<ReturnType<typeof useProjectContext>['context']>
  channels: Array<{ id: string; name: string }>
  onCancel: () => void
  onCreated: (ticket: Ticket, another: boolean) => void
}) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [form, setForm] = useState<Form>(() => initialForm(context, ctx.board))
  const [another, setAnother] = useState(false)
  const [busy, setBusy] = useState(false)
  const { uploads, readyIds, failed: uploadFailed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy })
  const [error, setError] = useState('')
  const [titleTouched, setTitleTouched] = useState(false)
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
    try {
      const ticket = await api.createTicket(projectId, body, keyRef.current.key)
      keyRef.current = null
      settle()
      if (another) {
        setForm({ ...initialForm(context, ctx.board), kind: form.kind, parentId: form.parentId, channelId: form.channelId, assigneeId: form.assigneeId, priority: form.priority, typeId: form.typeId, labelIds: form.labelIds })
        setTitleTouched(false)
        titleRef.current?.focus()
      }
      onCreated(ticket, another)
    } catch (cause) {
      const detail = isApiError(cause) && cause.status === 400 ? ` ${cause.message}` : ''
      setError(`${t('work.create.error')}${detail}`)
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
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      <div className="create-actions">
        <label className="board-toggle">
          <input type="checkbox" checked={another} onChange={(event) => setAnother(event.target.checked)} />
          {t('work.create.another')}
        </label>
        <small className="field-hint create-shortcut">{t('work.create.hint')}</small>
        <span className="create-buttons">
          <Button type="button" variant="secondary" onClick={onCancel}>{t('work.description.cancel')}</Button>
          <Button type="submit" disabled={!canSubmit}>{busy ? t('loading') : t('work.create.submit')}</Button>
        </span>
      </div>
      </div>
    </form>
  )
}
