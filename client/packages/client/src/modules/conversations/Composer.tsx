import { useEffect, useRef, useState } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { messageBodyBytes, plainText, RichTextEditor, type RichTextEditorHandle } from '../../richtext'
import { useI18n } from '../../i18n/I18nProvider'
import { EmojiPicker } from '../emoji/EmojiPicker'
import { AttachButton } from '../files/AttachButton'
import { FileDropZone } from '../files/FileDropZone'
import { UploadCards } from '../files/AttachmentList'
import { useFileUploads } from '../files/useFileUploads'

export function Composer({
  value,
  onChange,
  onSend,
  messageBodyMaxBytes,
  busy,
  label,
  placeholder,
  sendLabel,
  workspaceId,
  focusToken = null,
}: {
  value: RichTextDoc
  onChange: (value: RichTextDoc) => void
  onSend: (fileIds: string[]) => Promise<boolean>
  messageBodyMaxBytes: number | null | undefined
  busy: boolean
  label: string
  placeholder?: string
  sendLabel?: string
  workspaceId: string
  /** Changes when this composer becomes the one the person should type into. Null leaves focus alone. */
  focusToken?: string | null
}) {
  const { t } = useI18n()
  const editorRef = useRef<RichTextEditorHandle>(null)
  const emojiRef = useRef<HTMLDivElement>(null)
  const [emojiOpen, setEmojiOpen] = useState(false)
  const { uploads, readyIds, failed, blocking, addFiles, retry, remove, settle } = useFileUploads({ workspaceId, busy })
  useEffect(() => {
    if (focusToken == null) return
    // A dialog (the viewer, a menu) owns the keyboard until it closes.
    const blocked = () => document.activeElement instanceof HTMLElement && !!document.activeElement.closest('[role="dialog"], [role="menu"]')
    if (blocked()) return
    if (editorRef.current?.focus()) return
    const frame = window.requestAnimationFrame(() => {
      if (!blocked()) editorRef.current?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [focusToken])
  const bytes = messageBodyBytes(value)
  const blank = plainText(value).trim().length === 0
  const limitMissing = messageBodyMaxBytes === null
  const limitLoading = messageBodyMaxBytes === undefined
  const tooLong = typeof messageBodyMaxBytes === 'number' && bytes > messageBodyMaxBytes
  const canSend = !busy && !blocking && !limitMissing && !limitLoading && !tooLong && (!blank || readyIds.length > 0)

  useEffect(() => {
    if (!emojiOpen) return
    const onPointer = (event: MouseEvent) => {
      if (!emojiRef.current?.contains(event.target as Node)) setEmojiOpen(false)
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setEmojiOpen(false)
    }
    window.addEventListener('mousedown', onPointer)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onPointer)
      window.removeEventListener('keydown', onKey)
    }
  }, [emojiOpen])

  let notice: string | null = null
  if (limitLoading) notice = t('composer.loadingLimit')
  else if (limitMissing) notice = t('composer.limitMissing')
  else if (tooLong) notice = t('composer.tooLong')
  else if (failed) notice = t('composer.failedBlocks')

  async function send() {
    if (!canSend) return
    const sent = await onSend(readyIds)
    if (!sent) return
    settle()
  }

  return (
    <FileDropZone className="composer" onFiles={addFiles} disabled={busy}>
      <RichTextEditor
        ref={editorRef}
        value={value}
        onChange={onChange}
        onSend={() => { void send() }}
        onFiles={addFiles}
        label={label}
        placeholder={placeholder ?? t('composer.placeholder')}
      />
      <div className="composer-extra">
        <UploadCards items={uploads} busy={busy} onRetry={retry} onRemove={remove} />
        {notice ? <p className="limit-note" role={failed ? 'alert' : undefined}>{notice}</p> : null}
      </div>
      <div className="composer-footer">
        <div className="composer-slots">
          <div className="popover" ref={emojiRef}>
            <button type="button" aria-expanded={emojiOpen} aria-label={t('composer.emoji')} onClick={() => setEmojiOpen((open) => !open)}>
              😊
            </button>
            {emojiOpen ? <EmojiPicker onPick={(emoji) => editorRef.current?.insertText(emoji)} /> : null}
          </div>
          <AttachButton onFiles={addFiles} disabled={busy} />
        </div>
        <button type="button" disabled={!canSend} onClick={() => void send()}>
          {sendLabel ?? t('composer.send')}
        </button>
      </div>
    </FileDropZone>
  )
}
