import { useState, type FormEvent } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import type { BoardColumn, ProjectBoard, WorkflowState } from '@penfii/contracts'
import { api } from '../../api'
import { Badge, Button, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queryKeys } from '../../queryKeys'
import { errorCode, errorDetails } from './ticketModel'

const CATEGORIES: WorkflowState['category'][] = ['NOT_STARTED', 'IN_PROGRESS', 'DONE', 'CANCELED']

/**
 * Board settings: the Project's Workflow and its States. A State is one Board column, in Workflow
 * order. Anyone who can see the Board reads the list; only a project admin changes it. The
 * server keeps the Workflow valid (an initial State, a Done State, no State left with tickets).
 */
export function BoardSettings({ projectId, board }: { projectId: string; board: ProjectBoard }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [error, setError] = useState('')
  const [name, setName] = useState('')
  const [category, setCategory] = useState<WorkflowState['category']>('IN_PROGRESS')
  const manage = board.canManageWorkflow
  const states = [...board.states].sort((a, b) => a.position - b.position || a.id.localeCompare(b.id))
  const counts = new Map<string, number>(board.columns.flatMap((column: BoardColumn) => column.stateIds.map((id) => [id, column.ticketCount] as const)))

  function explain(cause: unknown): string {
    const code = errorCode(cause)
    if (code === 'workflow_version_conflict') return t('workflow.error.conflict')
    if (code === 'forbidden') return t('workflow.error.forbidden')
    if (code === 'state_in_use') {
      const tickets = errorDetails(cause).tickets
      return typeof tickets === 'number' ? t('workflow.error.inUse', { count: String(tickets) }) : t('workflow.error.inUseCategory')
    }
    if (code === 'workflow_requires_initial') return t('workflow.error.initial')
    if (code === 'workflow_requires_done') return t('workflow.error.needDone')
    if (code === 'workflow_requires_not_started') return t('workflow.error.needNotStarted')
    if (code === 'validation_failed' || code === 'validation') return t('workflow.error.name')
    return t('workflow.error.generic')
  }

  /** Runs a change; the Board is reloaded either way so the columns match the server. */
  const change = useMutation({
    mutationFn: (action: () => Promise<unknown>) => action(),
    onError: (cause) => setError(explain(cause)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }),
  })
  const busy = change.isPending
  function run(action: () => Promise<unknown>): Promise<boolean> {
    setError('')
    return change.mutateAsync(action).then(() => true, () => false)
  }

  async function add(event: FormEvent) {
    event.preventDefault()
    const trimmed = name.trim()
    if (!trimmed || busy) return
    const position = states.reduce((max, state) => Math.max(max, state.position), -1) + 1
    const ok = await run(() => api.createWorkflowState(projectId, { expectedWorkflowVersion: board.workflowVersion, name: trimmed, category, position }))
    if (ok) setName('')
  }

  function swap(index: number, step: -1 | 1) {
    const a = states[index]
    const b = states[index + step]
    if (!a || !b) return
    void run(async () => {
      // Two small changes in a row: the second uses the version the first returned.
      const first = await api.updateWorkflowState(projectId, a.id, { expectedWorkflowVersion: board.workflowVersion, position: b.position === a.position ? b.position + step : b.position })
      await api.updateWorkflowState(projectId, b.id, { expectedWorkflowVersion: first.version, position: a.position })
    })
  }

  return (
    <div className="board-settings">
      <strong>{t('workflow.title')}</strong>
      <p className="hint">{t('workflow.lead')}</p>
      {!manage ? <p className="board-note" role="note">{t('workflow.readOnly')}</p> : null}
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      <ul className="planning-list workflow-list" aria-label={t('workflow.title')}>
        {states.map((state, index) => (
          <StateRow
            key={`${state.id}:${state.name}:${state.category}`}
            state={state}
            count={counts.get(state.id) ?? 0}
            manage={manage}
            busy={busy}
            first={index === 0}
            last={index === states.length - 1}
            onSave={(change) => run(() => api.updateWorkflowState(projectId, state.id, { expectedWorkflowVersion: board.workflowVersion, ...change }))}
            onMove={(step) => swap(index, step)}
            onDelete={() => run(() => api.deleteWorkflowState(projectId, state.id, board.workflowVersion))}
          />
        ))}
      </ul>
      {manage ? (
        <form className="planning-form" onSubmit={(event) => void add(event)}>
          <strong>{t('workflow.add')}</strong>
          <input aria-label={t('workflow.add.name')} placeholder={t('workflow.add.name')} value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
          <label className="planning-target">
            <span>{t('workflow.add.category')}</span>
            <select value={category} onChange={(event) => setCategory(event.target.value as WorkflowState['category'])}>
              {CATEGORIES.map((item) => (
                <option key={item} value={item}>{t(`workflow.category.${item}` as MessageKey)}</option>
              ))}
            </select>
          </label>
          <small className="field-hint">{t('workflow.add.help')}</small>
          <Button type="submit" disabled={!name.trim() || busy}>{t('workflow.add.submit')}</Button>
        </form>
      ) : null}
    </div>
  )
}

