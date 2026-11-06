import { useCallback, useEffect, useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, newIdempotencyKey } from '@penfii/api-client'
import type { BulkResult, ScopeLens, ScopeProject, ViewDefinition, ViewSortField } from '@penfii/contracts'
import { api } from '../../api'
import { Button, EmptyState } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { BoardLayout } from './Board'
import { BoardSettings } from './BoardSettings'
import { BulkBar } from './BulkBar'
import { PhoneBulk } from './PhoneBulk'
import { ListLayout, type ListReorder } from './ListLayout'
import { moveErrorKey } from './moveErrors'
import { useProjectLive } from './live'
import { useTicketPanel } from './panelParam'
import { Popover } from './Popover'
import { refreshTicket } from './refresh'
import { TableLayout } from './TableLayout'
import { placementForStep } from './boardMoves'
import { useWorkView, type WorkTarget } from './useWorkView'
import { ViewBar } from './ViewBar'
import { buildSections, siblingIds, tableColumns, workTicketsFromBoard, type Section, type WorkTicket } from './viewModel'
import { GearIcon } from '../../design/icons'
import { useIsMobile } from '../../design/useIsMobile'

/**
 * Work: one Ticket dataset shown as a Board, a List or a Table, from wherever the person is. A
 * Project offers all three; a Team or the Workspace offer List and Table, because a Board's
 * columns are one Project's States. The View decides which Tickets, how grouped and how shown;
 * the layout only draws them.
 */
export function Work({ target, lens = 'ALL', title }: { target: WorkTarget; lens?: ScopeLens; title?: string }) {
  const { t } = useI18n()
  const [message, setMessage] = useState('')
  const onMissing = useCallback(() => setMessage(t('view.error.missing')), [t])
  const view = useWorkView(target, onMissing)
  const wide = target.kind !== 'PROJECT'
  return (
    <div className="board-view">
      {wide ? (
        <WideWork target={target} lens={lens} view={view} message={message} setMessage={setMessage} title={title} />
      ) : (
        <ProjectWork target={target} view={view} message={message} setMessage={setMessage} />
      )}
    </div>
  )
}

type ViewState = ReturnType<typeof useWorkView>

function sectionLabels(t: ReturnType<typeof useI18n>['t']) {
  return {
    unassigned: t('view.group.none'),
    noPriority: t('view.group.noPriority'),
    backlog: t('view.group.backlog'),
    priority: (value: string) => t(`work.priority.${value as 'LOW'}`),
    category: (value: 'NOT_STARTED' | 'IN_PROGRESS' | 'DONE' | 'CANCELED') => t(`work.category.${value}`),
    noMilestone: t('view.group.noMilestone'),
  }
}

/**
 * A phone has no Table: a saved Table view is shown as a List, and the Table's columns become the fields a
 * List row shows. On a wider screen the layout is the view's own and a row shows everything.
 */
function usePhoneLayout(view: ViewState, wide: boolean) {
  const mobile = useIsMobile()
  const layout = mobile && view.layout === 'TABLE' ? 'LIST' : view.layout
  return { layout, mobile, rowFields: mobile ? new Set(tableColumns(view.definition, wide)) : undefined }
}

// ---- Project scope ------------------------------------------------------------------------

