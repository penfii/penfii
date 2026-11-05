import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Link, useParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { Ticket } from '@penfii/contracts'
import { Button, Tabs } from '../../design/ui'
import { ResizeHandle, type PaneControls } from '../../shell/ResizeHandle'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { ActivityTab } from './ActivityTab'
import { CommentComposer, CommentList } from './CommentsTab'
import { useTicketScroll } from './useTicketScroll'
import { ConflictBanner } from './ConflictBanner'
import { DependenciesSection } from './DependenciesSection'
import { HierarchySection } from './HierarchySection'
import { TrashTicketButton } from '../admin/TrashTicketButton'
import { ReferencedBy } from '../docs/ReferencedBy'
import { HistoryTab } from './HistoryTab'
import { ScheduleSection } from './PlanFields'
import { SavedButtons } from '../personal/SavedButtons'
import { WatchButton } from '../attention/WatchButton'
import { TicketPlanning, TicketProperties } from './TicketProperties'
import { useProjectContext } from './useProjectContext'
import type { ProjectContext } from './projectContext'
import { descriptionText, formatDue, isPriority, textToDescription } from './ticketModel'
import { FileDropZone } from '../files/FileDropZone'
import { TicketAttachments, type TicketAttachmentHandle } from './TicketAttachments'
import { useTicketEditor } from './useTicketEditor'
import { CloseIcon } from '../../design/icons'

type TabId = 'comments' | 'activity' | 'history'

/**
 * Ticket detail beside the Board. The Board stays visible and the panel is addressed by the URL,
 * so the back button, a shared link and a refresh all return to the same Ticket.
 */
export function TicketPanel({ ticketId, onClose, onOpenTicket, pane }: { ticketId: string; onClose: () => void; onOpenTicket: (id: string) => void; pane?: PaneControls }) {
  const { t } = useI18n()
  const ticketQuery = useQuery(queries.ticket(ticketId))
  const projectId = ticketQuery.data?.projectId ?? ''
  const { board, context: projectContext } = useProjectContext(projectId)
  const enabled = projectId.length > 0

  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.key === 'Escape' && !event.defaultPrevented) onClose()
  }

  let content
  if (ticketQuery.isPending || (enabled && board.isPending)) {
    content = <p className="hint" role="status">{t('work.panel.loading')}</p>
  } else if (ticketQuery.isError) {
    content = isApiError(ticketQuery.error) && ticketQuery.error.status === 404
      ? <p className="hint" role="alert">{t('work.panel.notFound')}</p>
      : (
        <p className="hint" role="alert">
          {t('work.panel.error')} <Button variant="secondary" onClick={() => void ticketQuery.refetch()}>{t('work.panel.retry')}</Button>
        </p>
      )
  } else if (board.isError) {
    content = (
      <p className="hint" role="alert">
        {t('work.panel.error')} <Button variant="secondary" onClick={() => void board.refetch()}>{t('work.panel.retry')}</Button>
      </p>
    )
  } else if (!projectContext) {
    content = <p className="hint" role="status">{t('work.panel.loading')}</p>
  } else {
    const ctx = projectContext
    content = <TicketBody key={ticketQuery.data.id} ticket={ticketQuery.data} ctx={ctx} onClose={onClose} onOpenTicket={onOpenTicket} />
  }

  return (
    <aside className="detail-panel ticket-panel" aria-label={t('work.panel.label')} onKeyDown={onKeyDown}>
      {pane ? <ResizeHandle pane={pane} label={t('work.panel.resize')} resetLabel={t('layout.resetWidth')} /> : null}
      {ticketQuery.isSuccess ? content : (
        <div className="ticket-scroll">
          <div className="ticket-panel-head">
            <div className="head-cluster">
              <button type="button" className="btn btn-ghost panel-back" onClick={onClose}>← {t('work.panel.back')}</button>
            </div>
            <button type="button" className="icon-button panel-close" aria-label={t('work.panel.close')} onClick={onClose}><CloseIcon /></button>
          </div>
          {content}
        </div>
      )}
    </aside>
  )
}

