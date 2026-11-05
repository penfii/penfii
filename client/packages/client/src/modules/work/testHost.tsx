import { useSearchParams } from 'react-router'
import { CreateTicketPanel } from './CreateTicketPanel'
import { createContextFromParams, isCreatePanel, useTicketPanel } from './panelParam'
import { TicketPanel } from './TicketPanel'

/** Test stand-in for the shell's detail slot: renders the panel the URL asks for. */
export function DetailPanelHost() {
  const { selectedId, close, open } = useTicketPanel()
  const [params] = useSearchParams()
  const create = isCreatePanel(params.get('panel')) ? createContextFromParams(params) : null
  if (create) return <CreateTicketPanel context={create} onClose={close} onCreated={close} />
  return selectedId ? <TicketPanel ticketId={selectedId} onClose={close} onOpenTicket={open} /> : null
}
