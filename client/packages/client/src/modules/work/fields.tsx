import { useState, type ReactNode } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import type { ProjectMember, Ticket, TicketLabel, TicketType } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import type { ProjectContext } from './projectContext'
import { PRIORITIES, isPriority } from './ticketModel'

/*
 * Field editors shared by the Create Ticket panel and the Ticket detail panel, so both read and
 * write the same value shapes from the same option lists.
 */

export function PropRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="prop-row">
      <span className="prop-label">{label}</span>
      <div className="prop-value">{children}</div>
    </div>
  )
}

export function PriorityField({ value, onChange, disabled }: { value: string | null; onChange: (value: string | null) => void; disabled: boolean }) {
  const { t } = useI18n()
  return (
    <select aria-label={t('work.field.priority')} value={isPriority(value) ? value : ''} disabled={disabled} onChange={(event) => onChange(event.target.value || null)}>
      <option value="">{t('work.priority.none')}</option>
      {PRIORITIES.map((priority) => (
        <option key={priority} value={priority}>{t(`work.priority.${priority}`)}</option>
      ))}
    </select>
  )
}

export function AssigneeField({
  value,
  fallbackName,
  members,
  onChange,
  disabled,
}: {
  value: string | null
  fallbackName?: string | null
  members: ProjectMember[]
  onChange: (value: string | null) => void
  disabled: boolean
}) {
  const { t } = useI18n()
  const known = members.some((member) => member.userId === value)
  return (
    <select aria-label={t('work.field.assignee')} value={value ?? ''} disabled={disabled} onChange={(event) => onChange(event.target.value || null)}>
      <option value="">{t('work.assignee.none')}</option>
      {!known && value ? <option value={value}>{fallbackName ?? value}</option> : null}
      {members.map((member) => (
        <option key={member.userId} value={member.userId}>{member.displayName}</option>
      ))}
    </select>
  )
}

export function TypeField({ value, types, onChange, disabled }: { value: string | null; types: TicketType[]; onChange: (value: string | null) => void; disabled: boolean }) {
  const { t } = useI18n()
  return (
    <select aria-label={t('work.field.type')} value={value ?? ''} disabled={disabled} onChange={(event) => onChange(event.target.value || null)}>
      <option value="">{t('work.type.none')}</option>
      {types.map((type) => (
        <option key={type.id} value={type.id}>{type.name}</option>
      ))}
    </select>
  )
}

export function DueDateField({ value, onChange, disabled }: { value: string | null; onChange: (value: string | null) => void; disabled: boolean }) {
  const { t } = useI18n()
  return (
    <div className="prop-inline">
      <input type="date" aria-label={t('work.field.dueOn')} value={value ?? ''} disabled={disabled} onChange={(event) => onChange(event.target.value || null)} />
      {value && !disabled ? <Button type="button" variant="ghost" onClick={() => onChange(null)}>{t('work.due.clear')}</Button> : null}
    </div>
  )
}

export function LabelField({
  projectId,
  labels,
  selected,
  onChange,
  disabled,
  canCreate,
}: {
  projectId: string
  labels: TicketLabel[]
  selected: string[]
  onChange: (ids: string[]) => void
  disabled: boolean
  canCreate: boolean
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const create = useMutation({
    mutationFn: (value: string) => api.createProjectLabel(projectId, { name: value }),
    onSuccess: async () => {
      setName('')
      await queryClient.invalidateQueries({ queryKey: queryKeys.projectLabels(projectId) })
    },
  })
  const busy = create.isPending
  const chosen = new Set(selected)

  function add() {
    const value = name.trim()
    if (!value || busy) return
    create.mutate(value)
  }

  return (
    <div className="label-picker">
      {labels.length === 0 ? <span className="hint">{t('work.labels.empty')}</span> : null}
      {labels.map((label) => (
        <button
          key={label.id}
          type="button"
          className={chosen.has(label.id) ? 'label-toggle on' : 'label-toggle'}
          aria-pressed={chosen.has(label.id)}
          disabled={disabled}
          onClick={() => onChange(chosen.has(label.id) ? selected.filter((id) => id !== label.id) : [...selected, label.id])}
        >
          {label.name}
        </button>
      ))}
      {canCreate ? (
        <div className="label-create">
          <input
            aria-label={t('projects.labelName')}
            placeholder={t('projects.labelName')}
            value={name}
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault()
                add()
              }
            }}
          />
          <Button type="button" variant="secondary" disabled={!name.trim() || busy} onClick={add}>{t('projects.addLabel')}</Button>
        </div>
      ) : null}
    </div>
  )
}