/** The previous and next Ticket that share this one's parent and kind, in key order. */
function SiblingNav({ ticket, ctx, onOpen }: { ticket: Ticket; ctx: ProjectContext; onOpen: (id: string) => void }) {
  const { t } = useI18n()
  const siblings = ctx.board.columns
    .flatMap((column) => column.tickets)
    .filter((item) => item.parentId === ticket.parentId && item.hierarchyKind === ticket.hierarchyKind)
    .sort((a, b) => a.key.localeCompare(b.key))
  const index = siblings.findIndex((item) => item.id === ticket.id)
  if (index < 0 || siblings.length < 2) return null
  const previous = index > 0 ? siblings[index - 1] : null
  const next = index < siblings.length - 1 ? siblings[index + 1] : null
  return (
    <span className="ticket-siblings">
      <button type="button" className="btn btn-ghost" disabled={!previous} onClick={() => previous && onOpen(previous.id)}>{previous ? t('work.panel.prev', { key: previous.key }) : t('work.panel.prev', { key: '—' })}</button>
      <button type="button" className="btn btn-ghost" disabled={!next} onClick={() => next && onOpen(next.id)}>{next ? t('work.panel.next', { key: next.key }) : t('work.panel.next', { key: '—' })}</button>
    </span>
  )
}

/** The panel's top bar: key (click to copy), State, sibling navigation, watch and saved buttons. */
function TicketHead({ ticket, ctx, workspaceId, busy, stateName, titleHidden, toTop, onStatus, onClose, onOpenTicket }: {
  ticket: Ticket
  ctx: ProjectContext
  workspaceId: string
  busy: boolean
  stateName: string
  titleHidden: boolean
  toTop: () => void
  onStatus: (statusId: string) => void
  onClose: () => void
  onOpenTicket: (id: string) => void
}) {
  const { t } = useI18n()
  const canEdit = ctx.board.canEdit
  const [copied, setCopied] = useState(false)

  async function copyKey() {
    try {
      await navigator.clipboard.writeText(ticket.key)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      // Clipboard access can be refused; the key stays visible to copy by hand.
    }
  }

  return (
    <>
      <header className="ticket-panel-head">
        <div className="head-cluster">
          <button type="button" className="btn btn-ghost panel-back" onClick={onClose}>← {t('work.panel.back')}</button>
          <div className="ticket-id">
            <span className={`kind-chip kind-${ticket.hierarchyKind.toLowerCase()}`}>{t(`work.kind.${ticket.hierarchyKind}`)}</span>
            <button type="button" className="key-button" onClick={() => void copyKey()} title={t('work.panel.keyCopy')}>
              {copied ? t('work.panel.keyCopied') : ticket.key}
            </button>
            <select className="ticket-status" aria-label={t('work.panel.status')} value={ticket.statusId} disabled={!canEdit || busy} onChange={(event) => onStatus(event.target.value)}>
              {ctx.board.states.map((state) => <option key={state.id} value={state.id}>{state.name}</option>)}
            </select>
          </div>
          {titleHidden ? (
            <button type="button" className="compact-title" title={ticket.title} onClick={toTop}>
              <span className="compact-title-text">{ticket.title}</span>
              {stateName ? <span className="state-chip">{stateName}</span> : null}
            </button>
          ) : null}
          <SiblingNav ticket={ticket} ctx={ctx} onOpen={onOpenTicket} />
          <span className="ticket-saved">
            <WatchButton kind="TICKET" targetId={ticket.id} />
            <SavedButtons workspaceId={workspaceId} ticketId={ticket.id} projectId={ticket.projectId} />
          </span>
        </div>
        <button type="button" className="icon-button panel-close" aria-label={t('work.panel.close')} onClick={onClose}><CloseIcon /></button>
      </header>
    </>
  )
}

/** The message shown for a failed edit or status change. */
function failureMessageKey(failure: NonNullable<ReturnType<typeof useTicketEditor>['failure']>) {
  switch (failure) {
    case 'unfinished': return 'work.move.unfinished'
    case 'notAllowed': return 'work.move.notAllowed'
    case 'moveConflict': return 'work.move.conflict'
    case 'blocked': return 'work.move.blocked'
    case 'blockedCanceled': return 'work.move.blockedCanceled'
    case 'move': return 'work.move.error'
    case 'invalidValue': return 'work.panel.invalidValue'
    default: return 'work.panel.saveError'
  }
}

