import { useState, type FormEvent } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { CalendarDefinition, CalendarHoliday, CalendarInterval, CalendarWeekday, EstimateUnit, ProjectCalendarView } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { errorCode } from './ticketModel'
import { timeZones } from './zonedTime'
import { CloseIcon } from '../../design/icons'

const UNITS: EstimateUnit[] = ['NONE', 'POINTS', 'HOURS']
const DAYS: CalendarWeekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']

/**
 * The Project's estimate unit. Task estimates are written in it. Changing it never converts what
 * is already written, and while a Sprint is active the server refuses every change except the
 * first choice of a unit.
 */
export function EstimateUnitSection({ projectId, unit }: { projectId: string; unit: EstimateUnit }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [message, setMessage] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null)
  const save = useMutation({
    mutationFn: (next: EstimateUnit) => api.setProjectEstimateUnit(projectId, next),
    onSuccess: () => setMessage({ tone: 'ok', text: t('projectSettings.estimate.saved') }),
    onError: (cause) => setMessage({ tone: 'error', text: errorCode(cause) === 'estimate_unit_locked' ? t('projectSettings.estimate.locked') : t('projectSettings.estimate.error') }),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
      await queryClient.invalidateQueries({ queryKey: queryKeys.projectPlanning(projectId) })
    },
  })

  function change(next: EstimateUnit) {
    if (next === unit) return
    setMessage(null)
    save.mutate(next)
  }

  return (
    <section className="plan-settings" aria-labelledby="estimate-unit-title">
      <h3 id="estimate-unit-title">{t('projectSettings.estimate.title')}</h3>
      <p>{t('projectSettings.estimate.help')}</p>
      <p className="hint">{t('projectSettings.estimate.rule')}</p>
      <select aria-label={t('projectSettings.estimate.title')} value={unit} disabled={save.isPending} onChange={(event) => change(UNITS.find((item) => item === event.target.value) ?? unit)}>
        {UNITS.map((item) => (
          <option key={item} value={item}>{t(`work.estimate.unit.${item}`)}</option>
        ))}
      </select>
      {message ? <p className={message.tone === 'error' ? 'board-error' : 'hint'} role={message.tone === 'error' ? 'alert' : 'status'}>{message.text}</p> : null}
    </section>
  )
}

type Draft = { zone: string; week: Record<CalendarWeekday, CalendarInterval[]>; holidays: CalendarHoliday[] }

function draftFrom(zone: string, definition: CalendarDefinition): Draft {
  const week = Object.fromEntries(DAYS.map((day) => [day, (definition.week[day] ?? []).map((interval) => ({ ...interval }))])) as Draft['week']
  return { zone, week, holidays: definition.holidays.map((holiday) => ({ ...holiday })) }
}

function definitionFrom(draft: Draft): CalendarDefinition {
  const week: CalendarDefinition['week'] = {}
  for (const day of DAYS) if (draft.week[day].length > 0) week[day] = draft.week[day]
  return { version: 1, week, holidays: draft.holidays }
}

/**
 * The Project's work calendar: weekly working hours and whole-day holidays in one time zone. It is
 * created on purpose: with none, the form shows a proposal that does nothing until it is saved.
 * Saving needs the version that was read, so a calendar changed by someone else is never overwritten.
 */
