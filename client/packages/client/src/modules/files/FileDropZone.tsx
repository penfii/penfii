import { useRef, useState, type ClipboardEvent, type DragEvent, type ReactNode } from 'react'
import { useI18n } from '../../i18n/I18nProvider'

function hasFiles(types: readonly string[] | undefined): boolean {
  return [...(types ?? [])].includes('Files')
}

/** BlockNote takes a drop or paste on its own editor and marks it handled only after its own listener.
 * The surrounding drop surface would otherwise insert the same file a second time. */
function takenByEditor(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest('.bn-editor')
}

/**
 * The writing surface as a drop target: files dropped or pasted anywhere inside it go to `onFiles`,
 * and while files are dragged over it an overlay says so. An editor that takes files itself (the
 * rich text editor) marks the event handled, and it is not taken twice.
 */
export function FileDropZone({
  onFiles,
  disabled = false,
  className = '',
  children,
}: {
  onFiles: (files: File[]) => void
  disabled?: boolean
  className?: string
  children: ReactNode
}) {
  const { t } = useI18n()
  const [over, setOver] = useState(false)
  const depth = useRef(0)

  function enter(event: DragEvent) {
    if (disabled || !hasFiles(event.dataTransfer.types)) return
    event.preventDefault()
    depth.current += 1
    setOver(true)
  }

  function leave() {
    depth.current = Math.max(0, depth.current - 1)
    if (depth.current === 0) setOver(false)
  }

  function drop(event: DragEvent) {
    if (disabled || !hasFiles(event.dataTransfer.types)) return
    depth.current = 0
    setOver(false)
    // The document editor inserts the block itself. A drop on it is not also added at the cursor.
    if (event.defaultPrevented || takenByEditor(event.target)) return
    event.preventDefault()
    onFiles([...event.dataTransfer.files])
  }

  function paste(event: ClipboardEvent) {
    const files = [...(event.clipboardData.files ?? [])]
    if (disabled || files.length === 0 || event.defaultPrevented || takenByEditor(event.target)) return
    event.preventDefault()
    onFiles(files)
  }

  return (
    <div
      className={`${className} file-drop${over ? ' over' : ''}`.trim()}
      onDragEnter={enter}
      onDragOver={(event) => {
        if (!disabled && hasFiles(event.dataTransfer.types)) event.preventDefault()
      }}
      onDragLeave={leave}
      onDrop={drop}
      onPaste={paste}
    >
      {children}
      {over ? (
        <div className="file-drop-overlay" aria-hidden="true">
          <strong>{t('composer.dropHere')}</strong>
          <span>{t('composer.dropHint')}</span>
        </div>
      ) : null}
    </div>
  )
}
