import { useLayoutEffect, useRef } from 'react'
import { useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import type { Ticket } from '@penfii/contracts'
import { Badge } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { previewDocs } from '../preview/catalog'
import { taskById, type TaskStatus } from '../preview/shellModel'
import { TicketPanel } from '../modules/work/TicketPanel'
import { CreateTicketPanel } from '../modules/work/CreateTicketPanel'
import type { PaneControls } from './ResizeHandle'
import { ChooseProjectPanel } from '../modules/work/ChooseProjectPanel'
import { announceTicket } from '../modules/work/fromChat'
import { CREATE_KEYS, chatSourceFromParams, createContextFromParams, isCreatePanel, ticketIdFromPanel } from '../modules/work/panelParam'

const statusLabel: Record<TaskStatus, 'shell.board.backlog' | 'projects.todo' | 'projects.doing' | 'projects.done'> = {
  backlog: 'shell.board.backlog',
  todo: 'projects.todo',
  doing: 'projects.doing',
  done: 'projects.done',
}

export function DetailPanel({ ticketPane, bindClose }: { ticketPane?: PaneControls; bindClose?: (close: () => void) => void }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const [params, setParams] = useSearchParams()
  const panel = params.get('panel')
  const closeRef = useRef<() => void>(() => {})
  useLayoutEffect(() => {
    bindClose?.(() => closeRef.current())
    return () => bindClose?.(() => {})
  }, [bindClose])
  if (!panel) {
    closeRef.current = () => {}
    return null
  }
  const splitAt = panel.indexOf(':')
  const kind = splitAt === -1 ? panel : panel.slice(0, splitAt)
  const id = splitAt === -1 ? '' : panel.slice(splitAt + 1)

  function close() {
    const next = new URLSearchParams(params)
    next.delete('panel')
    setParams(next)
  }

  const createContext = isCreatePanel(panel) ? createContextFromParams(params) : null
  if (createContext) {
    // Back to the parent Ticket when Create was opened from it, otherwise back to the page.
    const leave = () => {
      const next = new URLSearchParams(params)
      for (const key of CREATE_KEYS) next.delete(key)
      if (createContext.parentId) next.set('panel', panelOpen('ticket', createContext.parentId))
      else next.delete('panel')
      setParams(next)
    }
    const { source } = createContext
    const created = source
      ? async (ticket: Ticket) => {
          // The Ticket exists either way; when the thread cannot be told, the Ticket is shown instead.
          try {
            await announceTicket(source, ticket, workspaceId, t('work.create.announced'))
            // Show the result: the thread now holds the message that links to the Ticket.
            const next = new URLSearchParams(params)
            for (const key of CREATE_KEYS) next.delete(key)
            next.delete('panel')
            const base = location.pathname.replace(/\/t\/[^/]+$/, '')
            const search = next.toString()
            void navigate(`${base}/t/${source.threadRootId}${search ? `?${search}` : ''}`)
          } catch {
            const next = new URLSearchParams(params)
            for (const key of CREATE_KEYS) next.delete(key)
            next.set('panel', panelOpen('ticket', ticket.id))
            setParams(next)
          }
        }
      : leave
    closeRef.current = leave
    return <CreateTicketPanel context={createContext} onClose={leave} onCreated={(ticket) => void created(ticket)} pane={ticketPane} />
  }
  const pendingSource = isCreatePanel(panel) ? chatSourceFromParams(params) : undefined
  if (pendingSource) {
    const leave = () => {
      const next = new URLSearchParams(params)
      for (const key of CREATE_KEYS) next.delete(key)
      next.delete('panel')
      setParams(next)
    }
    closeRef.current = leave
    return (
      <ChooseProjectPanel
        pane={ticketPane}
        onClose={leave}
        onChoose={(projectId) => {
          const next = new URLSearchParams(params)
          next.set('newProject', projectId)
          setParams(next)
        }}
      />
    )
  }
  const ticketId = ticketIdFromPanel(panel)
  if (ticketId) {
    closeRef.current = close
    return (
      <TicketPanel
        ticketId={ticketId}
        onClose={close}
        pane={ticketPane}
        onOpenTicket={(next) => {
          const nextParams = new URLSearchParams(params)
          nextParams.set('panel', panelOpen('ticket', next))
          setParams(nextParams)
        }}
      />
    )
  }

  const task = kind === 'task' ? taskById(id) : undefined
  const doc = kind === 'document' ? previewDocs.find((item) => item.id === id) : undefined

  closeRef.current = close
  return (
    <aside className="detail-panel" aria-label={t('shell.detail.task')}>
      <div className="detail-head">
        <h2>{task ? t('shell.detail.task') : doc ? t('nav.docs') : t('shell.detail.task')}</h2>
        <button type="button" className="btn btn-ghost" onClick={close}>{t('close')}</button>
      </div>
      {task ? (
        <>
          <h3>{task.title}</h3>
          <section>
            <h4>{t('shell.detail.properties')}</h4>
            <p><Badge>{t(statusLabel[task.status])}</Badge></p>
            <p>{task.assignee}</p>
          </section>
          <section>
            <h4>{t('shell.detail.comments')}</h4>
            {task.comments.length === 0 ? <p className="tree-empty">{t('shell.detail.noComments')}</p> : null}
            <ul className="plain-list">
              {task.comments.map((comment) => <li key={comment}>{comment}</li>)}
            </ul>
          </section>
        </>
      ) : null}
      {doc ? (
        <>
          <h3>{t(doc.titleKey)}</h3>
          <p>{t(doc.bodyKey)}</p>
        </>
      ) : null}
      {!task && !doc ? <p className="tree-empty">{t('shell.detail.missing')}</p> : null}
    </aside>
  )
}

export function panelOpen(kind: string, id: string): string {
  return `${kind}:${id}`
}
