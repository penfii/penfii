import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import { useTicketPanel } from './panelParam'
import type { Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { Avatar, Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import type { ProjectContext } from './projectContext'
import { refreshTicket } from './refresh'
import { childKind, eligibleParents, parentKind, progressPercent } from './ticketModel'
import type { TicketPatch } from './useTicketEditor'

/**
 * Where this Ticket sits in EPIC → TASK → SUBTASK, and the direct children below it. Parent and
 * child states are never derived from each other; this only shows counts and lets you jump.
 */
export function HierarchySection({
  ticket,
  parent,
  ctx,
  canEdit,
  busy,
  onSave,
  onOpen,
}: {
  ticket: Ticket
  /** The parent loaded in full, so a Subtask can show its Epic above the Task. */
  parent: Ticket | undefined
  ctx: ProjectContext
  canEdit: boolean
  busy: boolean
  onSave: (patch: TicketPatch) => void
  onOpen: (ticketId: string) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const kind = ticket.hierarchyKind
  const nextKind = childKind(kind)
  const wantsParent = parentKind(kind)
  const children = useQuery({ ...queries.ticketChildren(ticket.id), enabled: nextKind !== null })
  const [showAllChildren, setShowAllChildren] = useState(false)
  const [hideDone, setHideDone] = useState(false)
  const [collapsed, setCollapsed] = useState(false)
  const { openCreate } = useTicketPanel()
  const [togglingId, setTogglingId] = useState<string | null>(null)
  const parents = eligibleParents(ticket, ctx.board)
  const states = new Map(ctx.board.states.map((state) => [state.id, state]))
  const doneState = ctx.board.states.find((state) => state.category === 'DONE')
  const initialState = ctx.board.states.find((state) => state.isInitial)
  const summary = ticket.childSummary

  async function toggleDone(child: Ticket) {
    const finished = states.get(child.statusId)?.category === 'DONE'
    const target = finished ? initialState : doneState
    if (!target || togglingId) return
    setTogglingId(child.id)
    try {
      await api.changeTicketStatus(child.id, { expectedRevision: child.revisionNo, statusId: target.id }, newIdempotencyKey())
    } catch {
      // The list refreshes below, so the row shows the state the server kept.
    } finally {
      setTogglingId(null)
      await refreshTicket(queryClient, child.id, ticket.projectId)
    }
  }

  const chain = [parent?.parent ?? null, ticket.parent].filter((item): item is NonNullable<typeof item> => item !== null)

  return (
    <section className="ticket-section hierarchy" aria-label={t('work.hierarchy.label')}>
      <h3>{t('work.hierarchy.label')}</h3>
      {chain.length > 0 || wantsParent ? (
        <ol className="hierarchy-tree" aria-label={t('work.field.parent')}>
          {chain.map((ancestor, depth) => (
            <li key={ancestor.id} style={{ ['--depth' as string]: depth }}>
              <button type="button" className="hierarchy-link" aria-label={t('work.hierarchy.open', { key: ancestor.key })} onClick={() => onOpen(ancestor.id)}>
                <span className={`kind-chip kind-${ancestor.hierarchyKind.toLowerCase()}`}>{t(`work.kind.${ancestor.hierarchyKind}`)}</span>
                <small className="ticket-key">{ancestor.key}</small>
                <span className="hierarchy-title">{ancestor.title}</span>
              </button>
            </li>
          ))}
          <li className="current" style={{ ['--depth' as string]: chain.length }} aria-current="true">
            <span className={`kind-chip kind-${kind.toLowerCase()}`}>{t(`work.kind.${kind}`)}</span>
            <small className="ticket-key">{ticket.key}</small>
            <span className="hierarchy-title">{ticket.title}</span>
          </li>
        </ol>
      ) : null}
      {wantsParent && canEdit ? (
        <label className="parent-picker">
          <span className="prop-label">{t('work.field.parent')}</span>
          <select
            aria-label={t('work.parent.change')}
            value={ticket.parentId ?? ''}
            disabled={busy}
            onChange={(event) => onSave({ parentId: event.target.value || null })}
          >
            {kind === 'TASK' ? <option value="">{t('work.parent.none')}</option> : null}
            {ticket.parent && !parents.some((item) => item.id === ticket.parentId) ? (
              <option value={ticket.parent.id}>{ticket.parent.key} · {ticket.parent.title}</option>
            ) : null}
            {parents.map((item) => (
              <option key={item.id} value={item.id}>{item.key} · {item.title}</option>
            ))}
          </select>
          {parents.length === 0 && !ticket.parent ? <small className="field-hint">{t('work.parent.noneAvailable')}</small> : null}
          {kind === 'SUBTASK' ? <small className="field-hint">{t('work.parent.subtaskRequired')}</small> : null}
        </label>
      ) : null}

      {nextKind ? (
        <div className="children">
          <div className="children-head">
            <h4>{t(`work.children.title.${kind as 'EPIC' | 'TASK'}`)}</h4>
            {summary.total > 0 ? (
              <span className="children-progress" title={t('work.children.hiddenNote')}>
                <span className="progress-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progressPercent(summary)}>
                  <span style={{ width: `${String(progressPercent(summary))}%` }} />
                </span>
                {t('work.children.progress', { done: String(summary.done), total: String(summary.total) })}
                {summary.canceled > 0 ? ` · ${t('work.children.canceled', { count: String(summary.canceled) })}` : ''}
              </span>
            ) : null}
          </div>
          {children.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
          {children.isError ? <p className="hint" role="alert">{t('work.children.error')}</p> : null}
          {children.isSuccess && children.data.length === 0 ? <p className="hint">{t(`work.children.empty.${kind as 'EPIC' | 'TASK'}`)}</p> : null}
          {children.isSuccess && children.data.length > 0 ? (
            <div className="children-tools">
              <button type="button" className="more-button" aria-expanded={!collapsed} onClick={() => setCollapsed((value) => !value)}>
                {collapsed ? t('work.children.expand') : t('work.children.collapse')}
              </button>
              {children.data.some((child) => states.get(child.statusId)?.category === 'DONE') ? (
                <button type="button" className="more-button" aria-pressed={hideDone} onClick={() => setHideDone((value) => !value)}>
                  {hideDone ? t('work.children.showDone') : t('work.children.hideDone')}
                </button>
              ) : null}
            </div>
          ) : null}
          {collapsed ? null : children.isSuccess && children.data.length > 0 ? (
            <ul className="child-list">
              {(showAllChildren ? children.data : children.data.slice(0, 5)).filter((child) => !hideDone || states.get(child.statusId)?.category !== 'DONE').map((child) => {
                const state = states.get(child.statusId)
                const finished = state?.category === 'DONE'
                return (
                  <li key={child.id} className={state?.category === 'CANCELED' ? 'child-row canceled' : 'child-row'}>
                    {nextKind === 'SUBTASK' && canEdit ? (
                      <input
                        type="checkbox"
                        aria-label={t('work.children.toggleDone', { title: child.title })}
                        checked={finished}
                        disabled={togglingId !== null || busy}
                        onChange={() => void toggleDone(child)}
                      />
                    ) : null}
                    <button type="button" className="child-open" onClick={() => onOpen(child.id)}>
                      <small className="ticket-key">{child.key}</small>
                      <span className={finished ? 'child-title done' : 'child-title'}>{child.title}</span>
                    </button>
                    <span className="state-chip">{state?.name ?? ''}</span>
                    {child.assigneeName ? <span title={child.assigneeName}><Avatar name={child.assigneeName} /></span> : null}
                  </li>
                )
              })}
            </ul>
          ) : null}
          {!collapsed && children.isSuccess && children.data.length > 5 ? (
            <button type="button" className="more-button" aria-expanded={showAllChildren} onClick={() => setShowAllChildren((open) => !open)}>
              {showAllChildren ? t('work.children.less') : t('work.children.more', { count: String(children.data.length - 5) })}
            </button>
          ) : null}
          {canEdit ? (
            <Button
              type="button"
              variant="secondary"
              className="child-add-button"
              onClick={() =>
                openCreate({
                  projectId: ticket.projectId,
                  parentId: ticket.id,
                  kind: nextKind,
                  // A Subtask stays in its Task's channel; a Task under an Epic starts project-level.
                  channelId: kind === 'TASK' ? (ticket.channelId ?? undefined) : undefined,
                })
              }
            >
              + {t(`work.children.add.${kind as 'EPIC' | 'TASK'}`)}
            </Button>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}
