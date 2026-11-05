import type { MessageAttachment, RichTextDoc } from '@penfii/contracts'
import { useParams } from 'react-router'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { RichTextView, type DisplayNames } from '../../richtext'
import { AttachmentList } from '../files/AttachmentList'
import { IconButton } from '../../design/ui'
import { BookmarkIcon } from '../../design/icons'

export function MessageBody({
  message,
  names,
}: {
  message: { deleted: boolean; body: RichTextDoc | null; attachments?: MessageAttachment[] }
  names?: DisplayNames
}) {
  const { t } = useI18n()
  if (message.deleted) {
    return (
      <p className="deleted" data-message-state="deleted">
        {t('message.deleted')}
      </p>
    )
  }
  const attachments = message.attachments ?? []
  if (!message.body && attachments.length === 0) return null
  return (
    <>
      {message.body ? <RichTextView doc={message.body} names={names} /> : null}
      <MessageAttachments items={attachments} />
    </>
  )
}

function MessageAttachments({ items }: { items: MessageAttachment[] }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  async function saveFile(fileId: string) {
    try {
      await api.addLater(workspaceId, fileId, null, 'FILE')
    } catch {
      // Saving for later is a convenience; the file itself is untouched either way.
    }
  }
  return (
    <AttachmentList
      items={items}
      actions={(item) =>
        item.state === 'AVAILABLE' ? (
          <IconButton label={`${t('saved.later.addFile')}: ${item.filename}`} className="attachment-action" onClick={() => void saveFile(item.fileId)}><BookmarkIcon /></IconButton>
        ) : null
      }
    />
  )
}
