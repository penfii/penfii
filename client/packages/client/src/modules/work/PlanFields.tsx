import { useState } from 'react'
import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { EstimateUnit, HierarchyKind, ScheduleInput, Ticket, TicketEstimate } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { formatMinutes, isEstimateText, normalizeEstimateText, unitSuffix } from './planValues'
import { PropRow } from './fields'
import { workDayMinutes } from './ganttModel'
import { browserZone, formatZoned, instantToLocalInput, localInputToInstant } from './zonedTime'
import type { TicketPatch } from './useTicketEditor'

/**
 * The estimate of a Task, written in the Project's unit. Only a Task has one, and a Project that
 * does not estimate takes none; in both cases the reason is shown instead of an input that would
 * be refused.
 */
export function EstimateField({
  value,
  unit,
  kind,
  disabled,
  onChange,
}: {
  value: TicketEstimate | null
  unit: EstimateUnit
  kind: HierarchyKind
  disabled: boolean
  /** `null` clears the estimate. */
  onChange: (value: string | null) => void
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState(value && value.unit === unit ? value.value : '')
  const [invalid, setInvalid] = useState(false)
  const [retyping, setRetyping] = useState(false)
  // A value changed elsewhere replaces the field unless it has an edit of its own.
  const [lastSaved, setLastSaved] = useState(value?.value ?? '')
  const saved = value?.value ?? ''
  if (saved !== lastSaved) {
    setDraft((current) => (current === lastSaved ? saved : current))
    setLastSaved(saved)
  }

  if (kind !== 'TASK') return <small className="hint">{value ? `${value.value} ${unitSuffix(value.unit)}` : t('work.estimate.taskOnly')}</small>
  if (unit === 'NONE') return <small className="hint">{value ? `${value.value} ${unitSuffix(value.unit)} · ` : ''}{t('work.estimate.noUnit')}</small>

  // A value in the other unit is shown as it was written; it is never converted or shown in this unit.
  const otherUnit = value !== null && value.unit !== unit
  if (otherUnit && !retyping) {
    return (
      <div className="estimate-mismatch">
        <strong className="estimate-stored">{`${value.value} ${unitSuffix(value.unit)}`}</strong>
        <p className="hint">{t('work.estimate.otherUnit', { stored: t(`work.estimate.unit.${value.unit}`), unit: t(`work.estimate.unit.${unit}`) })}</p>
        {disabled ? null : (
          <div className="estimate-actions">
            <Button type="button" variant="secondary" onClick={() => { setDraft(''); setRetyping(true) }}>{t('work.estimate.retype', { unit: t(`work.estimate.unit.${unit}`) })}</Button>
            <Button type="button" variant="ghost" onClick={() => { setDraft(''); setInvalid(false); onChange(null) }}>{t('work.estimate.clear')}</Button>
          </div>
        )}
      </div>
    )
  }

  function commit() {
    const text = draft.trim()
    if (text === '') {
      setInvalid(false)
      if (value && !otherUnit) onChange(null)
      return
    }
    if (!isEstimateText(text)) {
      setInvalid(true)
      return
    }
    setInvalid(false)
    const normal = normalizeEstimateText(text)
    setDraft(normal)
    if (normal !== value?.value || otherUnit) {
      onChange(normal)
      setRetyping(false)
    }
  }

  return (
    <div className="prop-inline estimate-field">
      <input
        type="text"
        inputMode="decimal"
        aria-label={t('work.field.estimate')}
        aria-invalid={invalid}
        placeholder={t('work.estimate.placeholder')}
        value={draft}
        disabled={disabled}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
            event.preventDefault()
            commit()
          } else if (event.key === 'Escape') {
            setDraft(value?.value ?? '')
            setInvalid(false)
          }
        }}
      />
      <span className="estimate-unit" aria-hidden="true">{unitSuffix(unit)}</span>
      {value && !disabled && !otherUnit ? <Button type="button" variant="ghost" onClick={() => { setDraft(''); setInvalid(false); onChange(null) }}>{t('work.estimate.clear')}</Button> : null}
      {otherUnit ? <Button type="button" variant="ghost" onClick={() => { setDraft(''); setInvalid(false); setRetyping(false) }}>{t('work.schedule.cancel')}</Button> : null}
      {invalid ? <small className="field-error" role="alert">{t('work.estimate.invalid')}</small> : null}
    </div>
  )
}

