import { useQuery } from '@tanstack/react-query'
import type { BurndownPoint, EstimateUnit, ReportAmount, Sprint, SprintReport, VelocityEntry } from '@penfii/contracts'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queries } from '../../queries'
import { useTicketPanel } from './panelParam'

type Translate = ReturnType<typeof useI18n>['t']

/** A sum in the Sprint's unit: `5 pt`, `7 h`, or a number of tasks. */
export function amountText(amount: ReportAmount, unit: EstimateUnit | undefined, t: Translate): string {
  return t(`report.unit.${unit ?? 'NONE'}`, { value: amount.value })
}

/** What an amount leaves out, so a total is never read as complete when it is not. */
function AmountNotes({ amount }: { amount: ReportAmount }) {
  const { t } = useI18n()
  const notes = [
    amount.unestimated > 0 ? t('report.note.unestimated', { count: String(amount.unestimated) }) : '',
    amount.otherUnit > 0 ? t('report.note.otherUnit', { count: String(amount.otherUnit) }) : '',
  ].filter(Boolean)
  return <small className="report-notes">{notes.length > 0 ? notes.join(' · ') : t('report.note.none')}</small>
}

const WIDTH = 480
const HEIGHT = 170
const PAD = 30

/** A line over the points: inline SVG, with the same numbers in a table beside it. */
function Line({ values, label }: { values: number[]; label: string }) {
  const top = Math.max(...values, 1)
  const x = (index: number) => PAD + ((WIDTH - 2 * PAD) * index) / Math.max(values.length - 1, 1)
  const y = (value: number) => HEIGHT - PAD - ((HEIGHT - 2 * PAD) * value) / top
  return (
    <svg className="report-chart" viewBox={`0 0 ${String(WIDTH)} ${String(HEIGHT)}`} role="img" aria-label={label}>
      <line x1={PAD} y1={HEIGHT - PAD} x2={WIDTH - PAD} y2={HEIGHT - PAD} className="axis" />
      <line x1={PAD} y1={PAD} x2={PAD} y2={HEIGHT - PAD} className="axis" />
      <text x={PAD - 4} y={PAD + 4} textAnchor="end" className="axis-label">{top}</text>
      <text x={PAD - 4} y={HEIGHT - PAD + 4} textAnchor="end" className="axis-label">0</text>
      <polyline className="series" fill="none" points={values.map((value, index) => `${String(x(index))},${String(y(value))}`).join(' ')} />
      {values.map((value, index) => (
        <circle key={index} className="dot" cx={x(index)} cy={y(value)} r={3} />
      ))}
    </svg>
  )
}

/** A day of the Sprint's own calendar, written the way the start stamp beside it is. */
function dayText(day: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeZone: 'UTC' }).format(new Date(`${day}T00:00:00Z`))
}

function pointLabel(point: BurndownPoint, t: Translate, locale: string): string {
  return point.day ? dayText(point.day, locale) : `${t('report.burndown.start')} · ${formatStamp(point.at, locale)}`
}

