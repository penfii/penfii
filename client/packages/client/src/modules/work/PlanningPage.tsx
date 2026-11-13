import { useState, type FormEvent } from 'react'
import { Link, useSearchParams } from 'react-router'
import { useIsMobile } from '../../design/useIsMobile'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { newIdempotencyKey } from '@penfii/api-client'
import type { EstimateUnit, Milestone, ProjectPlanning, Sprint, SprintArchiveChoice, SprintOutcome, SprintResult } from '@penfii/contracts'
import { api } from '../../api'
import { Badge, Button, Dialog, Tabs } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { ListLayout } from './ListLayout'
import { useProjectLive } from './live'
import { useTicketPanel } from './panelParam'
import { refreshTicket } from './refresh'
import { SprintReportPanel, SprintSummaryStrip, VelocityPanel, velocityAverage } from './ReportPanels'
import { errorCode } from './ticketModel'
import { useNow } from '../../ui/useNow'
import { browserZone, formatZoned, instantToLocalInput, localInputToInstant, timeZones } from './zonedTime'
import { buildSections, defaultDefinition, emptyFilter, workTicketsFromBoard, type WorkTicket } from './viewModel'

type Mode = 'sprints' | 'milestones'
type Failure = 'generic' | 'conflict' | 'state' | null

/**
 * Planning: the Sprints and Milestones of one Project. A Sprint's Tickets are a List of the Work
 * dataset with a fixed Sprint filter while it is open; once it ends they come from the recorded
 * participation instead, because the current assignments are gone by then. Closing never changes a
 * Ticket's State.
 */
export function PlanningPage({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [mode, setMode] = useState<Mode>('sprints')
  const [search] = useSearchParams()
  const mobile = useIsMobile()
  const [failure, setFailure] = useState<Failure>(null)
  const planning = useQuery(queries.projectPlanning(projectId))
  useProjectLive(projectId)

  async function refresh() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.projectPlanning(projectId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintResults }),
      queryClient.invalidateQueries({ queryKey: queryKeys.all.sprintReport }),
      queryClient.invalidateQueries({ queryKey: queryKeys.projectVelocity(projectId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.projectStatusSummary(projectId) }),
    ])
  }

  async function run(action: () => Promise<unknown>): Promise<boolean> {
    setFailure(null)
    try {
      await action()
      await refresh()
      return true
    } catch (cause) {
      const code = errorCode(cause)
      setFailure(code === 'version_conflict' ? 'conflict' : code === 'invalid_sprint_state' ? 'state' : 'generic')
      await refresh()
      return false
    }
  }

  if (planning.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (planning.isError) return <p className="board-error" role="alert">{t('planning.error.generic')}</p>
  const data = planning.data
  // A Sprint opens as its own phone screen only if it is one of this Project's Sprints; any other id shows the list.
  const detailId = mobile ? search.get('sprintDetail') : null
  const detail = detailId ? data.sprints.find((sprint) => sprint.id === detailId) ?? null : null
  return (
    <div className="planning-page">
      {detail ? null : (
        <Tabs
          tabs={[
            { id: 'sprints', label: t('planning.tab.sprints') },
            { id: 'milestones', label: t('planning.tab.milestones') },
          ]}
          value={mode}
          onChange={(id) => setMode(id as Mode)}
        />
      )}
      {failure ? <p className="board-error" role="alert">{t(failure === 'conflict' ? 'planning.error.conflict' : failure === 'state' ? 'planning.error.state' : 'planning.error.generic')}</p> : null}
      {!data.canManage ? <p className="board-note" role="note">{t('planning.readOnly')}</p> : null}
      {mode === 'sprints' || detail ? (
        <SprintSection projectId={projectId} workspaceId={workspaceId} planning={data} run={run} detail={detail} />
      ) : (
        <MilestoneSection projectId={projectId} workspaceId={workspaceId} planning={data} run={run} />
      )}
    </div>
  )
}

type Run = (action: () => Promise<unknown>) => Promise<boolean>

/** The Project's Tickets as the rows List shows, loaded once for every list on the page. */
function useProjectRows(projectId: string, workspaceId: string) {
  const board = useQuery(queries.projectBoard(projectId))
  const space = useQuery(queries.space(workspaceId))
  const name = space.data?.projects.find((project) => project.id === projectId)?.name ?? ''
  const channels = new Map((space.data?.channels ?? []).filter((channel) => channel.projectId === projectId).map((channel) => [channel.id, channel.name]))
  return { board: board.data, tickets: board.data ? workTicketsFromBoard(board.data, name, channels) : [] }
}