/** The estimate row of the properties grid. */
export function EstimateRow({ ticket, unit, disabled, onSave }: { ticket: Ticket; unit: EstimateUnit; disabled: boolean; onSave: (patch: TicketPatch) => void }) {
  const { t } = useI18n()
  return (
    <PropRow label={t('work.field.estimate')}>
      <EstimateField
        value={ticket.estimate ?? null}
        unit={unit}
        kind={ticket.hierarchyKind}
        disabled={disabled}
        onChange={(value) => onSave({ estimate: value === null ? null : { value } })}
      />
    </PropRow>
  )
}

type DurationDraft = { value: string; unit: 'h' | 'min' }

function toDraft(minutes: number | null | undefined): DurationDraft {
  if (minutes === null || minutes === undefined) return { value: '', unit: 'h' }
  return minutes > 0 && minutes % 60 === 0 ? { value: String(minutes / 60), unit: 'h' } : { value: String(minutes), unit: 'min' }
}

/** Working minutes from what was typed; `undefined` when it cannot be a whole number of minutes. */
function toMinutes(draft: DurationDraft): number | null | undefined {
  const text = draft.value.trim()
  if (text === '') return null
  if (!/^\d+(\.\d+)?$/.test(text)) return undefined
  const minutes = draft.unit === 'h' ? Number(text) * 60 : Number(text)
  return Number.isInteger(minutes) ? minutes : undefined
}

/**
 * A Ticket's planned schedule: a start and working-minute durations. The planned end is not typed;
 * it is calculated by the server from the Project's work calendar and shown here. Times are read
 * and written on the calendar's wall clock, so "09:00" means the same thing to everyone.
 */