function Burndown({ report }: { report: SprintReport }) {
  const { t, locale } = useI18n()
  const unit = report.sprint.estimateUnit
  const points = report.burndown
  if (points.length === 0) return null
  const first = pointLabel(points[0], t, locale)
  const last = pointLabel(points[points.length - 1], t, locale)
  return (
    <section className="report-section" aria-label={t('report.burndown')}>
      <h5>{t('report.burndown')}</h5>
      <Line values={points.map((point) => Number(point.remaining.value))} label={t('report.burndown.alt', { start: first, end: last, days: String(Math.max(points.length - 1, 0)) })} />
      <table className="record-table report-table">
        <thead>
          <tr>
            <th scope="col">{t('report.burndown.when')}</th>
            <th scope="col">{t('report.burndown.remaining')}</th>
            <th scope="col">{t('report.burndown.tasks')}</th>
          </tr>
        </thead>
        <tbody>
          {points.map((point) => (
            <tr key={point.at}>
              <th scope="row">{pointLabel(point, t, locale)}</th>
              <td>{amountText(point.remaining, unit, t)}</td>
              <td>{point.remaining.tasks}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  )
}

function Changes({ report }: { report: SprintReport }) {
  const { t, locale } = useI18n()
  const { open } = useTicketPanel()
  const unit = report.sprint.estimateUnit
  const shown = (value: string | null, other: boolean) => (other ? t('report.tickets.otherUnit') : value === null ? '—' : t(`report.unit.${unit ?? 'NONE'}`, { value }))
  return (
    <section className="report-section" aria-label={t('report.changes')}>
      <h5>{t('report.changes')}</h5>
      {report.changes.length === 0 ? <p className="hint">{t('report.changes.empty')}</p> : (
        <table className="record-table report-table">
          <thead>
            <tr>
              <th scope="col">{t('report.change.when')}</th>
              <th scope="col">{t('report.change.kind')}</th>
              <th scope="col">{t('report.change.ticket')}</th>
              <th scope="col">{t('report.change.estimate')}</th>
            </tr>
          </thead>
          <tbody>
            {report.changes.map((change, index) => (
              <tr key={`${change.at}:${change.ticket.id}:${change.kind}:${String(index)}`}>
                <td>{formatStamp(change.at, locale)}</td>
                <td>{t(`report.change.${change.kind}`)}</td>
                <td>
                  <button type="button" className="table-open" aria-label={t('work.board.openTicket', { key: change.ticket.key })} onClick={() => open(change.ticket.id)}>
                    <small className="ticket-key">{change.ticket.key}</small> {change.ticket.title}
                  </button>
                </td>
                <td>
                  {change.kind === 'ESTIMATE_CHANGED'
                    ? `${shown(change.estimateFrom, false)} → ${shown(change.estimateTo, change.otherUnit)}`
                    : shown(change.kind === 'ADDED' ? change.estimateTo : change.estimateFrom, change.otherUnit)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  )
}

function TicketResults({ report }: { report: SprintReport }) {
  const { t } = useI18n()
  const { open } = useTicketPanel()
  const unit = report.sprint.estimateUnit
  const estimate = (value: string | null, other: boolean) => (other && value === null ? t('report.tickets.otherUnit') : value === null ? '—' : t(`report.unit.${unit ?? 'NONE'}`, { value }))
  return (
    <section className="report-section" aria-label={t('report.tickets')}>
      <h5>{t('report.tickets')}</h5>
      {report.tickets.length === 0 ? <p className="hint">{t('report.tickets.empty')}</p> : (
        <table className="record-table report-table">
          <thead>
            <tr>
              <th scope="col">{t('report.tickets.ticket')}</th>
              <th scope="col">{t('report.tickets.start')}</th>
              <th scope="col">{t('report.tickets.end')}</th>
              <th scope="col">{t('report.tickets.reason')}</th>
            </tr>
          </thead>
          <tbody>
            {report.tickets.map((row) => (
              <tr key={row.ticket.id} className={row.counted ? 'counted' : undefined}>
                <td>
                  <button type="button" className="table-open" aria-label={t('work.board.openTicket', { key: row.ticket.key })} onClick={() => open(row.ticket.id)}>
                    <small className="ticket-key">{row.ticket.key}</small> {row.ticket.title}
                  </button>
                  {!row.inBaseline ? <span className="where-chip report-added">{t('report.tickets.added')}</span> : null}
                </td>
                <td>{row.startCategory ? `${t(`work.category.${row.startCategory}`)} · ${estimate(row.startEstimate, row.otherUnit)}` : '—'}</td>
                <td>{row.endCategory ? `${t(`work.category.${row.endCategory}`)} · ${estimate(row.endEstimate, row.otherUnit)}` : '—'}</td>
                <td>{t(`report.reason.${row.reason}` as MessageKey)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  )
}

/**
 * The report of a running or finished Sprint: start plan, completed amount, plan adherence,
 * remaining work, Burndown with its table, scope changes and the result of each Ticket. A Sprint
 * that has no figures says why and shows nothing else. Everything is read from the server for the
 * person looking; the screen computes nothing.
 */
export function SprintReportPanel({ sprint }: { sprint: Sprint }) {
  const { t, locale } = useI18n()
  const report = useQuery(queries.sprintReport(sprint.id))
  if (report.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (report.isError) return <p className="board-error" role="alert">{t('report.loadError')}</p>
  const data = report.data
  if (!data.availability.available) {
    // A Sprint that never started has nothing to say; one that started before baselines did says why.
    if (data.availability.reason === 'NO_BASELINE' && sprint.state !== 'ACTIVE' && sprint.state !== 'COMPLETED') return null
    return (
      <section className="sprint-report" aria-label={t('report.title')}>
        <h4>{t('report.title')}</h4>
        <p className="hint" role="note">{t(`report.unavailable.${data.availability.reason ?? 'NO_BASELINE'}`)}</p>
      </section>
    )
  }
  const unit = data.sprint.estimateUnit
  const plan = data.startPlan
  const completed = data.completed
  const remaining = data.remaining
  const adherence = data.adherence
  return (
    <section className="sprint-report" aria-label={t('report.title')}>
      <h4>{t('report.title')}</h4>
      <p className="hint report-scope">
        {t('report.scope')}
        {data.provisional ? ` · ${t('report.provisional', { time: formatStamp(data.generatedAt, locale) })}` : ''}
      </p>
      {plan && completed && remaining && adherence ? (
        <dl className="report-figures">
          <div>
            <dt>{t('report.figure.plan')}</dt>
            <dd>{amountText(plan, unit, t)}</dd>
            <AmountNotes amount={plan} />
          </div>
          <div>
            <dt>{t('report.figure.completed')}</dt>
            <dd>{amountText(completed, unit, t)}</dd>
            <AmountNotes amount={completed} />
          </div>
          <div>
            <dt>{t('report.figure.adherence')}</dt>
            <dd>{adherence.notApplicable || adherence.percent === null ? t('report.adherence.na') : `${adherence.percent}%`}</dd>
            {adherence.notApplicable ? null : <small className="report-notes">{t('report.adherence.detail', { done: amountText(adherence.completedFromPlan, unit, t), plan: amountText(plan, unit, t) })}</small>}
          </div>
          <div>
            <dt>{t('report.figure.remaining')}</dt>
            <dd>{amountText(remaining, unit, t)}</dd>
            <small className="report-notes">{t('report.tasks', { count: String(remaining.tasks) })}</small>
          </div>
        </dl>
      ) : null}
      <Burndown report={data} />
      <Changes report={data} />
      <TicketResults report={data} />
    </section>
  )
}

function VelocityBars({ entries, label }: { entries: VelocityEntry[]; label: string }) {
  const values = entries.map((entry) => Number(entry.completed?.value ?? '0'))
  const top = Math.max(...values, 1)
  const slot = (WIDTH - 2 * PAD) / Math.max(values.length, 1)
  return (
    <svg className="report-chart" viewBox={`0 0 ${String(WIDTH)} ${String(HEIGHT)}`} role="img" aria-label={label}>
      <line x1={PAD} y1={HEIGHT - PAD} x2={WIDTH - PAD} y2={HEIGHT - PAD} className="axis" />
      <text x={PAD - 4} y={PAD + 4} textAnchor="end" className="axis-label">{top}</text>
      <text x={PAD - 4} y={HEIGHT - PAD + 4} textAnchor="end" className="axis-label">0</text>
      {values.map((value, index) => {
        const height = ((HEIGHT - 2 * PAD) * value) / top
        const center = PAD + slot * index + slot / 2
        const name = entries[index].sprint.name
        return (
          <g key={entries[index].sprint.id}>
            <rect className="bar" x={center - Math.min(slot * 0.7, 56) / 2} y={HEIGHT - PAD - height} width={Math.min(slot * 0.7, 56)} height={height} />
            <text x={center} y={HEIGHT - PAD - height - 4} textAnchor="middle" className="axis-label">{value}</text>
            <text x={center} y={HEIGHT - PAD + 12} textAnchor="middle" className="axis-label">{name.length > 12 ? `${name.slice(0, 11)}…` : name}</text>
          </g>
        )
      })}
    </svg>
  )
}

/** Mean of the included completed amounts, one decimal. Null when nothing is included. */
export function velocityAverage(entries: VelocityEntry[]): number | null {
  const values = entries.filter((entry) => entry.included && entry.completed).map((entry) => Number(entry.completed?.value)).filter((value) => Number.isFinite(value))
  if (values.length === 0) return null
  return Math.round((values.reduce((sum, value) => sum + value, 0) / values.length) * 10) / 10
}

/** One line on an active Sprint: the report's start, done, left, and how many scope changes. */
export function SprintSummaryStrip({ sprint }: { sprint: Sprint }) {
  const { t } = useI18n()
  const report = useQuery(queries.sprintReport(sprint.id))
  const data = report.data
  if (!data?.availability.available || !data.startPlan || !data.completed || !data.remaining) return null
  const unit = data.sprint.estimateUnit
  return (
    <p className="sprint-strip" role="status">
      {t('planning.sprint.strip', {
        plan: amountText(data.startPlan, unit, t),
        done: amountText(data.completed, unit, t),
        remaining: amountText(data.remaining, unit, t),
        changes: String(data.changes.length),
      })}
    </p>
  )
}

/** The completed amount of the last completed Sprints that share a unit; the others are listed apart. */
export function VelocityPanel({ projectId }: { projectId: string }) {
  const { t } = useI18n()
  const velocity = useQuery(queries.projectVelocity(projectId))
  if (velocity.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (velocity.isError) return <p className="board-error" role="alert">{t('velocity.loadError')}</p>
  const data = velocity.data
  const included = data.sprints.filter((entry) => entry.included).reverse()
  const left = data.sprints.filter((entry) => !entry.included)
  return (
    <section className="velocity" aria-label={t('velocity.title')}>
      <h3>{t('velocity.title')}</h3>
      {data.sprints.length === 0 ? <p className="hint">{t('velocity.empty')}</p> : null}
      {data.sprints.length > 0 && included.length === 0 ? <p className="hint">{t('velocity.noneComparable')}</p> : null}
      {included.length > 0 && data.unit ? (
        <>
          <p className="hint">{t('velocity.lead', { unit: t(`velocity.unit.${data.unit}`) })} · {t('velocity.average', { value: amountText({ value: String(velocityAverage(included) ?? 0), tasks: included.length, unestimated: 0, otherUnit: 0 }, data.unit, t) })} · {t('report.scope')}</p>
          <VelocityBars entries={included} label={t('velocity.chart', { count: String(included.length) })} />
          <table className="record-table report-table">
            <thead>
              <tr>
                <th scope="col">{t('velocity.sprint')}</th>
                <th scope="col">{t('velocity.completed')}</th>
              </tr>
            </thead>
            <tbody>
              {included.map((entry) => (
                <tr key={entry.sprint.id}>
                  <th scope="row">{entry.sprint.name}</th>
                  <td>
                    {entry.completed ? amountText(entry.completed, data.unit ?? undefined, t) : '—'}
                    {entry.completed ? <AmountNotes amount={entry.completed} /> : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      ) : null}
      {left.length > 0 ? (
        <>
          <h4>{t('velocity.excluded')}</h4>
          <ul className="velocity-left">
            {left.map((entry) => (
              <li key={entry.sprint.id}>
                {entry.sprint.name} · {entry.reason ? t(`velocity.reason.${entry.reason}`, { unit: t(`velocity.unit.${entry.sprint.estimateUnit ?? 'NONE'}`) }) : ''}
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </section>
  )
}

/** The few numbers that tell where a Project stands, read live for the Tickets the person can see. */
export function StatusSummary({ projectId }: { projectId: string }) {
  const { t, locale } = useI18n()
  const summary = useQuery(queries.projectStatusSummary(projectId))
  if (summary.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (summary.isError) return <p className="board-error" role="alert">{t('status.loadError')}</p>
  const { distribution, blocked, dueSoon, today, dueWithinDays } = summary.data
  return (
    <section className="status-summary" aria-label={t('status.title')}>
      <h2>{t('status.title')}</h2>
      <div className="metric-row">
        <article className="metric">
          <span>{t('status.distribution')}</span>
          <strong>{t('status.total', { count: String(distribution.total) })}</strong>
          <small>
            {[
              t('status.notStarted', { count: String(distribution.notStarted) }),
              t('status.inProgress', { count: String(distribution.inProgress) }),
              t('status.done', { count: String(distribution.done) }),
              t('status.canceled', { count: String(distribution.canceled) }),
            ].join(' · ')}
          </small>
        </article>
        <article className="metric" title={t('status.blockedHelp')}>
          <span>{t('status.blocked')}</span>
          <strong>{blocked}</strong>
          <small>{t('status.blockedHelp')}</small>
        </article>
        <article className="metric" title={t('status.dueSoonHelp', { today, days: String(dueWithinDays) })}>
          <span>{t('status.dueSoon')}</span>
          <strong>{dueSoon}</strong>
          <small>{t('status.dueSoonHelp', { today, days: String(dueWithinDays) })}</small>
        </article>
      </div>
      <p className="hint">{t('status.scope', { time: formatStamp(summary.data.generatedAt, locale) })}</p>
    </section>
  )
}