export function ParentField({
  value,
  options,
  fallback,
  allowNone,
  onChange,
  disabled,
  label,
}: {
  value: string | null
  options: Ticket[]
  fallback?: { id: string; key: string; title: string } | null
  allowNone: boolean
  onChange: (value: string | null) => void
  disabled: boolean
  label: string
}) {
  const { t } = useI18n()
  return (
    <select aria-label={label} value={value ?? ''} disabled={disabled} onChange={(event) => onChange(event.target.value || null)}>
      {allowNone || !value ? <option value="">{t('work.parent.none')}</option> : null}
      {fallback && !options.some((item) => item.id === fallback.id) ? <option value={fallback.id}>{fallback.key} · {fallback.title}</option> : null}
      {options.map((item) => (
        <option key={item.id} value={item.id}>{item.key} · {item.title}</option>
      ))}
    </select>
  )
}

export function SprintField({
  ticket,
  ctx,
  disabled,
  onChange,
}: {
  ticket: Ticket
  ctx: ProjectContext
  disabled: boolean
  onChange: (sprintId: string | null) => void
}) {
  const { t } = useI18n()
  const open = (ctx.planning?.sprints ?? []).filter((sprint) => sprint.state === 'PLANNED' || sprint.state === 'ACTIVE')
  const current = ticket.sprint
  const assignable = ticket.hierarchyKind === 'TASK'
  return (
    <div className="sprint-field">
      <select
        aria-label={t('work.field.sprint')}
        value={current?.id ?? ''}
        disabled={disabled || !assignable}
        onChange={(event) => onChange(event.target.value || null)}
      >
        <option value="">{t('work.sprint.none')}</option>
        {current && !open.some((sprint) => sprint.id === current.id) ? <option value={current.id}>{current.name}</option> : null}
        {open.map((sprint) => (
          <option key={sprint.id} value={sprint.id}>{sprint.name}</option>
        ))}
      </select>
      {open.length === 0 && !current && assignable ? <small className="field-hint">{t('work.sprint.noneYet')}</small> : null}
      {ticket.hierarchyKind === 'SUBTASK' ? <small className="field-hint">{t('work.sprint.inherited')}</small> : null}
      {ticket.hierarchyKind === 'EPIC' ? <small className="field-hint">{t('work.sprint.epic')}</small> : null}
    </div>
  )
}

export function MilestoneField({
  ticket,
  ctx,
  disabled,
  onChange,
}: {
  ticket: Ticket
  ctx: ProjectContext
  disabled: boolean
  onChange: (ids: string[]) => void
}) {
  const { t } = useI18n()
  const linked = ticket.milestones.map((item) => item.id)
  const choices = [
    ...ticket.milestones,
    ...(ctx.planning?.milestones ?? []).filter((item) => item.state === 'OPEN' && !linked.includes(item.id)),
  ]
  if (choices.length === 0) return <span className="hint">{t('work.milestone.empty')}</span>
  return (
    <div className="label-picker">
      {choices.map((item) => (
        <button
          key={item.id}
          type="button"
          className={linked.includes(item.id) ? 'label-toggle on' : 'label-toggle'}
          aria-pressed={linked.includes(item.id)}
          disabled={disabled}
          onClick={() => onChange(linked.includes(item.id) ? linked.filter((id) => id !== item.id) : [...linked, item.id])}
        >
          {item.name}
        </button>
      ))}
    </div>
  )
}