function SprintSection({ projectId, workspaceId, planning, run, detail }: { projectId: string; workspaceId: string; planning: ProjectPlanning; run: Run; detail: Sprint | null }) {
  const { t } = useI18n()
  const manage = planning.canManage
  const mobile = useIsMobile()
  const [params, setParams] = useSearchParams()
  // On a phone a Sprint opens as its own screen (a pushed history entry), not as a section inside the list.
  const openDetail = mobile ? (sprint: Sprint) => { const next = new URLSearchParams(params); next.set('sprintDetail', sprint.id); setParams(next) } : undefined
  const [creating, setCreating] = useState(false)
  const [closing, setClosing] = useState<Sprint | null>(null)
  const [canceling, setCanceling] = useState<Sprint | null>(null)
  const [starting, setStarting] = useState<Sprint | null>(null)
  const activeSprint = planning.sprints.find((sprint) => sprint.state === 'ACTIVE') ?? null
  // The next Sprint begins the day after the latest one ends.
  const lastEnd = planning.sprints.reduce<string>((latest, sprint) => (sprint.endsOn > latest ? sprint.endsOn : latest), '')

  const shown = detail ? [detail] : planning.sprints
  const open = shown.filter((sprint) => sprint.state === 'PLANNED' || sprint.state === 'ACTIVE')
  const ended = shown.filter((sprint) => sprint.state === 'COMPLETED' || sprint.state === 'CANCELED')
  return (
    <div className="planning-body">
      {detail ? null : <VelocityPanel projectId={projectId} />}
      {shown.length === 0 ? <p className="hint" data-empty="sprints">{t(manage ? 'planning.sprint.empty' : 'planning.sprint.emptyReadOnly')}</p> : null}
      {open.map((sprint) => (
        <SprintCard key={sprint.id} sprint={sprint} projectId={projectId} workspaceId={workspaceId} manage={manage} run={run} activeSprint={activeSprint} onStart={() => setStarting(sprint)} onClose={() => setClosing(sprint)} onCancel={() => setCanceling(sprint)} onDetail={openDetail} detail={detail !== null} />
      ))}
      {ended.length > 0 ? <h3 className="planning-ended">{t('planning.sprint.ended')}</h3> : null}
      {ended.map((sprint) => (
        <SprintCard key={sprint.id} sprint={sprint} projectId={projectId} workspaceId={workspaceId} manage={manage} run={run} activeSprint={activeSprint} onStart={() => undefined} onClose={() => undefined} onCancel={() => undefined} onDetail={openDetail} detail={detail !== null} />
      ))}
      {detail ? null : <small className="field-hint">{t('planning.sprint.hint')}</small>}
      {manage && !detail ? <Button type="button" onClick={() => setCreating(true)}>{t('planning.sprint.new')}</Button> : null}
      {creating ? <SprintCreateDialog projectId={projectId} startsAfter={lastEnd} run={run} onDone={() => setCreating(false)} /> : null}
      {starting ? <SprintStartDialog sprint={starting} workspaceId={workspaceId} run={run} onDone={() => setStarting(null)} /> : null}
      {closing ? <CloseDialog sprint={closing} run={run} onDone={() => setClosing(null)} /> : null}
      <Dialog open={canceling !== null} title={t('planning.cancel.title')} onClose={() => setCanceling(null)}>
        {canceling ? (
          <div className="view-form">
            <p>{t('planning.cancel.body', { name: canceling.name })}</p>
            <div className="dialog-actions">
              <Button type="button" variant="secondary" onClick={() => setCanceling(null)}>{t('view.cancel')}</Button>
              <Button type="button" onClick={() => void run(() => api.sprintCommand(canceling.id, 'cancel', canceling.version, newIdempotencyKey())).then(() => setCanceling(null))}>{t('planning.sprint.cancel')}</Button>
            </div>
          </div>
        ) : null}
      </Dialog>
    </div>
  )
}

