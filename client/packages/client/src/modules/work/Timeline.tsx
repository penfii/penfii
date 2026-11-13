import { useProjectLive } from './live'
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { Link } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { buildAxis, edgePath, ganttRows, nonWorkingSpans, planBar, planConflict, planConflicts, resizeDuration, scheduleBody, shiftStart, unplannedReason, workDayMinutes, type GanttRow } from './ganttModel'
import { useTicketPanel } from './panelParam'
import { useNow } from '../../ui/useNow'
import { formatMinutes } from './planValues'
import { errorCode } from './ticketModel'
import { browserZone, formatZoned } from './zonedTime'

const ROW_FINE = 30
/** A finger needs more room than a pointer: rows are taller where the pointer is coarse. */
const ROW_TOUCH = 40
const HEAD = 34

/** What the marks of the chart mean; only the ones this chart can show are listed. */
function Legend({ calendar, conflicts }: { calendar: boolean; conflicts: boolean }) {
  const { t } = useI18n()
  const items: Array<[string, string]> = [
    ['plan', t('legend.plan')],
    ['summary', t('legend.summary')],
    ['milestone', t('legend.milestone')],
    ['dependency', t('legend.dependency')],
  ]
  if (conflicts) items.push(['conflict', t('legend.conflict')])
  items.push(['today', t('legend.today')])
  if (calendar) items.push(['off', t('legend.off')])
  return (
    <ul className="gantt-legend" aria-label={t('legend.title')}>
      {items.map(([kind, label]) => (
        <li key={kind}>
          <span className={`swatch swatch-${kind}`} aria-hidden="true" />
          {label}
        </li>
      ))}
    </ul>
  )
}

/**
 * The Timeline of a Project: its Tickets as bars on a day axis in the work calendar's zone, the
 * blocking dependencies as arrows and an unplanned list. Bars come from planned schedules and
 * their derived ends.
 */