function ProjectWork({ target, view, message, setMessage }: { target: Extract<WorkTarget, { kind: 'PROJECT' }>; view: ViewState; message: string; setMessage: (message: string) => void }) {
  const { layout, mobile, rowFields } = usePhoneLayout(view, false)
  const { t } = useI18n()
  const { projectId, workspaceId } = target
  const queryClient = useQueryClient()
  const board = useQuery({ ...queries.projectBoard(projectId), refetchOnWindowFocus: true })
  const planning = useQuery(queries.projectPlanning(projectId))
  const labels = useQuery(queries.projectLabels(projectId))
  const members = useQuery(queries.projectMembers(projectId))
  const types = useQuery(queries.projectTypes(projectId))
  const space = useQuery(queries.space(workspaceId))
  useProjectLive(projectId)
  const { selectedId, open, openCreate } = useTicketPanel()

  const projectName = space.data?.projects.find((project) => project.id === projectId)?.name ?? ''
  const channels = useMemo(() => (space.data?.channels ?? []).filter((channel) => channel.projectId === projectId), [space.data, projectId])
  const data = board.data
  const tickets = useMemo(
    () => (data ? workTicketsFromBoard(data, projectName, new Map(channels.map((channel) => [channel.id, channel.name]))) : []),
    [data, projectName, channels],
  )
  const [selection, setSelection] = useState<Set<string>>(() => new Set())
  const [bulkResult, setBulkResult] = useState<BulkResult | null>(null)
  const [selecting, setSelecting] = useState(false)
  const [busy, setBusy] = useState<Set<string>>(() => new Set())
  useEffect(() => {
    setSelection(new Set())
    setBulkResult(null)
    setSelecting(false)
  }, [layout, projectId])

  if (board.isPending) return <div className="board-loading" aria-label={t('loading')}><span /><span /><span /></div>
  if (board.isError || !data) {
    return (
      <EmptyState title={t('shell.project.work')} body={isApiError(board.error) && board.error.status === 403 ? t('projects.boardPermissionError') : t('work.board.loadError')}>
        <Button variant="secondary" onClick={() => void board.refetch()}>{t('message.retry')}</Button>
      </EmptyState>
    )
  }

  const definition = view.definition
  const labelSet = sectionLabels(t)
  const sections = layout === 'BOARD' ? [] : buildSections(tickets, definition, { states: data.states, sprints: planning.data?.sprints, members: members.data ?? [], labels: labelSet })
  // The count of Tickets the View shows, whatever the layout draws.
  const shown = new Set(buildSections(tickets, { ...definition, group: 'NONE' }, { labels: labelSet }).flatMap((section) => section.tickets.map((ticket) => ticket.id))).size
  const total = tickets.length

  async function changeState(ticket: WorkTicket, stateId: string) {
    if (busy.has(ticket.id)) return
    setMessage('')
    setBusy((items) => new Set(items).add(ticket.id))
    try {
      await api.changeTicketStatus(ticket.id, { expectedRevision: ticket.revisionNo, statusId: stateId }, newIdempotencyKey())
    } catch (cause) {
      setMessage(t(moveErrorKey(cause)))
    } finally {
      setBusy((items) => {
        const next = new Set(items)
        next.delete(ticket.id)
        return next
      })
      await refreshTicket(queryClient, ticket.id, projectId)
    }
  }

  async function place(ticket: WorkTicket, placement: Parameters<typeof api.moveTicket>[1]) {
    if (busy.has(ticket.id)) return
    setMessage('')
    setBusy((items) => new Set(items).add(ticket.id))
    try {
      await api.moveTicket(ticket.id, placement, newIdempotencyKey())
    } catch (cause) {
      setMessage(t(moveErrorKey(cause)))
    } finally {
      setBusy((items) => {
        const next = new Set(items)
        next.delete(ticket.id)
        return next
      })
      await refreshTicket(queryClient, ticket.id, projectId)
    }
  }

  // The shared order is the Project's; any section of a List may move a row within itself.
  const ordered = definition.sort.field === 'RANK' && definition.sort.dir === 'ASC'
  const reorder: ListReorder | undefined =
    data.canEdit && ordered
      ? {
          canStep: (ticket, section) => siblingIds(section, ticket).length > 1,
          step: (ticket, section, step) => {
            const placement = placementForStep(siblingIds(section, ticket), ticket.id, step)
            if (placement) void place(ticket, placement)
          },
          top: (ticket, section) => {
            const first = siblingIds(section, ticket)[0]
            if (first && first !== ticket.id) void place(ticket, { beforeId: first })
            else if (!first) void place(ticket, { position: 'first' })
          },
        }
      : undefined

  const chosen = tickets.filter((ticket) => selection.has(ticket.id))

  function setSort(field: ViewSortField) {
    const same = definition.sort.field === field
    view.setDefinition({ ...definition, sort: { field, dir: same && definition.sort.dir === 'ASC' ? 'DESC' : 'ASC' } })
  }

  function afterBulk(result: BulkResult | null) {
    setBulkResult(result)
    if (!result) return
    const failed = new Set(result.items.filter((item) => !item.ok).map((item) => item.ticketId))
    setSelection(failed)
    void Promise.all([queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }), queryClient.invalidateQueries({ queryKey: queryKeys.projectPlanning(projectId) }), queryClient.invalidateQueries({ queryKey: queryKeys.all.ticket })])
  }

  return (
    <>
      <ViewBar
        target="PROJECT"
        layout={layout}
        isAdmin={data.canManageWorkflow}
        views={view.views}
        activeId={view.active?.id ?? null}
        definition={definition}
        dirty={view.dirty}
        labels={labels.data ?? []}
        members={members.data ?? []}
        types={types.data ?? []}
        planning={planning.data}
        channels={channels}
        onLayout={view.setLayout}
        onSelect={(id) => {
          setMessage('')
          view.select(id)
        }}
        onChange={view.setDefinition}
        onReset={view.reset}
        onSave={view.saveChanges}
        onSaveAs={view.saveAs}
        onRename={view.rename}
        onDelete={view.remove}
        actions={
          <Popover label={t('workflow.open')} icon={<GearIcon />} align="end" wide>
            {() => <BoardSettings projectId={projectId} board={data} />}
          </Popover>
        }
      />
      <div className="board-toolbar">
        {!data.canEdit ? <p className="board-note" role="note">{t('work.board.readOnly')}</p> : <span />}
        {shown !== total ? <small className="field-hint">{t('view.filter.result', { shown: String(shown), total: String(total) })}</small> : null}
        {data.canEdit && layout !== 'BOARD' ? <Button type="button" variant="secondary" onClick={() => openCreate({ projectId })}>{t('work.addTicket')}</Button> : null}
        {mobile && layout === 'LIST' && data.canEdit && total > 0 ? <Button type="button" variant="secondary" aria-pressed={selecting} onClick={() => { setSelecting((on) => !on); setSelection(new Set()); setBulkResult(null) }}>{t('bulk.select')}</Button> : null}
      </div>
      {layout === 'BOARD' ? (
        <BoardLayout projectId={projectId} data={data} definition={definition} planning={planning.data} members={members.data ?? []} message={message} onMessage={setMessage} />
      ) : (
        <>
          {message ? <p className="board-error" role="alert">{message}</p> : null}
          {layout === 'TABLE' && (chosen.length > 0 || bulkResult) ? <BulkBar workspaceId={workspaceId} selected={chosen} result={bulkResult} names={new Map(tickets.map((ticket) => [ticket.id, { key: ticket.key, title: ticket.title }]))} onClear={() => { setSelection(new Set()); setBulkResult(null) }} onResult={afterBulk} /> : null}
          {total === 0 ? <p className="board-empty">{t('work.board.empty')}</p> : null}
          {total > 0 && sections.length === 0 ? <p className="board-empty">{t('work.list.noMatch')}</p> : null}
          <div className="work-scroll">
            {layout === 'LIST' ? (
              <ListLayout
                sections={sections}
                selectedId={selectedId}
                compact={definition.display.compact}
                wide={false}
                states={data.states}
                busy={busy}
                canEdit={() => data.canEdit}
                reorder={reorder}
                onOpen={(id) => {
                  setMessage('')
                  open(id)
                }}
                onState={(ticket, stateId) => void changeState(ticket, stateId)}
                fields={rowFields}
                select={selecting ? { chosen: selection, onToggle: (id) => setSelection((current) => toggleAll(current, [id], !current.has(id))) } : undefined}
              />
            ) : (
              <TableLayout
                sections={sections}
                columns={tableColumns(definition, false)}
                sort={definition.sort}
                selectedId={selectedId}
                selection={selection}
                compact={definition.display.compact}
                states={data.states}
                busy={busy}
                canEdit={() => data.canEdit}
                onSort={setSort}
                onSelect={(ids, on) => setSelection((current) => toggleAll(current, ids, on))}
                onOpen={(id) => {
                  setMessage('')
                  open(id)
                }}
                onState={(ticket, stateId) => void changeState(ticket, stateId)}
              />
            )}
          </div>
          {selecting && layout === 'LIST' ? <PhoneBulk workspaceId={workspaceId} selected={chosen} result={bulkResult} names={new Map(tickets.map((ticket) => [ticket.id, { key: ticket.key, title: ticket.title }]))} onAll={() => setSelection((current) => (current.size >= shown ? new Set() : new Set(sections.flatMap((section) => section.tickets).map((ticket) => ticket.id))))} onDone={() => { setSelecting(false); setSelection(new Set()); setBulkResult(null) }} onResult={afterBulk} /> : null}
        </>
      )}
    </>
  )
}