function StateRow({
  state,
  count,
  manage,
  busy,
  first,
  last,
  onSave,
  onMove,
  onDelete,
}: {
  state: WorkflowState
  count: number
  manage: boolean
  busy: boolean
  first: boolean
  last: boolean
  onSave: (change: { name?: string; category?: WorkflowState['category']; isInitial?: boolean }) => Promise<boolean>
  onMove: (step: -1 | 1) => void
  onDelete: () => Promise<boolean>
}) {
  const { t } = useI18n()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [name, setName] = useState(state.name)
  const [category, setCategory] = useState(state.category)
  const dirty = name.trim() !== state.name || category !== state.category
  return (
    <li className="planning-item workflow-item" data-state-id={state.id}>
      {manage ? (
        <>
          <div className="planning-item-main">
            <input aria-label={t('workflow.rename', { name: state.name })} value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
            <select aria-label={t('workflow.category', { name: state.name })} value={category} onChange={(event) => setCategory(event.target.value as WorkflowState['category'])}>
              {CATEGORIES.map((item) => (
                <option key={item} value={item}>{t(`workflow.category.${item}` as MessageKey)}</option>
              ))}
            </select>
            <small>{t('workflow.count', { count: String(count) })}</small>
            {state.isInitial ? <Badge tone="accent">{t('workflow.initial')}</Badge> : null}
          </div>
          <div className="planning-actions">
            <Button type="button" variant="ghost" disabled={busy || first} aria-label={t('workflow.up', { name: state.name })} onClick={() => onMove(-1)}>↑</Button>
            <Button type="button" variant="ghost" disabled={busy || last} aria-label={t('workflow.down', { name: state.name })} onClick={() => onMove(1)}>↓</Button>
            {dirty ? <Button type="button" variant="secondary" disabled={busy || !name.trim()} onClick={() => void onSave({ ...(name.trim() !== state.name ? { name: name.trim() } : {}), ...(category !== state.category ? { category } : {}) })}>{t('workflow.save')}</Button> : null}
            {!state.isInitial && state.category === 'NOT_STARTED' ? <Button type="button" variant="ghost" disabled={busy} onClick={() => void onSave({ isInitial: true })}>{t('workflow.makeInitial')}</Button> : null}
            <Button type="button" variant="ghost" disabled={busy} aria-label={`${t('workflow.delete')} ${state.name}`} onClick={() => {
              void confirm({ title: t('workflow.delete'), body: t('workflow.delete.confirm', { name: state.name }), confirmLabel: t('workflow.delete') }).then((ok) => {
                if (ok) void onDelete()
              })
            }}>{t('workflow.delete')}</Button>
          </div>
        </>
      ) : (
        <div className="planning-item-main">
          <strong>{state.name}</strong>
          <Badge tone="neutral">{t(`workflow.category.${state.category}` as MessageKey)}</Badge>
          <small>{t('workflow.count', { count: String(count) })}</small>
          {state.isInitial ? <Badge tone="accent">{t('workflow.initial')}</Badge> : null}
        </div>
      )}
      {confirmDialog}
    </li>
  )
}
