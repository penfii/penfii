import { forwardRef, useImperativeHandle, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Ticket } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useSession } from '../identity'
import { canDetachAttachment } from '../spaces/capabilities'
import { AttachButton } from '../files/AttachButton'
import { AttachmentList, UploadCards } from '../files/AttachmentList'
import { FileDropZone } from '../files/FileDropZone'
import { useFileUploads } from '../files/useFileUploads'
import { TrashIcon } from '../../design/icons'
import { IconButton, useConfirm } from '../../design/ui'

export type TicketAttachmentHandle = { addFiles: (files: File[]) => void }

/** Files on the Ticket body. Anyone who can open the Ticket sees them; editors add them and remove their own; the Project admin removes any. */
export const TicketAttachments = forwardRef<TicketAttachmentHandle, { ticket: Ticket; canEdit: boolean; canManageProject: boolean }>(function TicketAttachments({ ticket, canEdit, canManageProject }, ref) {
  const { t } = useI18n()
  const session = useSession()
  const queryClient = useQueryClient()
  const attachments = useQuery(queries.ticketAttachments(ticket.id))
  const [removeFailed, setRemoveFailed] = useState(false)
  const { confirm, dialog: confirmDialog } = useConfirm()
  const { uploads, addFiles, retry, remove } = useFileUploads({
    workspaceId: ticket.workspaceId,
    onAvailable: async (fileId) => {
      await api.attachTicketFile(ticket.id, fileId)
      await queryClient.invalidateQueries({ queryKey: queryKeys.ticketAttachments(ticket.id) })
    },
  })
  useImperativeHandle(ref, () => ({ addFiles }), [addFiles])

  async function detach(attachmentId: string, filename: string) {
    if (!(await confirm({ title: t('work.attachments.remove'), body: t('work.attachments.removeConfirm', { name: filename }), confirmLabel: t('work.attachments.remove') }))) return
    setRemoveFailed(false)
    try {
      await api.deleteTicketAttachment(ticket.id, attachmentId)
      await queryClient.invalidateQueries({ queryKey: queryKeys.ticketAttachments(ticket.id) })
    } catch {
      setRemoveFailed(true)
    }
  }

  const items = attachments.data ?? []
  return (
    <section id="ticket-attachments" className="ticket-section ticket-attachments">
      <h3>{t('work.attachments.title')}</h3>
      <FileDropZone onFiles={addFiles} disabled={!canEdit} className="ticket-attachments-body">
        {attachments.isError ? <p className="board-error" role="alert">{t('work.attachments.loadError')}</p> : null}
        {attachments.isSuccess && items.length === 0 && uploads.length === 0 ? (
          <p className="hint">{canEdit ? t('work.attachments.drop') : t('work.attachments.empty')}</p>
        ) : null}
        <AttachmentList
          items={items}
          actions={(item) =>
            canEdit && canDetachAttachment(item, session.data?.id ?? '', canManageProject) ? (
              <IconButton label={`${t('work.attachments.remove')}: ${item.filename}`} className="attachment-action" onClick={() => void detach(item.id, item.filename)}><TrashIcon /></IconButton>
            ) : null
          }
        />
        <UploadCards items={uploads} onRetry={retry} onRemove={remove} />
        {removeFailed ? <p className="board-error" role="alert">{t('work.attachments.removeError')}</p> : null}
        {canEdit ? <div className="attachment-actions"><AttachButton onFiles={addFiles} /></div> : null}
      </FileDropZone>
      {confirmDialog}
    </section>
  )
})
