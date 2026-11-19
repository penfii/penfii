import { useEffect, useMemo, useRef } from 'react'
import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { Button } from '../../design/ui'
import { useIsMobile } from '../../design/useIsMobile'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { useNow } from '../../ui/useNow'
import { addDays, dayEntries, dayInZone, dueByDay, entriesOf, filterItems, middleOf, monthOf, monthWeeks, shiftMonth, unscheduled, weekPieces, weekStartFor, type Day, type Entry, type Filters } from './calendarModel'
import { useProjectLive } from './live'
import { useTicketPanel } from './panelParam'
import { Popover } from './Popover'
import { browserZone } from './zonedTime'

/** At most this many due Tickets are listed in a day; the rest are one "more" away, never cut off. */
const DAY_LIMIT = 3

export type CalendarState = { date: Day | null; plan: boolean }

/**
 * The month calendar of a Project's Timeline. A Ticket's due date is a flag on its day; its plan (start to the end the work
 * calendar derives) is a bar, drawn only when asked and always named "Plan", so the two meanings are never mixed. Dates are
 * the Project's days, not the browser's. Nothing here edits a date: a Ticket opens in its detail, where dates are changed.
 */
export function CalendarView({ workspaceId, projectId, filters, state, onState }: { workspaceId: string; projectId: string; filters: Filters; state: CalendarState; onState: (patch: Partial<CalendarState>) => void }) {
  const { t, locale } = useI18n()
  const mobile = useIsMobile()
  const { open: openPanel, selectedId } = useTicketPanel()
  const opener = useRef<string | null>(null)
  const previousPanel = useRef<string | null>(null)
  const today = useNow()
  const schedule = useQuery(queries.projectSchedule(projectId))
  useProjectLive(projectId)
  const data = schedule.data
  const zone = data?.calendar?.timeZone ?? browserZone()
  const todayDay = dayInZone(new Date(today).toISOString(), zone)
  // The shared day decides the month; with none, it is this month.
  const month = state.date ? monthOf(state.date) : monthOf(todayDay)
  const weekStart = weekStartFor(locale)
  const shown = useMemo(() => filterItems(data?.items ?? [], filters), [data?.items, filters])
  const entries = useMemo(() => entriesOf(shown, zone), [shown, zone])
  const weeks = useMemo(() => monthWeeks(month, weekStart), [month, weekStart])
  const pending = useMemo(() => unscheduled(shown, data?.calendar != null), [shown, data?.calendar])
  // What the filters hide: the same rule over everything this person may see, so "hidden by a filter" is told from "nothing is undated".
  const allPending = useMemo(() => unscheduled(data?.items ?? [], data?.calendar != null).length, [data?.items, data?.calendar])
  const due = useMemo(() => dueByDay(entries), [entries])

  /** Opens the Ticket and remembers where from, so closing the panel puts the focus back on the same entry. */
  function open(ticketId: string) {
    opener.current = ticketId
    openPanel(ticketId)
  }
  useEffect(() => {
    if (previousPanel.current && !selectedId && opener.current) {
      document.querySelector<HTMLElement>(`[data-cal-entry="${opener.current}"]`)?.focus()
    }
    previousPanel.current = selectedId
  }, [selectedId])

  if (schedule.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (schedule.isError || !data) {
    return (
      <div className="timeline-error" role="alert">
        <p className="board-error">{t('timeline.loadError')}</p>
        <Button type="button" variant="secondary" onClick={() => void schedule.refetch()}>{t('timeline.retry')}</Button>
      </div>
    )
  }

  const monthLabel = new Intl.DateTimeFormat(locale, { year: 'numeric', month: 'long', timeZone: 'UTC' }).format(new Date(`${month}-01T00:00:00Z`))
  const weekdayNames = weeks[0]?.map((day) => new Intl.DateTimeFormat(locale, { weekday: 'short', timeZone: 'UTC' }).format(new Date(`${day}T00:00:00Z`))) ?? []
  const inMonth = (day: Day) => monthOf(day) === month
  const firstDay = `${month}-01`
  const lastDay = addDays(`${shiftMonth(month, 1)}-01`, -1)
  const anyInMonth = entries.some((entry) => (entry.due !== null && inMonth(entry.due)) || (state.plan && entry.plan !== null && entry.plan.start <= lastDay && entry.plan.end >= firstDay))
  const zoneNote = data.calendar ? t('calendar.zone.project', { zone }) : t('calendar.zone.browser', { zone })
  const browser = browserZone()

  return (
    <div className="calendar-view">
      <div className="calendar-bar">
        <div className="calendar-nav" role="group" aria-label={t('calendar.nav')}>
          <Button type="button" variant="secondary" aria-label={t('calendar.prev')} onClick={() => onState({ date: middleOf(shiftMonth(month, -1)) })}>‹</Button>
          <h3 className="calendar-month" aria-live="polite">{monthLabel}</h3>
          <Button type="button" variant="secondary" aria-label={t('calendar.next')} onClick={() => onState({ date: middleOf(shiftMonth(month, 1)) })}>›</Button>
          <Button type="button" variant="secondary" onClick={() => onState({ date: null })} disabled={state.date === null}>{t('calendar.today')}</Button>
        </div>
        <label className="switch-line">
          <input type="checkbox" checked={state.plan} onChange={(event) => onState({ plan: event.target.checked })} />
          <span>{t('calendar.showPlan')}</span>
        </label>
      </div>
      <p className="hint calendar-zone">
        {zoneNote}
        {data.calendar && browser !== zone ? <> {t('calendar.zone.differs', { browser })}</> : null}
      </p>
      {!data.calendar ? <p className="hint" role="note">{t('calendar.noWorkCalendar')}</p> : null}
      {data.nextCursor ? <p className="hint" role="note">{t('timeline.more', { count: String(data.items.length) })}</p> : null}
      <Legend plan={state.plan} />

      {!anyInMonth ? <p className="hint" role="status">{t('calendar.monthEmpty')}</p> : null}

      {mobile ? (
        <Agenda weeks={weeks} entries={entries} due={due} month={month} plan={state.plan} todayDay={todayDay} onOpen={open} />
      ) : (
        // A month to read, not a date to pick: a native table gives the weekdays as column headers and each day its place in them,
        // and every event inside stays its own button reached with Tab.
        <table className="tlcalendar-table">
          <caption className="visually-hidden">{monthLabel}</caption>
          <thead>
            <tr>
              {weekdayNames.map((name) => (
                <th key={name} scope="col" className="tlcalendar-weekday">{name}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {weeks.map((week) => (
              <Week key={week[0]} week={week} entries={entries} due={due} inMonth={inMonth} plan={state.plan} todayDay={todayDay} onOpen={open} />
            ))}
          </tbody>
        </table>
      )}

      <section className="timeline-unplanned" aria-label={t('calendar.unscheduled')}>
        <h3>{t('calendar.unscheduled')}</h3>
        <p className="hint">{t('calendar.unscheduled.lead')}</p>
        {pending.length === 0 ? (
          <p className="hint" role="status">{allPending > 0 ? t('calendar.unscheduled.hidden', { count: String(allPending) }) : t('calendar.unscheduled.empty')}</p>
        ) : (
          <ul className="list-rows">
            {pending.map(({ item, reason }) => (
              <li key={item.ticket.id} className={['list-row', item.statusCategory === 'DONE' || item.statusCategory === 'CANCELED' ? 'is-done' : ''].filter(Boolean).join(' ')}>
                <button type="button" className="list-row-open" data-cal-entry={item.ticket.id} aria-label={t('work.board.openTicket', { key: item.ticket.key })} onClick={() => open(item.ticket.id)}>
                  <small className="ticket-key">{item.ticket.key}</small>
                  <strong className="list-row-title">{item.ticket.title}</strong>
                </button>
                <span className="list-row-meta">
                  <span className={`state-chip state-${item.statusCategory.toLowerCase()}`}>{t(`work.category.${item.statusCategory}`)}</span>
                  <span className="where-chip">{t(`timeline.unplanned.${reason}`)}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
        <Link className="calendar-work-link" to={`/w/${workspaceId}/projects/${projectId}/work`}>{t('calendar.unscheduled.work')}</Link>
      </section>
    </div>
  )
}

function Legend({ plan }: { plan: boolean }) {
  const { t } = useI18n()
  return (
    <ul className="gantt-legend calendar-legend" aria-label={t('legend.title')}>
      <li><span className="cal-flag" aria-hidden="true">⚑</span> {t('calendar.legend.due')}</li>
      {plan ? <li><span className="swatch swatch-plan" aria-hidden="true" /> {t('calendar.legend.plan')}</li> : null}
    </ul>
  )
}

function Week({ week, entries, due, inMonth, plan, todayDay, onOpen }: { week: Day[]; entries: Entry[]; due: Map<Day, Entry[]>; inMonth: (day: Day) => boolean; plan: boolean; todayDay: Day; onOpen: (id: string) => void }) {
  const { t, locale } = useI18n()
  const { pieces, lanes } = plan ? weekPieces(week, entries) : { pieces: [], lanes: 0 }
  return (
    <tr>
      {week.map((day, column) => {
        const list = due.get(day) ?? []
        const visible = list.slice(0, DAY_LIMIT)
        const rest = list.length - visible.length
        const date = new Date(`${day}T00:00:00Z`)
        const full = new Intl.DateTimeFormat(locale, { dateStyle: 'full', timeZone: 'UTC' }).format(date)
        return (
          <td
            key={day}
            aria-current={day === todayDay ? 'date' : undefined}
            className={['tlcalendar-day', inMonth(day) ? '' : 'is-outside', day === todayDay ? 'is-today' : ''].filter(Boolean).join(' ')}
          >
            <div className="tlcalendar-day-body">
              <span className="tlcalendar-date">
                <span aria-hidden="true">{date.getUTCDate()}</span>
                <span className="visually-hidden">{full}</span>
                {day === todayDay ? <small className="tlcalendar-today-tag"> {t('calendar.todayTag')}</small> : null}
              </span>
              {Array.from({ length: lanes }, (_, lane) => {
                const piece = pieces.find((candidate) => candidate.lane === lane && candidate.from <= column && candidate.to >= column)
                if (!piece) return <span key={lane} className="cal-lane empty" aria-hidden="true" />
                const first = column === piece.from
                const cls = ['cal-lane', 'cal-plan', `state-${piece.entry.category.toLowerCase()}`, piece.openLeft && first ? 'open-left' : '', column > piece.from ? 'joined-left' : '', column < piece.to ? 'joined-right' : '', piece.openRight && column === piece.to ? 'open-right' : ''].filter(Boolean).join(' ')
                return first ? (
                  <button key={lane} type="button" className={cls} data-cal-entry={piece.entry.id} aria-label={t('calendar.planEntry', { key: piece.entry.key, title: piece.entry.title, start: piece.entry.plan?.start ?? '', end: piece.entry.plan?.end ?? '' })} title={piece.entry.title} onClick={() => onOpen(piece.entry.id)}>
                    <span className="cal-kind">{t('calendar.plan')} · </span>
                    {piece.entry.key} {piece.entry.title}
                  </button>
                ) : (
                  <span key={lane} className={cls} aria-hidden="true" />
                )
              })}
              <ul className="cal-due-list">
                {visible.map((entry) => (
                  <li key={entry.id}>
                    <button type="button" className={['cal-due', `state-${entry.category.toLowerCase()}`].join(' ')} data-cal-entry={entry.id} title={entry.title} aria-label={t('calendar.dueEntry', { key: entry.key, title: entry.title })} onClick={() => onOpen(entry.id)}>
                      <span className="cal-flag" aria-hidden="true">⚑</span> {entry.key} {entry.title}
                    </button>
                  </li>
                ))}
              </ul>
              {rest > 0 ? (
                <Popover label={t('calendar.more', { count: String(rest) })} className="cal-more" wide>
                  {(close) => <DayList day={day} full={full} entries={entries} plan={plan} onOpen={(id) => { close(); onOpen(id) }} />}
                </Popover>
              ) : null}
            </div>
          </td>
        )
      })}
    </tr>
  )
}

/** Everything on one day, for the "more" panel: the Tickets due, then (when shown) the plans that cover the day. */
function DayList({ day, full, entries, plan, onOpen }: { day: Day; full: string; entries: Entry[]; plan: boolean; onOpen: (id: string) => void }) {
  const { t } = useI18n()
  const { due, plans } = dayEntries(day, entries, plan)
  return (
    <div className="cal-day-list">
      <strong>{full}</strong>
      <ul>
        {due.map((entry) => (
          <li key={`due-${entry.id}`}>
            <button type="button" className="cal-due" aria-label={t('calendar.dueEntry', { key: entry.key, title: entry.title })} onClick={() => onOpen(entry.id)}>
              <span className="cal-flag" aria-hidden="true">⚑</span> {entry.key} {entry.title}
            </button>
          </li>
        ))}
        {plans.map((entry) => (
          <li key={`plan-${entry.id}`}>
            <button type="button" className="cal-due cal-plan-line" aria-label={t('calendar.planEntry', { key: entry.key, title: entry.title, start: entry.plan?.start ?? '', end: entry.plan?.end ?? '' })} onClick={() => onOpen(entry.id)}>
              <span className="cal-kind">{t('calendar.plan')} · </span>{entry.key} {entry.title}
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}

/**
 * The phone's calendar: one list of the month's days that have something, in date order. A plan period is listed once, on
 * the day it starts (with its end), so a long plan is not repeated on every day.
 */
function Agenda({ weeks, entries, due, month, plan, todayDay, onOpen }: { weeks: Day[][]; entries: Entry[]; due: Map<Day, Entry[]>; month: string; plan: boolean; todayDay: Day; onOpen: (id: string) => void }) {
  const { t, locale } = useI18n()
  const days = weeks.flat().filter((day) => monthOf(day) === month)
  const rows = days
    .map((day) => {
      const dueHere = due.get(day) ?? []
      // A plan that began before the month is listed on the first day of the month.
      const plans = plan ? entries.filter((entry) => entry.plan && (entry.plan.start === day || (day === `${month}-01` && entry.plan.start < day && entry.plan.end >= day))) : []
      return { day, due: dueHere, plans }
    })
    .filter((row) => row.due.length > 0 || row.plans.length > 0)
  if (rows.length === 0) return null
  return (
    <ol className="calendar-agenda" aria-label={t('calendar.agenda')}>
      {rows.map(({ day, due, plans }) => {
        const date = new Date(`${day}T00:00:00Z`)
        const label = new Intl.DateTimeFormat(locale, { month: 'long', day: 'numeric', weekday: 'short', timeZone: 'UTC' }).format(date)
        return (
          <li key={day} className={day === todayDay ? 'is-today' : undefined}>
            <h4>{label}{day === todayDay ? <small> {t('calendar.todayTag')}</small> : null}</h4>
            <ul>
              {due.map((entry) => (
                <li key={`due-${entry.id}`}>
                  <button type="button" className="cal-due" data-cal-entry={entry.id} aria-label={t('calendar.dueEntry', { key: entry.key, title: entry.title })} onClick={() => onOpen(entry.id)}>
                    <span className="cal-flag" aria-hidden="true">⚑</span> {entry.key} {entry.title}
                  </button>
                </li>
              ))}
              {plans.map((entry) => (
                <li key={`plan-${entry.id}`}>
                  <button type="button" className="cal-due cal-plan-line" data-cal-entry={entry.id} aria-label={t('calendar.planEntry', { key: entry.key, title: entry.title, start: entry.plan?.start ?? '', end: entry.plan?.end ?? '' })} onClick={() => onOpen(entry.id)}>
                    <span className="cal-kind">{t('calendar.plan')} · </span>{entry.key} {entry.title}
                    <small className="hint"> {entry.plan?.start === entry.plan?.end ? '' : `${entry.plan?.start.slice(5) ?? ''} – ${entry.plan?.end.slice(5) ?? ''}`}</small>
                  </button>
                </li>
              ))}
            </ul>
          </li>
        )
      })}
    </ol>
  )
}