function TicketBody({ ticket, ctx, onClose, onOpenTicket }: { ticket: Ticket; ctx: ProjectContext; onClose: () => void; onOpenTicket: (id: string) => void }) {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const canEdit = ctx.board.canEdit
  const editor = useTicketEditor(ticket)
  const [tab, setTab] = useState<TabId>('comments')
  const [titleDraft, setTitleDraft] = useState(ticket.title)
  const lastTitle = useRef(ticket.title)
  const [discards, setDiscards] = useState(0)
  const [propsOpen, setPropsOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const [commentView, setCommentView] = useState<{ total: number; shown: number } | null>(null)
  const onCommentView = useCallback((view: { total: number; shown: number }) => setCommentView(view), [])
  const sectionIds = ['ticket-description', 'ticket-attachments', 'ticket-children', 'ticket-deps', 'ticket-comments']
  const attachmentsRef = useRef<TicketAttachmentHandle>(null)
  const scroll = useTicketScroll(rootRef, ticket.id, sectionIds)
  const commentCount = useQuery(queries.ticketComments(ticket.id)).data?.length
  const attachmentCount = useQuery(queries.ticketAttachments(ticket.id)).data?.length
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const parentTicket = useQuery({
    ...queries.ticket(ticket.parentId ?? ''),
    enabled: ticket.hierarchyKind === 'SUBTASK' && ticket.parentId !== null,
  })

  // A title changed elsewhere replaces the field only while it has no edit of its own.
  useEffect(() => {
    setTitleDraft((draft) => (draft === lastTitle.current ? ticket.title : draft))
    lastTitle.current = ticket.title
  }, [ticket.title])

  const channel = ticket.channelId ? space.data?.channels.find((item) => item.id === ticket.channelId) : undefined
  const project = space.data?.projects.find((item) => item.id === ticket.projectId)

  function commitTitle() {
    const value = titleDraft.trim()
    if (!canEdit || !value || value === ticket.title) {
      if (!value) setTitleDraft(ticket.title)
      return
    }
    void editor.save({ title: value })
  }

  const stateName = ctx.board.states.find((state) => state.id === ticket.statusId)?.name ?? ''
  const jump = (id: string) => document.getElementById(id)?.scrollIntoView?.({ block: 'start' })
  const failureText = editor.failure ? t(failureMessageKey(editor.failure)) : ''

  return (
    <>
      <TicketHead
        ticket={ticket}
        ctx={ctx}
        workspaceId={workspaceId}
        busy={editor.busy}
        stateName={stateName}
        titleHidden={scroll.titleHidden}
        toTop={scroll.toTop}
        onStatus={(statusId) => void editor.changeStatus(statusId)}
        onClose={onClose}
        onOpenTicket={onOpenTicket}
      />
      <nav className="section-chips" aria-label={t('work.panel.sections')}>
        {[
          { id: 'ticket-description', label: t('work.field.description'), count: null },
          { id: 'ticket-attachments', label: t('work.attachments.title'), count: attachmentCount ?? null },
          ...(ticket.hierarchyKind === 'SUBTASK' ? [] : [{ id: 'ticket-children', label: t(ticket.hierarchyKind === 'EPIC' ? 'work.children.title.EPIC' : 'work.children.title.TASK'), count: ticket.childSummary.total }]),
          { id: 'ticket-deps', label: t('work.dep.title'), count: ticket.dependencies.blockedBy + ticket.dependencies.blocks },
          { id: 'ticket-comments', label: t('work.tab.comments'), count: commentCount ?? null },
        ].map((chip) => (
          <button key={chip.id} type="button" className={scroll.active === chip.id ? 'active' : undefined} aria-current={scroll.active === chip.id ? 'true' : undefined} onClick={() => jump(chip.id)}>
            {chip.label}{chip.count ? <span className="chip-count"> {chip.count}</span> : null}
          </button>
        ))}
      </nav>
      <div className="ticket-scroll">
      <div className="ticket-body" ref={rootRef} tabIndex={-1}>
      <div className="ticket-title-block">
      {!canEdit ? <p className="board-note" role="note">{t('work.panel.readOnly')}</p> : null}
      {editor.conflict ? (
        <ConflictBanner conflict={editor.conflict} ticket={ticket} ctx={ctx} busy={editor.busy} onReapply={() => void editor.reapply()} onDiscard={() => {
          setTitleDraft(ticket.title)
          setDiscards((count) => count + 1)
          editor.discard()
        }} />
      ) : null}
      {failureText ? <p className="board-error" role="alert">{failureText}</p> : null}

      {canEdit ? (
        <input
          className="ticket-title-input"
          aria-label={t('work.field.title')}
          value={titleDraft}
          maxLength={300}
          onChange={(event) => setTitleDraft(event.target.value)}
          onBlur={commitTitle}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              event.preventDefault()
              event.currentTarget.blur()
            } else if (event.key === 'Escape' && titleDraft !== ticket.title) {
              event.preventDefault()
              setTitleDraft(ticket.title)
            }
          }}
        />
      ) : (
        <h2 className="ticket-title">{ticket.title}</h2>
      )}
      </div>

      <div className="ticket-columns">
        <div className={propsOpen ? 'ticket-side props-open' : 'ticket-side'}>
          <div className="prop-summary">
            {stateName ? <span className="state-chip">{stateName}</span> : null}
            {ticket.assigneeName ? <span>{ticket.assigneeName}</span> : null}
            {isPriority(ticket.priority) ? <span className={`priority-chip priority-${ticket.priority.toLowerCase()}`}>{t(`work.priority.${ticket.priority}`)}</span> : null}
            {ticket.dueOn ? <span className="ticket-due">{formatDue(ticket.dueOn, locale)}</span> : null}
            <button type="button" aria-expanded={propsOpen} onClick={() => setPropsOpen((open) => !open)}>{t('work.panel.allProps')}</button>
          </div>
          <TicketProperties ticket={ticket} ctx={ctx} canEdit={canEdit} busy={editor.busy} onSave={(patch) => void editor.save(patch)} onStatus={(statusId) => void editor.changeStatus(statusId)} />
          <div className="side-extra">
            <TicketPlanning ticket={ticket} ctx={ctx} canEdit={canEdit} busy={editor.busy} onPlan={(action) => void editor.perform(action)} />
            <ScheduleSection ticket={ticket} workspaceId={workspaceId} canManageProject={ctx.board.canManageWorkflow} canEdit={canEdit} busy={editor.busy} save={editor.save} />
          </div>
        </div>
        <div className="ticket-main">
          <FileDropZone className="ticket-file-surface" disabled={!canEdit} onFiles={(files) => attachmentsRef.current?.addFiles(files)}>
            <div id="ticket-description">
              <DescriptionSection key={`description-${String(discards)}`} ticket={ticket} canEdit={canEdit} busy={editor.busy} save={editor.save} conflicting={editor.conflict !== null} />
            </div>
            <TicketAttachments ref={attachmentsRef} ticket={ticket} canEdit={canEdit} canManageProject={ctx.board.canManageWorkflow} />
          </FileDropZone>
          <div id="ticket-children">
            <HierarchySection
              ticket={ticket}
              parent={parentTicket.data}
              ctx={ctx}
              canEdit={canEdit}
              busy={editor.busy}
              onSave={(patch) => void editor.save(patch)}
              onOpen={onOpenTicket}
            />
          </div>
          <div id="ticket-deps">
            <DependenciesSection ticket={ticket} ctx={ctx} onOpen={onOpenTicket} />
          </div>
          <ReferencedBy ticketId={ticket.id} workspaceId={workspaceId} />
          <section className="ticket-section" id="ticket-comments">
            <Tabs
              tabs={[
                { id: 'comments', label: t('work.tab.comments') },
                { id: 'activity', label: t('work.tab.activity') },
                { id: 'history', label: t('work.tab.history') },
              ]}
              value={tab}
              onChange={(id) => {
                setTab(id as TabId)
                window.requestAnimationFrame(() => jump('ticket-comments'))
              }}
            />
            <div className="tab-body" role="tabpanel">
              {tab === 'comments' ? <CommentList ticketId={ticket.id} workspaceId={ticket.workspaceId} onView={onCommentView} /> : null}
              {tab === 'activity' ? <ActivityTab ticketId={ticket.id} /> : null}
              {tab === 'history' ? <HistoryTab ticket={ticket} canEdit={canEdit} /> : null}
            </div>
          </section>
          <footer className="ticket-meta-foot">
            {project ? <Link to={`/w/${workspaceId}/projects/${project.id}/work`}>{project.name}</Link> : null}
            <span>{channel ? `#${channel.name}` : t('work.channel.project')}</span>
            <span>{t('work.field.created')} {formatStamp(ticket.createdAt, locale)}</span>
            {ctx.board.canManageWorkflow ? <TrashTicketButton ticketId={ticket.id} ticketKey={ticket.key} projectId={ticket.projectId} onDone={onClose} /> : null}
          </footer>
        </div>
      </div>
      </div>
    </div>
      {scroll.scrolled || (tab === 'comments' && commentView && commentView.shown < commentView.total) ? (
        <div className="scroll-hint">
          {tab === 'comments' && commentView && commentView.shown < commentView.total ? <span>{t('work.comments.viewing', { total: String(commentView.total), shown: String(commentView.shown) })}</span> : <span />}
          {scroll.scrolled ? <button type="button" className="to-top" onClick={scroll.toTop}>↑ {t('work.panel.toTop')}</button> : null}
        </div>
      ) : null}
      {tab === 'comments' ? <CommentComposer ticketId={ticket.id} workspaceId={ticket.workspaceId} /> : null}
    </>
  )
}