function toggleAll(current: Set<string>, ids: string[], on: boolean): Set<string> {
  const next = new Set(current)
  for (const id of ids) {
    if (on) next.add(id)
    else next.delete(id)
  }
  return next
}

// ---- Team and Workspace scope ---------------------------------------------------------------

function WideWork({
  target,
  lens,
  view,
  message,
  setMessage,
  title,
}: {
  target: Exclude<WorkTarget, { kind: 'PROJECT' }>
  lens: ScopeLens
  view: ViewState
  message: string
  setMessage: (message: string) => void
  title?: string
}) {
  const { layout, mobile, rowFields } = usePhoneLayout(view, true)
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { selectedId, open } = useTicketPanel()
  const scope = target.kind === 'TEAM' ? ({ kind: 'TEAM', teamId: target.teamId } as const) : ({ kind: 'WORKSPACE' } as const)
  // Choosing Projects in the filter also narrows the request, because a scope over the cap is cut on the server.
  const narrowed = [...view.definition.filter.projectIds].sort()
  const dataset = useQuery({ ...queries.scopeDataset(target.workspaceId, scope, lens, narrowed), refetchOnWindowFocus: true, placeholderData: (previous) => previous })
  const [selection, setSelection] = useState<Set<string>>(() => new Set())
  const [bulkResult, setBulkResult] = useState<BulkResult | null>(null)
  const [selecting, setSelecting] = useState(false)
  const [busy, setBusy] = useState<Set<string>>(() => new Set())
  useEffect(() => {
    setSelection(new Set())
    setBulkResult(null)
    setSelecting(false)
  }, [layout, lens])

  const tickets: WorkTicket[] = useMemo(() => dataset.data?.tickets ?? [], [dataset.data])
  const projects: ScopeProject[] = dataset.data?.projects ?? []
  const people = useMemo(() => {
    const seen = new Map<string, string>()
    for (const ticket of tickets) if (ticket.assigneeId && ticket.assigneeName) seen.set(ticket.assigneeId, ticket.assigneeName)
    return [...seen.entries()].map(([userId, displayName]) => ({ userId, displayName }))
  }, [tickets])

  if (dataset.isPending) return <div className="board-loading" aria-label={t('loading')}><span /><span /><span /></div>
  if (dataset.isError || !dataset.data) {
    return (
      <EmptyState title={title ?? t('work.wide.title')} body={t('work.wide.loadError')}>
        <Button variant="secondary" onClick={() => void dataset.refetch()}>{t('message.retry')}</Button>
      </EmptyState>
    )
  }

  const definition = view.definition
  const sections = buildSections(tickets, definition, { members: people, labels: sectionLabels(t) })
  const shown = new Set(sections.flatMap((section) => section.tickets.map((ticket) => ticket.id))).size
  const editable = new Map(projects.map((project) => [project.id, project.canEdit]))

  async function changeState(ticket: WorkTicket, stateId: string) {
    if (busy.has(ticket.id)) return
    setMessage('')
    setBusy((items) => new Set(items).add(ticket.id))
    try {
      await api.changeTicketStatus(ticket.id, { expectedRevision: ticket.revisionNo, statusId: stateId }, newIdempotencyKey())
    } catch (cause) {
      setMessage(t(moveErrorKey(cause)))
    } finally {
      setBusy((items) => {
        const next = new Set(items)
        next.delete(ticket.id)
        return next
      })
      await refreshTicket(queryClient, ticket.id, ticket.projectId)
    }
  }

  function setSort(field: ViewSortField) {
    const same = definition.sort.field === field
    view.setDefinition({ ...definition, sort: { field, dir: same && definition.sort.dir === 'ASC' ? 'DESC' : 'ASC' } })
  }

  const chosen = tickets.filter((ticket) => selection.has(ticket.id))
  function afterWideBulk(result: BulkResult | null) {
    setBulkResult(result)
    if (!result) return
    setSelection(new Set(result.items.filter((item) => !item.ok).map((item) => item.ticketId)))
    void Promise.all([queryClient.invalidateQueries({ queryKey: queryKeys.all.scopeDataset }), queryClient.invalidateQueries({ queryKey: queryKeys.all.ticket })])
  }
  const definitionForBar: ViewDefinition = definition

  return (
    <>
      <ViewBar
        target={target.kind}
        layout={layout}
        isAdmin={false}
        views={view.views}
        activeId={view.active?.id ?? null}
        definition={definitionForBar}
        dirty={view.dirty}
        labels={[]}
        members={people}
        types={[]}
        planning={undefined}
        projects={projects}
        onLayout={view.setLayout}
        onSelect={(id) => {
          setMessage('')
          view.select(id)
        }}
        onChange={view.setDefinition}
        onReset={view.reset}
        onSave={view.saveChanges}
        onSaveAs={view.saveAs}
        onRename={view.rename}
        onDelete={view.remove}
      />
      <div className="board-toolbar">
        <small className="field-hint">{t('work.wide.hint')}</small>
        {shown !== tickets.length ? <small className="field-hint">{t('view.filter.result', { shown: String(shown), total: String(tickets.length) })}</small> : null}
        {mobile && layout === 'LIST' && tickets.length > 0 ? <Button type="button" variant="secondary" aria-pressed={selecting} onClick={() => { setSelecting((on) => !on); setSelection(new Set()); setBulkResult(null) }}>{t('bulk.select')}</Button> : null}
      </div>
      {dataset.data.truncated ? <p className="board-note" role="note">{t('work.wide.truncated')}</p> : null}
      {message ? <p className="board-error" role="alert">{message}</p> : null}
      {layout === 'TABLE' && (chosen.length > 0 || bulkResult) ? (
        <BulkBar
          workspaceId={target.workspaceId}
          selected={chosen}
          result={bulkResult}
          names={new Map(tickets.map((ticket) => [ticket.id, { key: ticket.key, title: ticket.title }]))}
          onClear={() => {
            setSelection(new Set())
            setBulkResult(null)
          }}
          onResult={afterWideBulk}
        />
      ) : null}
      {tickets.length === 0 ? <p className="board-empty">{t('work.wide.empty')}</p> : null}
      {tickets.length > 0 && sections.length === 0 ? <p className="board-empty">{t('work.list.noMatch')}</p> : null}
      <div className="work-scroll">
        {layout === 'LIST' ? (
          <ListLayout
            sections={sections as Section[]}
            selectedId={selectedId}
            compact={definition.display.compact}
            wide
            busy={busy}
            canEdit={(ticket) => editable.get(ticket.projectId) ?? false}
            onOpen={(id) => {
              setMessage('')
              open(id)
            }}
            onState={(ticket, stateId) => void changeState(ticket, stateId)}
            fields={rowFields}
            select={selecting ? { chosen: selection, onToggle: (id) => setSelection((current) => toggleAll(current, [id], !current.has(id))) } : undefined}
          />
        ) : (
          <TableLayout
            sections={sections}
            columns={tableColumns(definition, true)}
            sort={definition.sort}
            selectedId={selectedId}
            selection={selection}
            compact={definition.display.compact}
            busy={busy}
            canEdit={(ticket) => editable.get(ticket.projectId) ?? false}
            onSort={setSort}
            onSelect={(ids, on) => setSelection((current) => toggleAll(current, ids, on))}
            onOpen={(id) => {
              setMessage('')
              open(id)
            }}
            onState={(ticket, stateId) => void changeState(ticket, stateId)}
          />
        )}
      </div>
      {selecting && layout === 'LIST' ? (
        <PhoneBulk
          workspaceId={target.workspaceId}
          selected={chosen}
          result={bulkResult}
          names={new Map(tickets.map((ticket) => [ticket.id, { key: ticket.key, title: ticket.title }]))}
          onAll={() => setSelection((current) => {
            const all = sections.flatMap((section) => section.tickets).filter((ticket) => editable.get(ticket.projectId)).map((ticket) => ticket.id)
            return current.size >= all.length ? new Set() : new Set(all)
          })}
          onDone={() => { setSelecting(false); setSelection(new Set()); setBulkResult(null) }}
          onResult={afterWideBulk}
        />
      ) : null}
    </>
  )
}
