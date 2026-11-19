import type { Ticket } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { ArchiveTicketButton } from './ArchiveTicketButton'
import { openChildCount } from './ticketModel'

/**
 * On a Done Task or Epic: how many children are open again. The parent stays Done on purpose (reopening a child never changes
 * its parent); this says so and leaves the decision to reopen the parent to the person.
 */
export function OpenChildrenNote({ summary, short = false }: { summary: Ticket['childSummary']; short?: boolean }) {
  const { t } = useI18n()
  const open = openChildCount(summary)
  if (open === 0) return null
  return <p className="board-note" role="note">{t(short ? 'work.children.openShort' : 'work.children.openNote', { count: String(open) })}</p>
}

/**
 * On a child whose parent is archived: why it cannot be reopened and what to do. Finishing or canceling it stays possible. The
 * way out is the parent's own unarchive, offered to the Project admin; everyone else is told who can do it. Shown also for an
 * older Ticket that is already open below an archived parent, so the contradiction is visible instead of silent.
 */
export function ParentArchivedNote({ ticket, parent, canManage, onOpenTicket }: { ticket: Ticket; parent: Ticket | undefined; canManage: boolean; onOpenTicket: (id: string) => void }) {
  const { t } = useI18n()
  if (!parent?.archivedAt || ticket.archivedAt) return null
  return (
    <div className="board-note parent-archived-note" role="note">
      <p>{t('archive.parentNote', { key: parent.key })}</p>
      {canManage ? null : <p className="hint">{t('archive.parentNoteAsk')}</p>}
      <div className="row-actions">
        <Button type="button" variant="secondary" onClick={() => onOpenTicket(parent.id)}>{t('archive.openParent', { key: parent.key })}</Button>
        {canManage ? <ArchiveTicketButton ticketId={parent.id} ticketKey={parent.key} projectId={parent.projectId} archived /> : null}
      </div>
    </div>
  )
}