export function CalendarSection({ projectId }: { projectId: string }) {
  const { t } = useI18n()
  const calendar = useQuery(queries.projectCalendar(projectId))
  if (calendar.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (calendar.isError || !calendar.data) return <p className="board-error" role="alert">{t('projectSettings.calendar.loadError')}</p>
  const view = calendar.data
  // The form restarts whenever the stored calendar changes, so it always edits the version it names.
  return <CalendarForm key={view.calendar?.version ?? 0} projectId={projectId} view={view} />
}

function CalendarForm({ projectId, view }: { projectId: string; view: ProjectCalendarView }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const stored = view.calendar
  const source = stored ? { zone: stored.timeZone, definition: stored.definition } : { zone: view.suggestion?.timeZone ?? 'UTC', definition: view.suggestion?.definition ?? { version: 1 as const, week: {}, holidays: [] } }
  const [draft, setDraft] = useState<Draft>(() => draftFrom(source.zone, source.definition))
  const [holidayDate, setHolidayDate] = useState('')
  const [holidayName, setHolidayName] = useState('')
  const [message, setMessage] = useState<{ tone: 'ok' | 'error'; text: string } | null>(null)
  const manage = view.canManage
  const known = timeZones()
  const dirty = JSON.stringify(draft) !== JSON.stringify(draftFrom(source.zone, source.definition))
  const zoneUnknown = draft.zone.trim() !== '' && known.length > 0 && !known.includes(draft.zone.trim())

  function setInterval(day: CalendarWeekday, index: number, change: Partial<CalendarInterval>) {
    setDraft((current) => ({ ...current, week: { ...current.week, [day]: current.week[day].map((item, at) => (at === index ? { ...item, ...change } : item)) } }))
  }

  const put = useMutation({
    mutationFn: () => api.putProjectCalendar(projectId, { expectedVersion: stored?.version ?? 0, timeZone: draft.zone.trim(), definition: definitionFrom(draft) }),
    onSuccess: () => setMessage({ tone: 'ok', text: t('projectSettings.calendar.saved') }),
    onError: (cause) => {
      const code = errorCode(cause)
      setMessage({
        tone: 'error',
        text: code === 'calendar_version_conflict' ? t('projectSettings.calendar.conflict') : code === 'validation' || code === 'validation_failed' ? t('projectSettings.calendar.invalid') : t('projectSettings.calendar.error'),
      })
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.projectCalendar(projectId) }),
  })
  const busy = put.isPending

  function save(event: FormEvent) {
    event.preventDefault()
    setMessage(null)
    put.mutate()
  }

  function addHoliday() {
    if (!holidayDate || !holidayName.trim()) return
    setDraft((current) => ({ ...current, holidays: [...current.holidays.filter((item) => item.date !== holidayDate), { date: holidayDate, name: holidayName.trim() }].sort((a, b) => a.date.localeCompare(b.date)) }))
    setHolidayDate('')
    setHolidayName('')
  }

  return (
    <section className="plan-settings calendar-settings" aria-labelledby="calendar-title">
      <h3 id="calendar-title">{t('projectSettings.calendar.title')}</h3>
      <p>{t('projectSettings.calendar.help')}</p>
      {stored ? <small className="hint">{t('projectSettings.calendar.version', { version: String(stored.version) })}</small> : <p className="hint">{t('projectSettings.calendar.missing')}</p>}
      <form onSubmit={save}>
        <label className="calendar-zone">
          <span>{t('projectSettings.calendar.zone')}</span>
          <input aria-label={t('projectSettings.calendar.zone')} list="calendar-time-zones" value={draft.zone} disabled={!manage || busy} onChange={(event) => setDraft({ ...draft, zone: event.target.value })} />
          <datalist id="calendar-time-zones">{known.map((name) => <option key={name} value={name} />)}</datalist>
          <small className="hint">{zoneUnknown ? t('projectSettings.calendar.zoneUnknown') : t('projectSettings.calendar.zoneHint')}</small>
        </label>
        <fieldset className="calendar-week">
          <legend>{t('projectSettings.calendar.week')}</legend>
          {DAYS.map((day) => {
            const label = t(`projectSettings.calendar.day.${day}`)
            return (
              <div key={day} className="calendar-day">
                <strong>{label}</strong>
                {draft.week[day].length === 0 ? <small className="hint">{t('projectSettings.calendar.dayOff')}</small> : null}
                {draft.week[day].map((interval, index) => (
                  <span key={index} className="calendar-interval">
                    <input aria-label={`${label} ${t('projectSettings.calendar.from')}`} value={interval.from} placeholder="09:00" size={5} maxLength={5} disabled={!manage || busy} onChange={(event) => setInterval(day, index, { from: event.target.value })} />
                    –
                    <input aria-label={`${label} ${t('projectSettings.calendar.to')}`} value={interval.to} placeholder="18:00" size={5} maxLength={5} disabled={!manage || busy} onChange={(event) => setInterval(day, index, { to: event.target.value })} />
                    {manage ? (
                      <button type="button" className="icon-button" aria-label={t('projectSettings.calendar.removeInterval', { day: label })} disabled={busy} onClick={() => setDraft((current) => ({ ...current, week: { ...current.week, [day]: current.week[day].filter((_, at) => at !== index) } }))}><CloseIcon /></button>
                    ) : null}
                  </span>
                ))}
                {manage ? (
                  <Button type="button" variant="ghost" disabled={busy} onClick={() => setDraft((current) => ({ ...current, week: { ...current.week, [day]: [...current.week[day], { from: '09:00', to: '18:00' }] } }))}>
                    {t('projectSettings.calendar.addInterval', { day: label })}
                  </Button>
                ) : null}
              </div>
            )
          })}
        </fieldset>
        <fieldset className="calendar-holidays">
          <legend>{t('projectSettings.calendar.holidays')}</legend>
          {draft.holidays.length === 0 ? <small className="hint">{t('projectSettings.calendar.noHolidays')}</small> : null}
          <ul>
            {draft.holidays.map((holiday) => (
              <li key={holiday.date}>
                <time dateTime={holiday.date}>{holiday.date}</time> {holiday.name}
                {manage ? (
                  <button type="button" className="icon-button" aria-label={t('projectSettings.calendar.removeHoliday', { name: holiday.name })} disabled={busy} onClick={() => setDraft((current) => ({ ...current, holidays: current.holidays.filter((item) => item.date !== holiday.date) }))}><CloseIcon /></button>
                ) : null}
              </li>
            ))}
          </ul>
          {manage ? (
            <div className="prop-inline">
              <input type="date" aria-label={t('projectSettings.calendar.holidayDate')} value={holidayDate} disabled={busy} onChange={(event) => setHolidayDate(event.target.value)} />
              <input aria-label={t('projectSettings.calendar.holidayName')} placeholder={t('projectSettings.calendar.holidayName')} maxLength={80} value={holidayName} disabled={busy} onChange={(event) => setHolidayName(event.target.value)} />
              <Button type="button" variant="secondary" disabled={busy || !holidayDate || !holidayName.trim()} aria-describedby="holiday-needs" onClick={addHoliday}>{t('projectSettings.calendar.addHoliday')}</Button>
            </div>
          ) : null}
          {manage && (!holidayDate || !holidayName.trim()) ? <small className="hint" id="holiday-needs">{t('projectSettings.calendar.holidayNeeds')}</small> : null}
        </fieldset>
        {manage && dirty && !message ? <p className="hint" role="status">{t('projectSettings.calendar.unsaved')}</p> : null}
        {message ? <p className={message.tone === 'error' ? 'board-error' : 'hint'} role={message.tone === 'error' ? 'alert' : 'status'}>{message.text}</p> : null}
        {manage ? <Button type="submit" disabled={busy}>{stored ? t('projectSettings.calendar.save') : t('projectSettings.calendar.create')}</Button> : null}
      </form>
    </section>
  )
}
