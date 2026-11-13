import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Button, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queryKeys } from '../../queryKeys'

/** "Archive" / "Unarchive" on a Ticket, shown to the Project admin. */
export function ArchiveTicketButton({ ticketId, ticketKey, projectId, archived }: { ticketId: string; ticketKey: string; projectId: string; archived: boolean }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { confirm, dialog } = useConfirm()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<MessageKey | null>(null)

  async function run() {
    if (!archived && !(await confirm({ title: t('archive.confirmTitle'), body: t('archive.confirmBody', { key: ticketKey }), confirmLabel: t('archive.button'), tone: 'primary' }))) return
    setBusy(true)
    setError(null)
    try {
      await (archived ? api.unarchiveTicket(ticketId) : api.archiveTicket(ticketId))
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.ticket(ticketId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) }),
        queryClient.invalidateQueries({ queryKey: queryKeys.projectArchive(projectId) }),
      ])
    } catch (cause) {
      const code = isApiError(cause) ? cause.code : ''
      setError(code === 'open_children' ? 'archive.error.open_children' : code === 'ticket_trashed' ? 'archive.error.ticket_trashed' : 'archive.error.generic')
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <Button variant="ghost" disabled={busy} onClick={() => void run()}>{archived ? t('archive.unarchive') : t('archive.button')}</Button>
      {error ? <span className="form-error" role="alert">{t(error)}</span> : null}
      {dialog}
    </>
  )
}