function DescriptionSection({
  ticket,
  canEdit,
  busy,
  save,
  conflicting,
}: {
  ticket: Ticket
  canEdit: boolean
  busy: boolean
  save: (patch: { description: unknown }) => Promise<boolean>
  conflicting: boolean
}) {
  const { t } = useI18n()
  const text = descriptionText(ticket.description)
  const [draft, setDraft] = useState<string | null>(null)
  const [expanded, setExpanded] = useState(false)
  // Kept open while a conflict is unresolved so what was typed is never lost.
  const editing = draft !== null

  // Once the saved description matches what was typed, the edit is finished.
  useEffect(() => {
    setDraft((current) => (current !== null && current.trim() === text.trim() ? null : current))
  }, [text])

  async function commit() {
    if (draft === null) return
    if (draft.trim() === text.trim()) {
      setDraft(null)
      return
    }
    const ok = await save({ description: textToDescription(draft) })
    if (ok) setDraft(null)
  }

  return (
    <section className="ticket-section description">
      <h3>{t('work.field.description')}</h3>
      {editing ? (
        <div className="description-edit">
          <textarea
            autoFocus
            rows={6}
            aria-label={t('work.field.description')}
            placeholder={t('work.description.placeholder')}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault()
                event.stopPropagation()
                if (!conflicting) setDraft(null)
              } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) {
                event.preventDefault()
                void commit()
              }
            }}
          />
          <div className="dialog-actions">
            <Button type="button" variant="secondary" disabled={busy} onClick={() => setDraft(null)}>{t('work.description.cancel')}</Button>
            <Button type="button" disabled={busy} onClick={() => void commit()}>{t('work.description.save')}</Button>
          </div>
        </div>
      ) : canEdit ? (
        <>
          <button type="button" className={`${text ? 'description-view editable' : 'description-view editable empty'}${expanded ? '' : ' clamped'}`} aria-label={t('work.description.edit')} onClick={() => setDraft(text)}>
            {text || t('work.description.placeholder')}
          </button>
          {text.split('\n').length > 8 || text.length > 600 ? <button type="button" className="more-button" aria-expanded={expanded} onClick={() => setExpanded((open) => !open)}>{expanded ? t('work.less') : t('work.more')}</button> : null}
        </>
      ) : (
        <>
          <p className={`${text ? 'description-view' : 'description-view empty'}${expanded ? '' : ' clamped'}`}>{text || t('work.value.empty')}</p>
          {text.split('\n').length > 8 || text.length > 600 ? <button type="button" className="more-button" aria-expanded={expanded} onClick={() => setExpanded((open) => !open)}>{expanded ? t('work.less') : t('work.more')}</button> : null}
        </>
      )}
    </section>
  )
}
