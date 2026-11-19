import { useLayoutEffect, useRef } from 'react'
import { useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import type { Ticket } from '@penfii/contracts'
import { useI18n } from '../i18n/I18nProvider'
import { notify } from '../design/notices'
import { TicketPanel } from '../modules/work/TicketPanel'
import { CreateTicketPanel } from '../modules/work/CreateTicketPanel'
import type { PaneControls } from './ResizeHandle'
import { ChooseProjectPanel } from '../modules/work/ChooseProjectPanel'
import { announceTicket } from '../modules/work/fromChat'
import { useLeaveOverlay } from './overlayTrail'
import { CREATE_KEYS, chatSourceFromParams, createContextFromParams, isCreatePanel, ticketIdFromPanel } from '../modules/work/panelParam'

export function DetailPanel({ ticketPane, bindClose }: { ticketPane?: PaneControls; bindClose?: (close: () => void) => void }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const [params, setParams] = useSearchParams()
  const leaveTo = useLeaveOverlay()
  // Close by going to the same page with these query keys; on a phone this steps back when it can (see overlayTrail).
  const leaveWith = (next: URLSearchParams) => leaveTo(location.pathname + (next.toString() ? `?${next.toString()}` : ''))
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
  function close() {
    const next = new URLSearchParams(params)
    next.delete('panel')
    leaveWith(next)
  }

  const createContext = isCreatePanel(panel) ? createContextFromParams(params) : null
  if (createContext) {
    // Back to the parent Ticket when Create was opened from it, otherwise back to the page.
    const leave = () => {
      const next = new URLSearchParams(params)
      for (const key of CREATE_KEYS) next.delete(key)
      if (createContext.parentId) next.set('panel', panelOpen('ticket', createContext.parentId))
      else next.delete('panel')
      leaveWith(next)
    }
    const { source } = createContext
    // The panel the URL asks for when `ticketId` is opened from where the person is now (a notice can be pressed long after).
    const ticketHref = (ticketId: string) => {
      const next = new URLSearchParams(window.location.search)
      for (const key of CREATE_KEYS) next.delete(key)
      next.set('panel', panelOpen('ticket', ticketId))
      return `${window.location.pathname}?${next.toString()}`
    }
    // The new Ticket in place of the form: replaced, so Back does not step into the finished form again.
    const openMade = (ticket: Ticket) => {
      const next = new URLSearchParams(params)
      for (const key of CREATE_KEYS) next.delete(key)
      next.set('panel', panelOpen('ticket', ticket.id))
      setParams(next, { replace: true })
    }
    const announceCreated = (ticket: Ticket, label: string) =>
      notify({ message: t('work.create.created', { key: ticket.key }), action: { label, run: () => void navigate(ticketHref(ticket.id)) } })
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
            // The conversation keeps its place; the new Ticket is one press away.
            announceCreated(ticket, t('work.create.openWork'))
          } catch {
            const next = new URLSearchParams(params)
            for (const key of CREATE_KEYS) next.delete(key)
            next.set('panel', panelOpen('ticket', ticket.id))
            setParams(next)
          }
        }
      : (ticket: Ticket) => {
          if (createContext.parentId) {
            // Back to the parent Ticket, with the new one a press away.
            leave()
            announceCreated(ticket, t('work.create.open'))
          } else {
            // From a Project screen the new Ticket opens at once.
            openMade(ticket)
          }
        }
    closeRef.current = leave
    return <CreateTicketPanel context={createContext} onClose={leave} onCreated={(ticket) => void created(ticket)} onOpenTicket={openMade} pane={ticketPane} />
  }
  const pendingSource = isCreatePanel(panel) ? chatSourceFromParams(params) : undefined
  if (pendingSource) {
    const leave = () => {
      const next = new URLSearchParams(params)
      for (const key of CREATE_KEYS) next.delete(key)
      next.delete('panel')
      leaveWith(next)
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

  // Nothing else opens here: a panel kind this build does not know.
  closeRef.current = close
  return (
    <aside className="detail-panel" aria-label={t('shell.detail.task')}>
      <div className="detail-head">
        <h2>{t('shell.detail.task')}</h2>
        <button type="button" className="btn btn-ghost" onClick={close}>{t('close')}</button>
      </div>
      <p className="tree-empty">{t('shell.detail.missing')}</p>
    </aside>
  )
}

export function panelOpen(kind: string, id: string): string {
  return `${kind}:${id}`
}