function SprintCard({ sprint, projectId, workspaceId, manage, run, activeSprint, onStart, onClose, onCancel, onDetail, detail = false }: { sprint: Sprint; projectId: string; workspaceId: string; manage: boolean; run: Run; activeSprint: Sprint | null; onStart: () => void; onClose: () => void; onCancel: () => void; onDetail?: (sprint: Sprint) => void; detail?: boolean }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(sprint.state === 'ACTIVE')
  // A phone shows the tickets on the Sprint's own screen; the list keeps one summary card per Sprint.
  const expanded = detail || (!onDetail && open)
  const isOpen = sprint.state === 'PLANNED' || sprint.state === 'ACTIVE'
  return (
    <article className={`planning-item sprint-card state-${sprint.state.toLowerCase()}`}>
      <div className="planning-item-main">
        <strong>{sprint.name}</strong>
        <Badge tone={sprint.state === 'ACTIVE' ? 'accent' : 'neutral'}>{t(`planning.sprint.state.${sprint.state}`)}</Badge>
        <small>{sprint.startsOn} – {sprint.endsOn}</small>
        {sprint.timeZone && sprint.estimateUnit ? <small className="sprint-basis">{t('planning.sprint.zoneUnit', { zone: sprint.timeZone, unit: t(`work.estimate.unit.${sprint.estimateUnit}`) })}</small> : null}
        {detail ? null : (
          <button type="button" className="btn btn-ghost" aria-expanded={onDetail ? undefined : expanded} onClick={() => (onDetail ? onDetail(sprint) : setOpen((value) => !value))}>
            {expanded ? t('planning.sprint.hideTickets') : t('planning.sprint.showTickets')}
          </button>
        )}
        <Link className="btn btn-ghost" to={`/w/${workspaceId}/projects/${projectId}/work?sprint=${sprint.id}&layout=board`}>{t('planning.sprint.board')}</Link>
      </div>
      {sprint.goal ? <p className="hint">{sprint.goal}</p> : null}
      {sprint.state === 'ACTIVE' ? <SprintSummaryStrip sprint={sprint} /> : null}
      {manage && isOpen ? (
        <div className="planning-actions">
          {sprint.state === 'PLANNED' ? (
            <Button type="button" variant="secondary" disabled={activeSprint !== null} aria-describedby={activeSprint ? `start-blocked-${sprint.id}` : undefined} onClick={onStart}>{t('planning.sprint.start')}</Button>
          ) : null}
          {sprint.state === 'PLANNED' && activeSprint ? <small id={`start-blocked-${sprint.id}`} className="field-hint">{t('planning.sprint.startBlocked', { name: activeSprint.name })}</small> : null}
          {sprint.state === 'ACTIVE' ? <Button type="button" variant="secondary" onClick={onClose}>{t('planning.sprint.complete')}</Button> : null}
          <Button type="button" variant="ghost" onClick={onCancel}>{t('planning.sprint.cancel')}</Button>
        </div>
      ) : null}
      {sprint.state === 'COMPLETED' ? <SprintArchiveNote sprint={sprint} manage={manage} /> : null}
      {expanded && sprint.state !== 'PLANNED' ? <SprintReportPanel sprint={sprint} /> : null}
      {expanded ? (isOpen ? <OpenSprintList sprint={sprint} projectId={projectId} workspaceId={workspaceId} manage={manage} run={run} /> : <ClosedSprintResults sprint={sprint} />) : null}
    </article>
  )
}

/** A PLANNED or ACTIVE Sprint: the Work dataset with a fixed Sprint filter, as a List. */
function OpenSprintList({ sprint, projectId, workspaceId, manage, run }: { sprint: Sprint; projectId: string; workspaceId: string; manage: boolean; run: Run }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { board, tickets } = useProjectRows(projectId, workspaceId)
  const { selectedId, open } = useTicketPanel()
  const [pick, setPick] = useState('')
  const results = useQuery(queries.sprintResults(sprint.id))
  if (!board) return <p className="hint" role="status">{t('loading')}</p>
  const definition = { ...defaultDefinition(), group: 'STATUS' as const, filter: { ...emptyFilter(), sprintIds: [sprint.id] } }
  const sections = buildSections(tickets, definition, { states: board.states, labels: labelSet(t) })
  const backlog = tickets.filter((ticket) => ticket.hierarchyKind === 'TASK' && ticket.sprint === null && ticket.stateCategory !== 'DONE' && ticket.stateCategory !== 'CANCELED')
  const added = new Set(results.data?.results.filter((row) => !row.inBaseline).map((row) => row.ticket.id) ?? [])
  const baseline = results.data?.baselineCount
  return (
    <div className="sprint-tickets">
      {baseline != null ? <small className="field-hint">{t('planning.sprint.baseline', { count: String(baseline), added: String(added.size) })}</small> : null}
      {sections.length === 0 ? <p className="hint">{t('planning.sprint.noTickets')}</p> : null}
      <ListLayout
        sections={sections}
        selectedId={selectedId}
        compact
        wide={false}
        states={board.states}
        busy={new Set()}
        canEdit={() => board.canEdit}
        onOpen={open}
        onState={(ticket, stateId) => {
          void api.changeTicketStatus(ticket.id, { expectedRevision: ticket.revisionNo, statusId: stateId }, newIdempotencyKey()).finally(() => void refreshTicket(queryClient, ticket.id, projectId))
        }}
      />
      {manage && backlog.length > 0 ? (
        <div className="dep-add">
          <select aria-label={t('planning.sprint.addPick')} value={pick} onChange={(event) => setPick(event.target.value)}>
            <option value="">{t('planning.sprint.addPick')}</option>
            {backlog.map((ticket) => (
              <option key={ticket.id} value={ticket.id}>{ticket.key} · {ticket.title}</option>
            ))}
          </select>
          <Button type="button" variant="secondary" disabled={!pick} onClick={() => void run(() => api.assignSprint(pick, sprint.id, newIdempotencyKey())).then((ok) => ok && setPick(''))}>{t('planning.sprint.add')}</Button>
        </div>
      ) : null}
    </div>
  )
}

