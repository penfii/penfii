import { useEffect, useMemo, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { BlockNoteEditor } from '@blocknote/core'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { formatSize } from '../files/AttachmentList'
import { DocUploads, docUploadsExtension, type DocUploadLabels } from './docUploads'

/**
 * The upload state of one Document editor: the `uploadFile` and extension to give BlockNote, and a
 * holder the editor is put into so a cancelled upload can remove its block.
 */
export function useDocUploads(workspaceId: string, attach: (fileId: string) => Promise<void>) {
  const { t } = useI18n()
  const limit = useQuery(queries.serverInfo()).data?.limits.fileMaxBytes ?? null
  const limitRef = useRef(limit)
  limitRef.current = limit
  const attachRef = useRef(attach)
  attachRef.current = attach
  const holder = useRef<BlockNoteEditor<any, any, any> | null>(null)
  const labels: DocUploadLabels = {
    uploading: t('composer.uploading'),
    cancel: t('composer.cancelUpload'),
    retry: t('composer.retry'),
    remove: t('composer.removeFile'),
    tooLarge: (bytes) => t('composer.uploadTooLarge', { limit: formatSize(bytes) }),
    rejected: t('composer.uploadRejected'),
    failed: t('composer.uploadFailed'),
  }
  const labelsRef = useRef(labels)
  labelsRef.current = labels
  const uploads = useMemo(
    () =>
      new DocUploads({
        workspaceId,
        attach: (fileId) => attachRef.current(fileId),
        limit: () => limitRef.current,
        removeBlock: (blockId) => {
          const editor = holder.current
          if (editor?.getBlock(blockId)) editor.removeBlocks([blockId])
        },
        setUrl: (blockId, url) => {
          const editor = holder.current
          if (editor?.getBlock(blockId)) editor.updateBlock(blockId, { props: { url } } as never)
        },
      }),
    [workspaceId],
  )
  const extension = useMemo(() => docUploadsExtension(uploads, () => labelsRef.current), [uploads])
  useEffect(() => () => uploads.dispose(), [uploads])
  return { uploads, extension, holder }
}
