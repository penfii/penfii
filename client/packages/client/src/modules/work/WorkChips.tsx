import { useI18n } from '../../i18n/I18nProvider'
import type { WorkTicket } from './viewModel'

/** Open predecessors keep a Ticket from being completed; once it is complete the same count is only a recheck. */
export function BlockedChip({ ticket }: { ticket: WorkTicket }) {
  const { t } = useI18n()
  const open = ticket.dependencies.openBlockers
  if (open === 0) return null
  const waiting = ticket.stateCategory === 'NOT_STARTED' || ticket.stateCategory === 'IN_PROGRESS'
  return (
    <span className={waiting ? 'blocked-chip' : 'blocked-chip recheck'} title={t(waiting ? 'work.card.blockedHelp' : 'work.card.recheckHelp', { count: String(open) })}>
      {t(waiting ? 'work.card.blocked' : 'work.card.recheck', { count: String(open) })}
    </span>
  )
}
