import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { useI18n } from '../../i18n/I18nProvider'
import { CloseIcon, DownloadIcon } from '../../design/icons'
import { IconButton } from '../../design/ui'
import { useSheetBack } from '../../design/sheetHistory'
import { CsvTableView, useCsv } from './CsvPreview'

/** One thing the viewer can show. Images and videos can be paged through; a table opens alone. */
export type ViewerItem = {
  kind: 'image' | 'video' | 'csv'
  src: string
  name: string
}

type Open = { items: ViewerItem[]; index: number }

let current: Open | null = null
const listeners = new Set<(open: Open | null) => void>()

function publish(open: Open | null) {
  current = open
  for (const listener of listeners) listener(open)
}

export function attachmentContentUrl(attachmentId: string): string {
  return `/v1/attachments/${attachmentId}/content`
}

export function openMediaViewer(items: ViewerItem[], index = 0) {
  if (items.length > 0) publish({ items, index: Math.min(Math.max(index, 0), items.length - 1) })
}

export function closeMediaViewer() {
  publish(null)
}

/** The one viewer of the app: images, video and the full table of a CSV file. */
export function MediaViewerHost() {
  const { t } = useI18n()
  const [open, setOpen] = useState<Open | null>(current)
  const [videoFailed, setVideoFailed] = useState<string | null>(null)
  // On a phone Back closes the viewer before it leaves the screen, like a sheet.
  useSheetBack(open !== null, closeMediaViewer)

  useEffect(() => {
    listeners.add(setOpen)
    return () => {
      listeners.delete(setOpen)
    }
  }, [])

  const total = open?.items.length ?? 0
  const index = open?.index ?? 0
  function step(by: number) {
    if (!open || total < 2) return
    publish({ items: open.items, index: (open.index + by + total) % total })
  }

  useEffect(() => {
    if (!open) return
    const previousOverflow = document.body.style.overflow
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    document.body.style.overflow = 'hidden'
    if (!(document.activeElement instanceof HTMLElement && document.activeElement.closest('.media-viewer'))) {
      document.querySelector<HTMLElement>('.media-viewer .media-viewer-bar button')?.focus()
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        closeMediaViewer()
      } else if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight') && !(event.target instanceof HTMLMediaElement)) {
        event.preventDefault()
        step(event.key === 'ArrowLeft' ? -1 : 1)
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      document.body.style.overflow = previousOverflow
      window.removeEventListener('keydown', onKey, true)
      previousFocus?.focus()
    }
    // `step` reads the latest `open`, which is what this effect is keyed on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  if (!open) return null
  const item = open.items[open.index]
  if (!item) return null
  return createPortal(
    <div
      className={`image-viewer media-viewer kind-${item.kind}`}
      role="dialog"
      aria-modal="true"
      aria-label={item.kind === 'csv' ? t('composer.csvTitle') : t('composer.viewer')}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) closeMediaViewer()
      }}
    >
      <header className="media-viewer-bar" onMouseDown={(event) => event.stopPropagation()}>
        <strong title={item.name}>{item.name}</strong>
        {total > 1 ? <span className="media-viewer-count">{index + 1} / {total}</span> : null}
        <a className="attachment-action" href={item.src} download={item.name} aria-label={`${t('composer.download')}: ${item.name}`} title={t('composer.download')}><DownloadIcon /></a>
        <IconButton label={t('composer.closePreview')} onClick={() => closeMediaViewer()}><CloseIcon /></IconButton>
      </header>
      {total > 1 ? (
        <button type="button" className="media-viewer-step prev" aria-label={t('composer.prev')} onMouseDown={(event) => event.stopPropagation()} onClick={() => step(-1)}>‹</button>
      ) : null}
      <div className="media-viewer-stage" onMouseDown={(event) => { if (event.target === event.currentTarget) closeMediaViewer() }}>
        {item.kind === 'image' ? (
          <img key={item.src} src={item.src} alt={item.name} onMouseDown={(event) => event.stopPropagation()} />
        ) : null}
        {item.kind === 'video' ? (
          videoFailed === item.src ? (
            <p className="media-viewer-note" onMouseDown={(event) => event.stopPropagation()}>{t('composer.videoUnavailable')}</p>
          ) : (
            <video
              key={item.src}
              src={item.src}
              controls
              preload="metadata"
              playsInline
              onError={() => setVideoFailed(item.src)}
              onMouseDown={(event) => event.stopPropagation()}
            />
          )
        ) : null}
        {item.kind === 'csv' ? <CsvFull src={item.src} /> : null}
      </div>
      {total > 1 ? (
        <button type="button" className="media-viewer-step next" aria-label={t('composer.next')} onMouseDown={(event) => event.stopPropagation()} onClick={() => step(1)}>›</button>
      ) : null}
    </div>,
    document.body,
  )
}

function CsvFull({ src }: { src: string }) {
  const { t } = useI18n()
  const csv = useCsv(src, { bytes: 5 * 1024 * 1024, rows: 1000, columns: 50 })
  return (
    <div className="csv-full" onMouseDown={(event) => event.stopPropagation()}>
      {csv.state === 'loading' ? <p className="media-viewer-note">{t('loading')}</p> : null}
      {csv.state === 'failed' ? <p className="media-viewer-note" role="alert">{t('composer.csvLoadFailed')}</p> : null}
      {csv.state === 'ready' ? (
        <>
          <CsvTableView table={csv.table} />
          {csv.table.truncated ? <p className="csv-note">{t('composer.csvTruncated')}</p> : null}
        </>
      ) : null}
    </div>
  )
}