function labelSet(t: ReturnType<typeof useI18n>['t']) {
  return {
    unassigned: t('view.group.none'),
    noPriority: t('view.group.noPriority'),
    backlog: t('view.group.backlog'),
    priority: (value: string) => t(`work.priority.${value as 'LOW'}`),
    category: (value: 'NOT_STARTED' | 'IN_PROGRESS' | 'DONE' | 'CANCELED') => t(`work.category.${value}`),
    noMilestone: t('view.group.noMilestone'),
  }
}

const OUTCOME_ORDER: SprintOutcome[] = ['DONE', 'NOT_DONE', 'CANCELED', 'REMOVED']

/** An ended Sprint: each Task with how it left, built from the recorded participation. */
function ClosedSprintResults({ sprint }: { sprint: Sprint }) {
  const { t } = useI18n()
  const { open } = useTicketPanel()
  const results = useQuery(queries.sprintResults(sprint.id))
  if (results.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (results.isError) return <p className="board-error" role="alert">{t('planning.error.generic')}</p>
  const rows = results.data.results
  const count = (outcome: SprintOutcome) => rows.filter((row) => row.outcome === outcome).length
  return (
    <div className="sprint-results">
      <p className="sprint-summary" role="status">
        {OUTCOME_ORDER.map((outcome) => t('planning.results.count', { label: t(`planning.outcome.${outcome}`), count: String(count(outcome)) })).join(' · ')}
        {results.data.baselineCount != null ? ` · ${t('planning.results.baseline', { count: String(results.data.baselineCount) })}` : ''}
      </p>
      {rows.length === 0 ? <p className="hint">{t('planning.sprint.noTickets')}</p> : null}
      <ul className="list-rows">
        {rows.map((row) => (
          <li key={row.ticket.id} className="list-row">
            <button type="button" className="list-row-open" aria-label={t('work.board.openTicket', { key: row.ticket.key })} onClick={() => open(row.ticket.id)}>
              <small className="ticket-key">{row.ticket.key}</small>
              <strong className="list-row-title">{row.ticket.title}</strong>
            </button>
            <span className="list-row-meta">
              <OutcomeChip row={row} />
              {!row.inBaseline ? <span className="where-chip">{t('planning.results.added')}</span> : null}
              {row.carriedFrom ? <span className="where-chip">{t('planning.results.from', { name: row.carriedFrom.name })}</span> : null}
              {row.needsNextAssignment ? <span className="blocked-chip">{t('planning.results.needsNext')}</span> : null}
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}

function OutcomeChip({ row }: { row: SprintResult }) {
  const { t } = useI18n()
  if (row.outcome === null) return <span className="state-chip">{t('planning.outcome.UNKNOWN')}</span>
  const label = t(`planning.outcome.${row.outcome}`)
  return (
    <span className={`state-chip outcome-${row.outcome.toLowerCase()}`}>
      {label}
      {row.carriedTo ? ` → ${row.carriedTo.name}` : ''}
    </span>
  )
}

function addDays(day: string, days: number): string {
  const date = new Date(`${day}T00:00:00Z`)
  date.setUTCDate(date.getUTCDate() + days)
  return date.toISOString().slice(0, 10)
}

const DURATIONS = [1, 2, 3, 4] as const

/** Create a Sprint: the start follows the previous Sprint, the end follows the chosen length and can still be edited. */
function SprintCreateDialog({ projectId, startsAfter, run, onDone }: { projectId: string; startsAfter: string; run: Run; onDone: () => void }) {
  const { t } = useI18n()
  const now = new Date(useNow())
  const today = `${String(now.getFullYear())}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`
  const firstStart = startsAfter ? addDays(startsAfter, 1) : today
  const [name, setName] = useState('')
  const [goal, setGoal] = useState('')
  const [starts, setStarts] = useState(firstStart)
  const [weeks, setWeeks] = useState<number>(2)
  const [ends, setEnds] = useState(addDays(firstStart, 13))
  // Both are optional: left alone, a Sprint takes the Workspace time zone and the Project's unit.
  const [zone, setZone] = useState('')
  const [unit, setUnit] = useState<'' | EstimateUnit>('')
  const [busy, setBusy] = useState(false)
  const ready = name.trim() !== '' && starts !== '' && ends !== '' && ends >= starts

  async function create(event: FormEvent) {
    event.preventDefault()
    if (!ready || busy) return
    setBusy(true)
    const ok = await run(() => api.createSprint(projectId, { name: name.trim(), ...(goal.trim() ? { goal: goal.trim() } : {}), startsOn: starts, endsOn: ends, ...(zone.trim() ? { timeZone: zone.trim() } : {}), ...(unit ? { estimateUnit: unit } : {}) }, newIdempotencyKey()))
    setBusy(false)
    if (ok) onDone()
  }

  return (
    <Dialog open title={t('planning.sprint.new')} onClose={onDone} presentation="full">
      <form className="view-form planning-form" onSubmit={(event) => void create(event)}>
        <input aria-label={t('planning.sprint.name')} placeholder={t('planning.sprint.name')} value={name} maxLength={80} autoFocus onChange={(event) => setName(event.target.value)} />
        <input aria-label={t('planning.sprint.goal')} placeholder={t('planning.sprint.goal')} value={goal} maxLength={500} onChange={(event) => setGoal(event.target.value)} />
        <div className="planning-dates">
          <label>
            <span>{t('planning.sprint.starts')}</span>
            <input type="date" value={starts} onChange={(event) => { setStarts(event.target.value); if (event.target.value) setEnds(addDays(event.target.value, weeks * 7 - 1)) }} />
          </label>
          <label>
            <span>{t('planning.sprint.length')}</span>
            <select value={weeks} onChange={(event) => { const next = Number(event.target.value); setWeeks(next); if (starts) setEnds(addDays(starts, next * 7 - 1)) }}>
              {DURATIONS.map((count) => <option key={count} value={count}>{t('planning.sprint.weeks', { count: String(count) })}</option>)}
            </select>
          </label>
          <label><span>{t('planning.sprint.ends')}</span><input type="date" value={ends} min={starts || undefined} onChange={(event) => setEnds(event.target.value)} /></label>
        </div>
        {startsAfter ? <small className="field-hint">{t('planning.sprint.startFollows', { day: startsAfter })}</small> : null}
        <details className="planning-advanced">
          <summary>{t('planning.sprint.timeZone')} · {t('planning.sprint.estimateUnit')}</summary>
          <label>
            <span>{t('planning.sprint.timeZone')}</span>
            <input aria-label={t('planning.sprint.timeZone')} list="sprint-time-zones" value={zone} placeholder="Asia/Seoul" onChange={(event) => setZone(event.target.value)} />
            <datalist id="sprint-time-zones">{timeZones().map((zoneName) => <option key={zoneName} value={zoneName} />)}</datalist>
          </label>
          <label>
            <span>{t('planning.sprint.estimateUnit')}</span>
            <select aria-label={t('planning.sprint.estimateUnit')} value={unit} onChange={(event) => setUnit(event.target.value === 'NONE' || event.target.value === 'POINTS' || event.target.value === 'HOURS' ? event.target.value : '')}>
              <option value="">—</option>
              {(['NONE', 'POINTS', 'HOURS'] as const).map((item) => <option key={item} value={item}>{t(`work.estimate.unit.${item}`)}</option>)}
            </select>
          </label>
          <small className="field-hint">{t('planning.sprint.fixedAtStart')}</small>
        </details>
        <div className="dialog-actions">
          <Button type="button" variant="secondary" onClick={onDone}>{t('view.cancel')}</Button>
          <Button type="submit" disabled={!ready || busy}>{t('planning.sprint.create')}</Button>
        </div>
      </form>
    </Dialog>
  )
}

function SprintStartDialog({ sprint, workspaceId, run, onDone }: { sprint: Sprint; workspaceId: string; run: Run; onDone: () => void }) {
  const { t } = useI18n()
  const { tickets } = useProjectRows(sprint.projectId, workspaceId)
  const velocity = useQuery(queries.projectVelocity(sprint.projectId))
  const included = tickets.filter((ticket) => ticket.hierarchyKind === 'TASK' && ticket.sprint?.id === sprint.id)
  const unit = sprint.estimateUnit
  const estimated = included.filter((ticket) => ticket.estimate && (!unit || unit === 'NONE' || ticket.estimate.unit === unit))
  const estimate = !unit || unit === 'NONE'
    ? String(included.length)
    : String(Math.round(estimated.reduce((sum, ticket) => sum + Number(ticket.estimate?.value ?? 0), 0) * 10) / 10)
  const average = velocity.data ? velocityAverage(velocity.data.sprints) : null
  const [busy, setBusy] = useState(false)
  async function start() {
    if (busy) return
    setBusy(true)
    const ok = await run(() => api.sprintCommand(sprint.id, 'start', sprint.version, newIdempotencyKey()))
    setBusy(false)
    if (ok) onDone()
  }
  return (
    <Dialog open title={t('planning.start.title', { name: sprint.name })} onClose={onDone}>
      <div className="view-form">
        <p>{sprint.startsOn} – {sprint.endsOn}</p>
        {sprint.goal ? <p className="hint">{sprint.goal}</p> : null}
        <p>{unit && unit !== 'NONE'
          ? t('planning.start.included', { count: String(included.length), estimate: t(`report.unit.${unit}`, { value: estimate }) })
          : t('planning.start.includedCount', { count: String(included.length) })}</p>
        <p className="hint">{average === null ? t('planning.start.noAverage') : t('planning.start.average', { average: unit && unit !== 'NONE' ? t(`report.unit.${unit}`, { value: String(average) }) : String(average) })}</p>
        <small className="field-hint">{t('planning.start.note')}</small>
        <div className="dialog-actions">
          <Button type="button" variant="secondary" onClick={onDone}>{t('view.cancel')}</Button>
          <Button type="button" disabled={busy} onClick={() => void start()}>{t('planning.sprint.start')}</Button>
        </div>
      </div>
    </Dialog>
  )
}

/** The archive scheduled when a Sprint was completed: when it runs, what it did, and (for an admin) change or cancel it. */
function SprintArchiveNote({ sprint, manage }: { sprint: Sprint; manage: boolean }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const schedule = useQuery(queries.sprintArchiveSchedule(sprint.id))
  const [editing, setEditing] = useState(false)
  const [local, setLocal] = useState('')
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)
  const zone = schedule.data?.timeZone ?? sprint.timeZone ?? browserZone()
  const now = useNow()
  const data = schedule.data
  if (!data) return null
  const instant = editing && local ? localInputToInstant(local, zone) : null
  const future = instant !== null && Date.parse(instant) > now

  async function change(action: () => Promise<unknown>) {
    setBusy(true)
    setFailed(false)
    try {
      await action()
      setEditing(false)
      await queryClient.invalidateQueries({ queryKey: queryKeys.sprintArchiveSchedule(sprint.id) })
    } catch {
      setFailed(true)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="sprint-archive-note" role="status">
      {data.state === 'SCHEDULED' ? <span>{t('planning.archive.scheduled', { when: formatZoned(data.dueAt, zone, locale) })}</span> : null}
      {data.state === 'DONE' ? <span>{t('planning.archive.done', { archived: String(data.archivedCount ?? 0), skipped: String(data.skippedCount ?? 0) })}</span> : null}
      {data.state === 'CANCELED' ? <span>{t('planning.archive.canceled')}</span> : null}
      {manage && data.state === 'SCHEDULED' ? (
        editing ? (
          <>
            <input type="datetime-local" aria-label={t('planning.close.archive.when', { zone })} value={local} onChange={(event) => setLocal(event.target.value)} />
            <Button type="button" disabled={busy || !future} onClick={() => void change(() => api.rescheduleSprintArchive(sprint.id, data.version, instant ?? ''))}>{t('planning.archive.save')}</Button>
            <Button type="button" variant="ghost" onClick={() => setEditing(false)}>{t('view.cancel')}</Button>
          </>
        ) : (
          <>
            <Button type="button" variant="ghost" onClick={() => { setLocal(instantToLocalInput(data.dueAt, zone)); setEditing(true) }}>{t('planning.archive.change')}</Button>
            <Button type="button" variant="ghost" disabled={busy} onClick={() => void change(() => api.cancelSprintArchive(sprint.id, data.version))}>{t('planning.archive.cancel')}</Button>
          </>
        )
      ) : null}
      {failed ? <small className="form-error" role="alert">{t('planning.archive.error')}</small> : null}
    </div>
  )
}

/** The one decision of closing: where the unfinished Tasks go. The preview is the place to turn back. */
function CloseDialog({ sprint, run, onDone }: { sprint: Sprint; run: Run; onDone: () => void }) {
  const { t } = useI18n()
  const preview = useQuery({ ...queries.sprintPreview(sprint.id), gcTime: 0 })
  const [choice, setChoice] = useState<string | null | undefined>(undefined)
  const [busy, setBusy] = useState(false)
  const target = choice === undefined ? (preview.data?.defaultTargetId ?? null) : choice
  const zone = sprint.timeZone ?? browserZone()
  const [archiveMode, setArchiveMode] = useState<'NONE' | 'IMMEDIATE' | 'AT'>('NONE')
  const [archiveLocal, setArchiveLocal] = useState('')
  const now = useNow()
  const archiveInstant = archiveMode === 'AT' && archiveLocal ? localInputToInstant(archiveLocal, zone) : null
  const archivePast = archiveInstant !== null && Date.parse(archiveInstant) <= now
  const archiveReady = archiveMode !== 'AT' || (archiveInstant !== null && !archivePast)
  async function confirm() {
    if (!preview.data || busy || !archiveReady) return
    setBusy(true)
    const archive: SprintArchiveChoice = archiveMode === 'AT' && archiveInstant ? { mode: 'AT', at: archiveInstant } : { mode: archiveMode === 'IMMEDIATE' ? 'IMMEDIATE' : 'NONE' }
    const ok = await run(() => api.sprintCommand(sprint.id, 'complete', preview.data.sprintVersion, newIdempotencyKey(), target, archive))
    setBusy(false)
    if (ok) onDone()
  }
  return (
    <Dialog open title={t('planning.close.title', { name: sprint.name })} onClose={onDone}>
      {preview.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {preview.isError ? <p className="board-error" role="alert">{t('planning.error.generic')}</p> : null}
      {preview.data ? (
        <div className="view-form close-form">
          <p className="close-counts" role="status">
            {t('planning.close.counts', { done: String(preview.data.done.length), canceled: String(preview.data.canceled.length), unfinished: String(preview.data.unfinished.length) })}
          </p>
          {preview.data.unfinished.length > 0 ? (
            <ul className="close-list">
              {preview.data.unfinished.map((ticket) => (
                <li key={ticket.id}><small className="ticket-key">{ticket.key}</small> {ticket.title}</li>
              ))}
            </ul>
          ) : null}
          <fieldset className="filter-group">
            <legend>{t('planning.close.where')}</legend>
            <label className="filter-choice">
              <input type="radio" name="carry" checked={target === null} onChange={() => setChoice(null)} />
              <span>{t('planning.close.backlog')}</span>
            </label>
            {preview.data.targets.map((item) => (
              <label key={item.id} className="filter-choice">
                <input type="radio" name="carry" checked={target === item.id} onChange={() => setChoice(item.id)} />
                <span>{item.name}{item.id === preview.data.defaultTargetId ? ` (${t('planning.close.next')})` : ''}</span>
              </label>
            ))}
          </fieldset>
          <small className="field-hint">{t('planning.close.note')}</small>
          <fieldset className="filter-group">
            <legend>{t('planning.close.archive')}</legend>
            {(['NONE', 'IMMEDIATE', 'AT'] as const).map((mode) => (
              <label key={mode} className="filter-choice">
                <input type="radio" name="archive" checked={archiveMode === mode} onChange={() => setArchiveMode(mode)} />
                <span>{t(`planning.close.archive.${mode === 'AT' ? 'at' : mode === 'NONE' ? 'none' : 'immediate'}`)}</span>
              </label>
            ))}
            {archiveMode === 'AT' ? (
              <label className="field">
                <span>{t('planning.close.archive.when', { zone })}</span>
                <input type="datetime-local" value={archiveLocal} onChange={(event) => setArchiveLocal(event.target.value)} />
              </label>
            ) : null}
            {archivePast ? <small className="form-error" role="alert">{t('planning.close.archive.past')}</small> : null}
            <small className="field-hint">{t('planning.close.archive.note')}</small>
          </fieldset>
          <div className="dialog-actions">
            <Button type="button" variant="secondary" onClick={onDone}>{t('view.cancel')}</Button>
            <Button type="button" disabled={busy || !archiveReady} onClick={() => void confirm()}>{t('planning.close.confirm')}</Button>
          </div>
        </div>
      ) : null}
    </Dialog>
  )
}

// ---- Milestones ---------------------------------------------------------------------------

function MilestoneSection({ projectId, workspaceId, planning, run }: { projectId: string; workspaceId: string; planning: ProjectPlanning; run: Run }) {
  const { t } = useI18n()
  const manage = planning.canManage
  const [name, setName] = useState('')
  const [target, setTarget] = useState('')
  const [busy, setBusy] = useState(false)

  async function create(event: FormEvent) {
    event.preventDefault()
    if (!name.trim() || busy) return
    setBusy(true)
    const ok = await run(() => api.createMilestone(projectId, { name: name.trim(), targetOn: target || null }, newIdempotencyKey()))
    setBusy(false)
    if (ok) {
      setName('')
      setTarget('')
    }
  }
  return (
    <div className="planning-body">
      {planning.milestones.length === 0 ? <p className="hint" data-empty="milestones">{t(manage ? 'planning.milestone.empty' : 'planning.milestone.emptyReadOnly')}</p> : null}
      {planning.milestones.map((milestone) => (
        <MilestoneCard key={milestone.id} milestone={milestone} projectId={projectId} workspaceId={workspaceId} manage={manage} run={run} />
      ))}
      <small className="field-hint">{t('planning.milestone.hint')}</small>
      {manage ? (
        <form className="planning-form" onSubmit={(event) => void create(event)}>
          <strong>{t('planning.milestone.new')}</strong>
          <input aria-label={t('planning.milestone.name')} placeholder={t('planning.milestone.name')} value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
          <label className="planning-target"><span>{t('planning.milestone.target')}</span><input type="date" value={target} onChange={(event) => setTarget(event.target.value)} /></label>
          <Button type="submit" disabled={!name.trim() || busy}>{t('planning.milestone.create')}</Button>
        </form>
      ) : null}
    </div>
  )
}

function daySpan(from: string, to: string): number {
  return Math.round((Date.parse(`${to}T00:00:00Z`) - Date.parse(`${from}T00:00:00Z`)) / 86_400_000)
}

function localDay(iso: string): string {
  const date = new Date(iso)
  return `${String(date.getFullYear())}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}

function MilestoneCard({ milestone, projectId, workspaceId, manage, run }: { milestone: Milestone; projectId: string; workspaceId: string; manage: boolean; run: Run }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { tickets } = useProjectRows(projectId, workspaceId)
  const now = useNow()
  const [expanded, setExpanded] = useState(false)
  const setState = (state: Milestone['state']) => void run(() => api.updateMilestone(milestone.id, { expectedVersion: milestone.version, state }))
  const linked = tickets.filter((ticket) => ticket.hierarchyKind !== 'EPIC' && ticket.milestones.some((item) => item.id === milestone.id) && ticket.stateCategory !== 'CANCELED')
  const done = linked.filter((ticket) => ticket.stateCategory === 'DONE').length
  const percent = linked.length === 0 ? 0 : Math.round((done / linked.length) * 100)
  const today = localDay(new Date(now).toISOString())
  const remaining = milestone.targetOn ? daySpan(today, milestone.targetOn) : null
  const reachedDay = milestone.reachedAt ? localDay(milestone.reachedAt) : null
  const reachedDelta = milestone.state === 'REACHED' && reachedDay && milestone.targetOn ? daySpan(reachedDay, milestone.targetOn) : null
  const warning = milestone.state === 'OPEN' && remaining !== null && remaining < 0
  return (
    <article className={`planning-item state-${milestone.state.toLowerCase()}${warning ? ' is-warning' : ''}`}>
      <div className="planning-item-main">
        <strong>{milestone.name}</strong>
        <Badge tone={milestone.state === 'REACHED' ? 'accent' : 'neutral'}>{t(`planning.milestone.state.${milestone.state}`)}</Badge>
        {milestone.targetOn ? <small>{milestone.targetOn}</small> : null}
        {linked.length > 0 ? <small>{t('planning.milestone.progress', { done: String(done), total: String(linked.length), percent: String(percent) })}</small> : null}
        {milestone.state === 'OPEN' && remaining !== null ? (
          <small className={warning ? 'field-error' : 'hint'}>
            {remaining < 0 ? t('planning.milestone.overdue', { days: String(-remaining) }) : remaining === 0 ? t('planning.milestone.dueToday') : t('planning.milestone.remaining', { days: String(remaining) })}
          </small>
        ) : null}
        {milestone.state === 'REACHED' ? (
          <small>
            {reachedDelta === null ? t('planning.milestone.reachedUnknown') : reachedDelta > 0 ? t('planning.milestone.early', { days: String(reachedDelta) }) : reachedDelta < 0 ? t('planning.milestone.late', { days: String(-reachedDelta) }) : t('planning.milestone.onTime')}
          </small>
        ) : null}
        <button type="button" className="btn btn-ghost" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}>
          {expanded ? t('planning.sprint.hideTickets') : t('planning.sprint.showTickets')}
        </button>
      </div>
      {manage ? (
        <div className="planning-actions">
          {milestone.state === 'OPEN' ? (
            <>
              <Button type="button" variant="secondary" onClick={() => setState('REACHED')}>{t('planning.milestone.reach')}</Button>
              <Button type="button" variant="ghost" onClick={() => setState('CANCELED')}>{t('planning.milestone.cancel')}</Button>
            </>
          ) : (
            <Button type="button" variant="ghost" onClick={() => setState('OPEN')}>{t('planning.milestone.reopen')}</Button>
          )}
        </div>
      ) : null}
      {expanded ? <MilestoneTickets milestone={milestone} projectId={projectId} workspaceId={workspaceId} onChanged={(ticket) => void refreshTicket(queryClient, ticket.id, projectId)} /> : null}
    </article>
  )
}

function MilestoneTickets({ milestone, projectId, workspaceId, onChanged }: { milestone: Milestone; projectId: string; workspaceId: string; onChanged: (ticket: WorkTicket) => void }) {
  const { t } = useI18n()
  const { board, tickets } = useProjectRows(projectId, workspaceId)
  const { selectedId, open } = useTicketPanel()
  if (!board) return <p className="hint" role="status">{t('loading')}</p>
  const definition = { ...defaultDefinition(), group: 'STATUS' as const, filter: { ...emptyFilter(), milestoneIds: [milestone.id] } }
  const sections = buildSections(tickets, definition, { states: board.states, labels: labelSet(t) })
  return (
    <div className="sprint-tickets">
      {sections.length === 0 ? <p className="hint">{t('planning.milestone.noTickets')}</p> : null}
      <ListLayout
        sections={sections}
        selectedId={selectedId}
        compact
        wide={false}
        states={board.states}
        busy={new Set()}
        canEdit={() => board.canEdit}
        onOpen={open}
        onState={(ticket, stateId) => {
          void api.changeTicketStatus(ticket.id, { expectedRevision: ticket.revisionNo, statusId: stateId }, newIdempotencyKey()).finally(() => onChanged(ticket))
        }}
      />
    </div>
  )
}