export function ScheduleSection({
  ticket,
  workspaceId,
  canManageProject,
  canEdit,
  busy,
  save,
}: {
  ticket: Ticket
  workspaceId: string
  canManageProject: boolean
  canEdit: boolean
  busy: boolean
  save: (patch: TicketPatch) => Promise<boolean>
}) {
  const { t, locale } = useI18n()
  const calendar = useQuery(queries.projectCalendar(ticket.projectId))
  const stored = calendar.data?.calendar ?? null
  const zone = stored?.timeZone ?? browserZone()
  const schedule = ticket.schedule ?? null
  const [editing, setEditing] = useState(false)
  const [start, setStart] = useState('')
  const [duration, setDuration] = useState<DurationDraft>({ value: '', unit: 'h' })
  const [milestone, setMilestone] = useState(false)
  const [invalid, setInvalid] = useState(false)
  const labels = { hour: t('work.schedule.hourShort'), minute: t('work.schedule.minuteShort') }

  function open() {
    setStart(schedule?.plannedStartAt ? instantToLocalInput(schedule.plannedStartAt, zone) : '')
    // A schedule milestone stores 0, which no other schedule may have: leave the field empty for the person who turns it off.
    setDuration(toDraft(schedule?.isScheduleMilestone ? null : schedule?.durationMinutes))
    setMilestone(schedule?.isScheduleMilestone ?? false)
    setInvalid(false)
    setEditing(true)
  }

  async function commit() {
    const plannedStartAt = start === '' ? null : localInputToInstant(start, zone)
    const durationMinutes = milestone ? 0 : toMinutes(duration)
    const bad = (start !== '' && plannedStartAt === null) || durationMinutes === undefined || (!milestone && durationMinutes === 0)
    if (bad || durationMinutes === undefined) {
      setInvalid(true)
      return
    }
    const input: ScheduleInput = { plannedStartAt, durationMinutes, isScheduleMilestone: milestone }
    const empty = plannedStartAt === null && durationMinutes === null && !milestone
    if (await save({ schedule: empty ? null : input })) setEditing(false)
  }

  const endText = (() => {
    if (!schedule || !calendar.isSuccess) return ''
    if (schedule.plannedEndAt) return formatZoned(schedule.plannedEndAt, zone, locale)
    if (!schedule.plannedStartAt || (schedule.durationMinutes === null && !schedule.isScheduleMilestone)) return t('work.schedule.endNeedsStart')
    return stored ? t('work.schedule.endBeyond') : t('work.schedule.endNoCalendar')
  })()

  return (
    <section className="ticket-section schedule" aria-label={t('work.schedule.title')}>
      <h3>{t('work.schedule.title')}</h3>
      <p className="hint">{t('work.schedule.lead')}</p>
      {editing ? (
        <form
          className="schedule-form"
          onSubmit={(event) => {
            event.preventDefault()
            void commit()
          }}
        >
          <div className="prop-grid">
            <PropRow label={t('work.schedule.start')}>
              <input type="datetime-local" aria-label={t('work.schedule.start')} value={start} onChange={(event) => setStart(event.target.value)} />
            </PropRow>
            <PropRow label={t('work.schedule.duration')}>
              <DurationInput label={t('work.schedule.duration')} draft={duration} disabled={milestone} onChange={setDuration} />
            </PropRow>
          </div>
          <label className="schedule-milestone">
            <input type="checkbox" checked={milestone} onChange={(event) => setMilestone(event.target.checked)} /> {t('work.schedule.milestone')}
          </label>
          <small className="hint">{t('work.schedule.milestoneHint')}</small>
          {stored ? <small className="hint">{t('work.schedule.dayHint', { day: formatMinutes(workDayMinutes(stored.definition), labels) })}</small> : null}
          <small className="hint">{stored ? t('work.schedule.zoneCalendar', { zone }) : t('work.schedule.zoneBrowser', { zone })}</small>
          {invalid ? <p className="board-error" role="alert">{t('work.schedule.invalid')}</p> : null}
          <div className="dialog-actions">
            {schedule ? <Button type="button" variant="ghost" disabled={busy} onClick={() => void save({ schedule: null }).then((ok) => ok && setEditing(false))}>{t('work.schedule.clear')}</Button> : null}
            <Button type="button" variant="secondary" disabled={busy} onClick={() => setEditing(false)}>{t('work.schedule.cancel')}</Button>
            <Button type="submit" disabled={busy}>{t('work.schedule.save')}</Button>
          </div>
        </form>
      ) : (
        <>
          {schedule ? (
            <dl className="schedule-view">
              <div><dt>{t('work.schedule.start')}</dt><dd>{schedule.plannedStartAt ? formatZoned(schedule.plannedStartAt, zone, locale) : t('work.value.empty')}</dd></div>
              <div>
                <dt>{t('work.schedule.duration')}</dt>
                <dd>
                  {schedule.isScheduleMilestone ? t('work.schedule.milestoneChip') : schedule.durationMinutes !== null ? formatMinutes(schedule.durationMinutes, labels) : t('work.value.empty')}
                </dd>
              </div>
              <div><dt>{t('work.schedule.end')}</dt><dd>{endText}</dd></div>
            </dl>
          ) : (
            <p className="hint">{t('work.schedule.none')}</p>
          )}
          {schedule && stored && zone !== browserZone() ? <p className="hint">{t('work.schedule.zoneDiffers', { zone, browser: browserZone() })}</p> : null}
          {schedule && !stored && calendar.isSuccess ? (
            <p className="hint">
              {t('work.schedule.zoneBrowser', { zone })}{' '}
              {canManageProject ? <Link to={`/w/${workspaceId}/projects/${ticket.projectId}/settings`}>{t('work.schedule.createCalendar')}</Link> : null}
            </p>
          ) : null}
          {/* Times are read on the calendar's wall clock, so the zone must be known before editing starts. */}
          {canEdit ? <Button type="button" variant="secondary" disabled={busy || calendar.isPending} onClick={open}>{schedule ? t('work.schedule.edit') : t('work.schedule.add')}</Button> : null}
        </>
      )}
    </section>
  )
}

function DurationInput({ label, draft, disabled, onChange }: { label: string; draft: DurationDraft; disabled: boolean; onChange: (draft: DurationDraft) => void }) {
  const { t } = useI18n()
  return (
    <div className="prop-inline">
      <input
        type="text"
        inputMode="decimal"
        aria-label={label}
        value={draft.value}
        disabled={disabled}
        onChange={(event) => onChange({ ...draft, value: event.target.value })}
      />
      <select aria-label={t('work.schedule.unitLabel', { field: label })} value={draft.unit} disabled={disabled} onChange={(event) => onChange({ ...draft, unit: event.target.value === 'min' ? 'min' : 'h' })}>
        <option value="h">{t('work.schedule.unit.hours')}</option>
        <option value="min">{t('work.schedule.unit.minutes')}</option>
      </select>
    </div>
  )
}