export function TimelinePage({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const { open: openPanel, selectedId } = useTicketPanel()
  const opener = useRef<string | null>(null)
  const previousPanel = useRef<string | null>(null)
  const scrolled = useRef(false)
  const today = useNow()
  const [coarse] = useState(() => typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches)
  const ROW = coarse ? ROW_TOUCH : ROW_FINE
  const scrollRef = useRef<HTMLDivElement>(null)
  const schedule = useQuery(queries.projectSchedule(projectId))
  useProjectLive(projectId)
  const calendar = useQuery(queries.projectCalendar(projectId))
  const [scale, setScale] = useState<'day' | 'week' | 'month'>('week')
  const planning = useQuery(queries.projectPlanning(projectId))
  const dayWidth = scale === 'day' ? 36 : scale === 'month' ? 8 : 16
  const [live, setLive] = useState('')
  // A bar being dragged: body moves the start, the right handle changes the duration, in whole days.
  const [drag, setDrag] = useState<{ id: string; kind: 'move' | 'resize'; startX: number; days: number; moved: boolean } | null>(null)

  const data = schedule.data
  const zone = data?.calendar?.timeZone ?? browserZone()
  const rows = useMemo(() => ganttRows(data?.items ?? []), [data?.items])
  const dayMinutes = workDayMinutes(calendar.data?.calendar?.definition)

  const axis = useMemo(() => {
    const dates: string[] = []
    for (const row of rows) {
      const bar = planBar(row.item)
      if (bar) dates.push(bar.start.toISOString(), bar.end.toISOString())
    }
    for (const sprint of planning.data?.sprints ?? []) dates.push(`${sprint.startsOn}T00:00:00Z`, `${sprint.endsOn}T00:00:00Z`)
    for (const milestone of planning.data?.milestones ?? []) if (milestone.targetOn && milestone.state !== 'CANCELED') dates.push(`${milestone.targetOn}T00:00:00Z`)
    dates.push(new Date(today).toISOString())
    return buildAxis(dates, zone, dayWidth)
  }, [rows, zone, planning.data, dayWidth, today])

  /** Opens the Ticket and remembers where from, so closing the panel puts the focus back there. */
  function open(ticketId: string) {
    opener.current = ticketId
    openPanel(ticketId)
  }

  useEffect(() => {
    if (previousPanel.current && !selectedId && opener.current) {
      const id = opener.current
      const target = document.querySelector<HTMLElement | SVGElement>(`[data-gantt-bar="${id}"]`) ?? document.querySelector<HTMLElement>(`[data-gantt-label="${id}"]`)
      target?.focus()
    }
    previousPanel.current = selectedId
  }, [selectedId])

  // A narrow screen starts at today, not at the left edge of a long axis.
  useEffect(() => {
    if (scrolled.current || !axis || !scrollRef.current) return
    scrolled.current = true
    scrollRef.current.scrollLeft = Math.max(0, axis.x(today) - 48)
  }, [axis, schedule.isSuccess, today])

  if (schedule.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (schedule.isError || !data) {
    return (
      <div className="timeline-error" role="alert">
        <p className="board-error">{t('timeline.loadError')}</p>
        <Button type="button" variant="secondary" onClick={() => void schedule.refetch()}>{t('timeline.retry')}</Button>
      </div>
    )
  }

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectSchedule(projectId) })
  }

  async function change(row: GanttRow, kind: 'move' | 'resize', direction: number) {
    const current = row.item.schedule
    const ticket = row.item.ticket
    if (!data?.canEdit) {
      setLive(t('timeline.readOnly'))
      return
    }
    if (!current) return
    const next = kind === 'move' ? shiftStart(current, direction, zone) : resizeDuration(current, direction, dayMinutes)
    if (!next) return
    try {
      await api.updateTicket(ticket.id, { expectedRevision: row.item.revisionNo, schedule: scheduleBody(next) }, newIdempotencyKey())
      await refresh()
      const labels = { hour: t('work.schedule.hourShort'), minute: t('work.schedule.minuteShort') }
      setLive(
        kind === 'move'
          ? t('timeline.moved', { key: ticket.key, start: next.plannedStartAt ? formatZoned(next.plannedStartAt, zone, locale) : '' })
          : t('timeline.resized', { key: ticket.key, duration: formatMinutes(next.durationMinutes ?? 0, labels) }),
      )
    } catch (cause) {
      const code = errorCode(cause)
      setLive(code === 'version_conflict' ? t('timeline.conflict') : code === 'validation' ? t('timeline.invalid') : t('timeline.saveError'))
      await refresh()
    }
  }

  function onBarKey(event: KeyboardEvent<SVGGElement>, row: GanttRow) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      open(row.item.ticket.id)
      return
    }
    if (!event.altKey || (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight')) return
    event.preventDefault()
    void change(row, event.shiftKey ? 'resize' : 'move', event.key === 'ArrowRight' ? 1 : -1)
  }

  function beginDrag(event: ReactPointerEvent<SVGElement>, row: GanttRow, kind: 'move' | 'resize') {
    if (!data?.canEdit || !row.item.schedule || event.button !== 0) return
    event.currentTarget.setPointerCapture(event.pointerId)
    event.stopPropagation()
    setDrag({ id: row.item.ticket.id, kind, startX: event.clientX, days: 0, moved: false })
  }

  function moveDrag(event: ReactPointerEvent<SVGElement>, row: GanttRow) {
    if (!drag || drag.id !== row.item.ticket.id || !axis) return
    const raw = Math.round((event.clientX - drag.startX) / axis.dayWidth)
    let days = raw
    // A start never comes to rest on a day off: keep going the way the pointer moves until a working day.
    const current = row.item.schedule
    if (drag.kind === 'move' && current?.plannedStartAt && off.length > 0) {
      const base = shiftStart(current, 0, zone)
      for (let guard = 0; guard < 14 && base; guard += 1) {
        const target = shiftStart(current, days, zone)
        const x = target?.plannedStartAt ? axis.x(target.plannedStartAt) : null
        if (x === null || !off.some((span) => x >= span.x && x < span.x + span.width) || raw === 0) break
        days += raw > 0 ? 1 : -1
      }
    }
    const moved = drag.moved || Math.abs(event.clientX - drag.startX) > 3
    if (days !== drag.days || moved !== drag.moved) setDrag({ ...drag, days, moved })
  }

  function endDrag(event: ReactPointerEvent<SVGElement>, row: GanttRow) {
    if (!drag || drag.id !== row.item.ticket.id) return
    event.currentTarget.releasePointerCapture(event.pointerId)
    const finished = drag
    setDrag(null)
    if (finished.moved && finished.days !== 0) void change(row, finished.kind, finished.days)
  }

  const labels = { hour: t('work.schedule.hourShort'), minute: t('work.schedule.minuteShort') }
  const index = new Map(rows.map((row, at) => [row.item.ticket.id, at]))
  const unplanned = rows.map((row) => ({ row, reason: unplannedReason(row.item, data.calendar !== null) })).filter((entry) => entry.reason !== null)
  const height = HEAD + rows.length * ROW + 8
  const drawn = rows.some((row) => planBar(row.item))
  const conflicts = planConflicts(data.edges, rows)
  const off = axis ? nonWorkingSpans(axis, calendar.data?.calendar?.definition) : []
  const keyOf = (id: string) => rows[index.get(id) ?? -1]?.item.ticket.key ?? ''
  const ticketOf = (id: string) => rows[index.get(id) ?? -1]?.item.ticket

  /** Brings today back into view; the first visit already starts there. */
  function goToday() {
    const box = scrollRef.current
    if (!box || !axis) return
    const left = Math.max(0, axis.x(today) - 48)
    if (typeof box.scrollTo === 'function') box.scrollTo({ left, behavior: typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' })
    else box.scrollLeft = left
  }

  return (
    <section className="timeline" aria-label={t('timeline.title')}>
      <header className="timeline-head">
        <h2>{t('timeline.title')}</h2>
        <div className="kind-switch" role="radiogroup" aria-label={t('timeline.scale')}>
          {(['day', 'week', 'month'] as const).map((value) => (
            <label key={value} className={scale === value ? 'is-selected' : undefined}>
              <input type="radio" name="timeline-scale" value={value} checked={scale === value} onChange={() => setScale(value)} />
              {t(`timeline.scale.${value}`)}
            </label>
          ))}
        </div>
        <div className="timeline-head-aside">
          {rows.length > 0 && axis ? <Button type="button" variant="secondary" onClick={goToday}>{t('timeline.goToday')}</Button> : null}
          {data.calendar ? (
            <small className="hint">{t('timeline.calendar.zone', { zone })}</small>
          ) : (
            <p className="hint" role="note">
              {t('timeline.calendar.none')}{' '}
              {calendar.data?.canManage ? <Link to={`/w/${workspaceId}/projects/${projectId}/settings`}>{t('timeline.calendar.create')}</Link> : null}
            </p>
          )}
        </div>
      </header>

      <SprintPeriods sprints={planning.data?.sprints ?? []} />
      {rows.length === 0 ? <p className="hint">{t('gantt.empty.none')}</p> : !drawn ? <p className="hint">{t('gantt.empty.noPlan')}</p> : null}
      {!data.canEdit ? <p className="board-note" role="note">{t('timeline.readOnly')}</p> : null}
      {conflicts.length > 0 ? (
        <div className="board-note conflict-note" role="note">
          <p>{t('timeline.planConflict', { count: String(conflicts.length) })}</p>
          <ul className="conflict-list">
            {conflicts.map((edge) => {
              const before = ticketOf(edge.predecessorId)
              const after = ticketOf(edge.successorId)
              return (
                <li key={edge.id}>
                  <button type="button" className="table-open" title={before?.title} aria-label={t('work.board.openTicket', { key: keyOf(edge.predecessorId) })} onClick={() => open(edge.predecessorId)}>
                    <small className="ticket-key">{keyOf(edge.predecessorId)}</small> {before?.title}
                  </button>
                  <span aria-hidden="true"> → </span>
                  <button type="button" className="table-open" title={after?.title} aria-label={t('work.board.openTicket', { key: keyOf(edge.successorId) })} onClick={() => open(edge.successorId)}>
                    <small className="ticket-key">{keyOf(edge.successorId)}</small> {after?.title}
                  </button>
                </li>
              )
            })}
          </ul>
        </div>
      ) : null}
      <p className="timeline-live hint" role="status" aria-live="polite">{live}</p>

      {rows.length > 0 && axis ? (
        <>
          <div className="gantt" role="group" aria-label={t('timeline.chart')}>
            <ul className="gantt-labels" aria-label={t('timeline.list')} style={{ paddingTop: HEAD }}>
              {rows.map((row) => {
                const ticket = row.item.ticket
                return (
                  <li key={ticket.id} style={{ height: ROW, paddingLeft: row.depth * 14 }} className={`gantt-label state-${row.item.statusCategory.toLowerCase()}`}>
                    <button type="button" className="table-open" data-gantt-label={ticket.id} title={ticket.title} aria-label={t('work.board.openTicket', { key: ticket.key })} onClick={() => open(ticket.id)}>
                      <small className="ticket-key">{ticket.key}</small> <span className="gantt-title">{ticket.title}</span>
                    </button>
                  </li>
                )
              })}
            </ul>
            <div className="gantt-scroll" ref={scrollRef}>
              <svg width={axis.width} height={height} viewBox={`0 0 ${String(axis.width)} ${String(height)}`} className="gantt-svg">
                <defs>
                  <marker id="gantt-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
                    <path d="M0,0 L8,4 L0,8 z" className="arrow-head" />
                  </marker>
                </defs>
                {off.map((span) => (
                  <rect key={span.x} x={span.x} y={HEAD - 6} width={span.width} height={height - HEAD + 6} className={span.holiday ? 'nonworking holiday' : 'nonworking'}>
                    <title>{span.name ? t('timeline.holiday', { name: span.name }) : t('timeline.offDay')}</title>
                  </rect>
                ))}
                {(planning.data?.sprints ?? []).map((sprint) => {
                  const start = axis.x(`${sprint.startsOn}T00:00:00Z`)
                  const end = axis.x(`${sprint.endsOn}T00:00:00Z`) + axis.dayWidth
                  if (!Number.isFinite(start) || !Number.isFinite(end)) return null
                  return <rect key={sprint.id} className={`sprint-band state-${sprint.state.toLowerCase()}`} x={Math.min(start, end)} y={2} width={Math.max(Math.abs(end - start), 2)} height={8}><title>{t('timeline.sprintBand', { name: sprint.name })}</title></rect>
                })}
                {(planning.data?.milestones ?? []).filter((milestone) => milestone.targetOn && milestone.state !== 'CANCELED').map((milestone) => {
                  const x = axis.x(`${milestone.targetOn}T00:00:00Z`)
                  if (!Number.isFinite(x)) return null
                  return (
                    <g key={milestone.id}>
                      <line x1={x} x2={x} y1={12} y2={height} className={`milestone-line state-${milestone.state.toLowerCase()}`} />
                      <title>{t('timeline.milestoneLine', { name: milestone.name })}</title>
                    </g>
                  )
                })}
                {axis.days.map((day, at) => {
                  const showLabel = scale === 'day' || (scale === 'week' && (at % 7 === 0 || day.first)) || (scale === 'month' && day.first)
                  return (
                  <g key={day.label}>
                    <line x1={day.x} x2={day.x} y1={HEAD - 6} y2={height} className="grid" />
                    {showLabel ? <text x={day.x + 3} y={HEAD - 12} className="axis-label">{day.label.slice(5)}</text> : null}
                  </g>
                  )
                })}
                {today >= axis.from && today <= axis.to ? (
                  <g>
                    <line x1={axis.x(today)} x2={axis.x(today)} y1={HEAD - 6} y2={height} className="today" />
                    <text x={axis.x(today) + 3} y={12} className="axis-label today-label">{t('timeline.today')}</text>
                  </g>
                ) : null}
                {data.edges.map((edge) => {
                  const from = index.get(edge.predecessorId)
                  const to = index.get(edge.successorId)
                  if (from === undefined || to === undefined) return null
                  const a = planBar(rows[from].item)
                  const b = planBar(rows[to].item)
                  if (!a || !b) return null
                  const canceled = edge.predecessorCategory === 'CANCELED' && !edge.waived
                  const conflict = planConflict(edge, rows)
                  const names = { from: rows[from].item.ticket.key, to: rows[to].item.ticket.key }
                  return (
                    <path
                      key={edge.id}
                      d={edgePath(axis.x(a.end), HEAD + from * ROW + ROW / 2, axis.x(b.start), HEAD + to * ROW + ROW / 2, ROW)}
                      className={['edge', canceled ? 'canceled' : '', conflict ? 'conflict' : '', edge.waived ? 'waived' : ''].filter(Boolean).join(' ')}
                      markerEnd="url(#gantt-arrow)"
                      fill="none"
                    >
                      <title>{canceled ? t('timeline.edge.canceled') : conflict ? t('timeline.edge.planConflict', names) : t('timeline.edge', names)}</title>
                    </path>
                  )
                })}
                {rows.map((row, at) => {
                  const bar = planBar(row.item)
                  const y = HEAD + at * ROW + (ROW - ROW_FINE) / 2
                  const ticket = row.item.ticket
                  return (
                    <g key={ticket.id}>
                      {bar ? (
                        <g
                          tabIndex={0}
                          role="button"
                          data-gantt-bar={ticket.id}
                          className={['bar', row.hasChildren ? 'summary' : '', `state-${row.item.statusCategory.toLowerCase()}`].filter(Boolean).join(' ')}
                          aria-label={
                            bar.milestone
                              ? t('timeline.barNode', { key: ticket.key, title: ticket.title, start: formatZoned(bar.start.toISOString(), zone, locale) })
                              : t('timeline.bar', { key: ticket.key, title: ticket.title, start: formatZoned(bar.start.toISOString(), zone, locale), end: formatZoned(bar.end.toISOString(), zone, locale) })
                          }
                          onKeyDown={(event) => onBarKey(event, row)}
                          onClick={() => { if (!(drag?.moved)) open(ticket.id) }}
                          onPointerDown={(event) => { if (!row.hasChildren) beginDrag(event, row, 'move') }}
                          onPointerMove={(event) => moveDrag(event, row)}
                          onPointerUp={(event) => endDrag(event, row)}
                          onPointerCancel={() => setDrag(null)}
                          transform={drag?.id === ticket.id && drag.kind === 'move' ? `translate(${String(drag.days * axis.dayWidth)} 0)` : undefined}
                        >
                          <rect className="hit" x={axis.x(bar.start) - 8} y={HEAD + at * ROW} width={Math.max(axis.x(bar.end) - axis.x(bar.start), 4) + 16} height={ROW} />
                          {bar.milestone ? (
                            <polygon points={`${String(axis.x(bar.start))},${String(y + 7)} ${String(axis.x(bar.start) + 7)},${String(y + 14)} ${String(axis.x(bar.start))},${String(y + 21)} ${String(axis.x(bar.start) - 7)},${String(y + 14)}`} />
                          ) : (
                            <rect x={axis.x(bar.start)} y={y + (row.hasChildren ? 10 : 6)} width={Math.max(axis.x(bar.end) - axis.x(bar.start), 4)} height={row.hasChildren ? 6 : 14} rx={3} />
                          )}
                          {!bar.milestone && !row.hasChildren && data.canEdit && row.item.schedule?.durationMinutes != null ? (
                            <rect
                              className="bar-handle"
                              x={axis.x(bar.end) - 4 + (drag?.id === ticket.id && drag.kind === 'resize' ? drag.days * axis.dayWidth : 0)}
                              y={y + 6}
                              width={8}
                              height={14}
                              onPointerDown={(event) => beginDrag(event, row, 'resize')}
                            />
                          ) : null}
                          {drag?.id === ticket.id && drag.moved && drag.days !== 0 ? (
                            <text className="drag-tip" x={axis.x(bar.end) + 12 + (drag.kind === 'resize' ? drag.days * axis.dayWidth : 0)} y={y + 18}>
                              {drag.kind === 'move'
                                ? formatZoned(shiftStart(row.item.schedule ?? { plannedStartAt: null, durationMinutes: null, isScheduleMilestone: false }, drag.days, zone)?.plannedStartAt ?? bar.start.toISOString(), zone, locale)
                                : formatMinutes(Math.max(1, (row.item.schedule?.durationMinutes ?? 0) + drag.days * dayMinutes), labels)}
                            </text>
                          ) : null}
                        </g>
                      ) : null}
                    </g>
                  )
                })}
              </svg>
            </div>
          </div>
          <Legend calendar={off.length > 0} conflicts={conflicts.length > 0} />
          <small className="hint">{t('timeline.hint', { day: formatMinutes(dayMinutes, labels) })}</small>
        </>
      ) : null}
      {data.nextCursor ? <p className="hint" role="note">{t('timeline.more', { count: String(data.items.length) })}</p> : null}

      <section className="timeline-unplanned" aria-label={t('timeline.unplanned')}>
        <h3>{t('timeline.unplanned')}</h3>
        {unplanned.length === 0 ? <p className="hint">{t('timeline.unplanned.empty')}</p> : (
          <ul className="list-rows">
            {unplanned.map(({ row, reason }) => (
              <li key={row.item.ticket.id} className="list-row">
                <button type="button" className="list-row-open" aria-label={t('work.board.openTicket', { key: row.item.ticket.key })} onClick={() => open(row.item.ticket.id)}>
                  <small className="ticket-key">{row.item.ticket.key}</small>
                  <strong className="list-row-title">{row.item.ticket.title}</strong>
                </button>
                <span className="list-row-meta">
                  <span className="where-chip">{reason ? t(`timeline.unplanned.${reason}`) : ''}</span>
                  {data.canEdit ? <Button type="button" variant="ghost" onClick={() => open(row.item.ticket.id)}>{t('timeline.unplanned.edit')}</Button> : null}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <details className="timeline-table">
        <summary>{t('timeline.table')}</summary>
        <div className="table-scroll" role="region" tabIndex={0} aria-label={t('timeline.table')}>
          <table className="record-table report-table">
            <thead>
              <tr>
                <th scope="col">{t('timeline.col.ticket')}</th>
                <th scope="col">{t('timeline.col.assignee')}</th>
                <th scope="col">{t('timeline.col.start')}</th>
                <th scope="col">{t('timeline.col.duration')}</th>
                <th scope="col">{t('timeline.col.end')}</th>
                <th scope="col">{t('timeline.col.state')}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const s = row.item.schedule
                return (
                  <tr key={row.item.ticket.id}>
                    <th scope="row" style={{ paddingLeft: `${String(0.5 + row.depth)}rem` }}>
                      <small className="ticket-key">{row.item.ticket.key}</small> {row.item.ticket.title}
                      {row.hasChildren ? <span className="where-chip">{t('timeline.summary')}</span> : null}
                    </th>
                    <td>{row.item.assigneeName ?? '—'}</td>
                    <td>{s?.plannedStartAt ? formatZoned(s.plannedStartAt, zone, locale) : '—'}</td>
                    <td>{s?.isScheduleMilestone ? t('work.schedule.milestoneChip') : s?.durationMinutes != null ? formatMinutes(s.durationMinutes, labels) : '—'}</td>
                    <td>{s?.plannedEndAt ? formatZoned(s.plannedEndAt, zone, locale) : '—'}</td>
                    <td>{t(`work.category.${row.item.statusCategory}`)}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      </details>
    </section>
  )
}

/**
 * Each Sprint's planned period (the band) and, where it differs, the actual start/end already recorded.
 * One line per Sprint; nothing here is a baseline or a snapshot.
 */
function SprintPeriods({ sprints }: { sprints: Array<{ id: string; name: string; startsOn: string; endsOn: string; startedAt: string | null; endedAt: string | null }> }) {
  const { t } = useI18n()
  const day = (iso: string) => iso.slice(0, 10)
  const rows = sprints.filter((sprint) => {
    if (!sprint.startedAt) return false
    return day(sprint.startedAt) !== sprint.startsOn || (sprint.endedAt !== null && day(sprint.endedAt) !== sprint.endsOn)
  })
  if (rows.length === 0) return null
  return (
    <ul className="sprint-periods" aria-label={t('timeline.sprintPeriods')}>
      {rows.map((sprint) => (
        <li key={sprint.id} className="sprint-actual-line">
          <strong>{sprint.name}</strong>{' · '}
          {t('timeline.sprintPlanned', { start: sprint.startsOn, end: sprint.endsOn })}{' · '}
          {sprint.endedAt
            ? t('timeline.sprintActual', { start: day(sprint.startedAt ?? ''), end: day(sprint.endedAt) })
            : t('timeline.sprintActualOngoing', { start: day(sprint.startedAt ?? '') })}
        </li>
      ))}
    </ul>
  )
}
