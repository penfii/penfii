import { useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { useAdminAction } from './useAdminAction'

/** "Move to trash" on a Ticket, shown to the Project admin; the Ticket can be restored from Project settings. */
export function TrashTicketButton({ ticketId, ticketKey, projectId, onDone }: { ticketId: string; ticketKey: string; projectId: string; onDone: () => void }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()

  async function trash() {
    if (!(await confirm({ title: t('admin.trash.move'), body: t('admin.trash.moveConfirm', { key: ticketKey }), confirmLabel: t('admin.trash.move'), tone: 'primary' }))) return
    void perform(
      () => api.trashTicket(ticketId),
      async () => {
        await queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
        await queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) })
        // The open panel is about to close; marking the Ticket stale (not refetching it) avoids asking for a Ticket that is gone.
        await queryClient.invalidateQueries({ queryKey: queryKeys.ticket(ticketId), refetchType: 'none' })
        onDone()
      },
    )
  }

  return (
    <>
      <Button variant="ghost" disabled={busy} onClick={() => void trash()}>{t('admin.trash.move')}</Button>
      {error ? <span className="form-error" role="alert">{error}</span> : null}
      {dialog}
      {confirmDialog}
    </>
  )
}
