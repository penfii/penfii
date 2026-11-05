import { useRef, useState, type ReactNode } from 'react'
import type { MessageAttachment } from '@penfii/contracts'
import { Button, IconButton } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { attachmentContentUrl, openMediaViewer, type ViewerItem } from './mediaViewer'
import { CsvPreview } from './CsvPreview'
import { previewKind } from './kind'
import type { UploadItem } from './useFileUploads'
import { CloseIcon, DownloadIcon, ExpandIcon, FileIcon, PlayIcon } from '../../design/icons'

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function statusLabel(state: string): MessageKey {
  if (state === 'UPLOADING') return 'composer.uploading'
  if (state === 'QUARANTINED') return 'composer.quarantined'
  if (state === 'REJECTED') return 'composer.rejected'
  return 'composer.imageUnavailable'
}

function clock(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds))
  return `${String(Math.floor(whole / 60))}:${String(whole % 60).padStart(2, '0')}`
}

/**
 * The files attached to a message, a Ticket or a comment, shown by what they are: images inline
 * (several in a grid) opening the viewer, a video as a poster that plays in place, audio with a
 * player, the first rows of a CSV file, and anything else as a compact card. Every file has a
 * download link. `actions` adds buttons of the screen's own (save for later, remove).
 */
export function AttachmentList<T extends MessageAttachment>({
  items,
  actions,
}: {
  items: T[]
  actions?: (item: T) => ReactNode
}) {
  const { t } = useI18n()
  const [failed, setFailed] = useState<ReadonlySet<string>>(new Set())
  if (items.length === 0) return null
  const fail = (id: string) => setFailed((current) => new Set(current).add(id))
  const kindOf = (item: T) => (item.state === 'AVAILABLE' && !failed.has(item.id) ? previewKind(item.mime) : 'file')
  // Images and videos of this list can be paged through in the viewer, in the order they appear.
  const viewable: ViewerItem[] = items.flatMap((item) => {
    const kind = kindOf(item)
    return kind === 'image' || kind === 'video' ? [{ kind, src: attachmentContentUrl(item.id), name: item.filename }] : []
  })
  const open = (item: T) => openMediaViewer(viewable, Math.max(0, viewable.findIndex((entry) => entry.src === attachmentContentUrl(item.id))))
  const images = items.filter((item) => kindOf(item) === 'image')
  const rest = items.filter((item) => kindOf(item) !== 'image')
  return (
    <div className="attachments">
      {images.length > 0 ? (
        <ul className={`attachment-grid${images.length > 1 ? ' multi' : ''}`}>
          {images.map((item) => (
            <li key={item.id} className="attachment-card image">
              <button type="button" className="attachment-preview" aria-label={`${t('composer.zoom')}: ${item.filename}`} onClick={() => open(item)}>
                <img className="attachment-thumb" src={attachmentContentUrl(item.id)} alt={item.filename} onError={() => fail(item.id)} />
              </button>
              {actions || images.length === 1 ? (
                <div className="attachment-tools">
                  <span className="attachment-caption" title={item.filename}>{images.length === 1 ? item.filename : ''}{images.length === 1 ? ' · ' : ''}{formatSize(item.byteSize)}</span>
                  {actions?.(item)}
                  <a className="attachment-action" href={attachmentContentUrl(item.id)} download={item.filename} aria-label={`${t('composer.download')}: ${item.filename}`} title={t('composer.download')}><DownloadIcon /></a>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      {rest.length > 0 ? (
        <ul className="attachment-list">
          {rest.map((item) => {
            const href = attachmentContentUrl(item.id)
            const kind = kindOf(item)
            const unavailable = item.state !== 'AVAILABLE'
            const download = item.state === 'AVAILABLE' ? (
              <a className="attachment-action" href={href} download={item.filename} aria-label={`${t('composer.download')}: ${item.filename}`} title={t('composer.download')}><DownloadIcon /></a>
            ) : null
            const card = (
              <li key={item.id} className={`attachment-card file ${item.state.toLowerCase()}`}>
                <span className="file-glyph" aria-hidden="true"><FileIcon /></span>
                <div>
                  <strong>{item.filename}</strong>
                  <span>{formatSize(item.byteSize)}</span>
                  {failed.has(item.id) && (previewKind(item.mime) === 'video' || previewKind(item.mime) === 'audio') ? (
                    <span className="attachment-fallback">{t(previewKind(item.mime) === 'video' ? 'composer.videoUnavailable' : 'composer.audioUnavailable')}</span>
                  ) : null}
                  {failed.has(item.id) && previewKind(item.mime) === 'image' ? <span className="attachment-fallback">{t('composer.imageUnavailable')}</span> : null}
                  {unavailable ? <span className="attachment-fallback">{t(statusLabel(item.state))}</span> : null}
                </div>
                {actions?.(item)}
                {download}
              </li>
            )
            if (kind === 'video') {
              return (
                <li key={item.id} className="attachment-card video">
                  <VideoAttachment item={item} href={href} onFail={() => fail(item.id)} onOpen={() => open(item)} />
                  <div className="attachment-media-foot">
                    <strong>{item.filename}</strong>
                    <span>{formatSize(item.byteSize)}</span>
                    {actions?.(item)}
                    {download}
                  </div>
                </li>
              )
            }
            if (kind === 'audio') {
              return (
                <li key={item.id} className="attachment-card audio">
                  <div className="attachment-media-foot">
                    <strong>{item.filename}</strong>
                    <span>{formatSize(item.byteSize)}</span>
                    {actions?.(item)}
                    {download}
                  </div>
                  <audio controls preload="none" src={href} aria-label={item.filename} onError={() => fail(item.id)} />
                </li>
              )
            }
            if (kind === 'csv') {
              return (
                <li key={item.id} className="attachment-card csv">
                  <div className="attachment-media-foot">
                    <span className="file-glyph" aria-hidden="true"><FileIcon /></span>
                    <strong>{item.filename}</strong>
                    <span>{formatSize(item.byteSize)}</span>
                    {actions?.(item)}
                    {download}
                  </div>
                  <CsvPreview src={href} onOpen={() => openMediaViewer([{ kind: 'csv', src: href, name: item.filename }])} />
                </li>
              )
            }
            return card
          })}
        </ul>
      ) : null}
    </div>
  )
}

/** A video as a poster (its first frame) that plays where it is, never by itself; a video the browser cannot play becomes a file card. */
function VideoAttachment({ item, href, onFail, onOpen }: { item: MessageAttachment; href: string; onFail: () => void; onOpen: () => void }) {
  const { t } = useI18n()
  const video = useRef<HTMLVideoElement>(null)
  const [playing, setPlaying] = useState(false)
  const [duration, setDuration] = useState<number | null>(null)
  function play() {
    setPlaying(true)
    void video.current?.play().catch(() => undefined)
  }
  return (
    <div className="video-frame">
      <video
        ref={video}
        src={href}
        preload="metadata"
        playsInline
        controls={playing}
        aria-label={item.filename}
        onLoadedMetadata={(event) => setDuration(event.currentTarget.duration)}
        onError={onFail}
        onEnded={() => setPlaying(false)}
      />
      {!playing ? (
        <>
          <button type="button" className="video-play" aria-label={`${t('composer.play')}: ${item.filename}`} onClick={play}><PlayIcon /></button>
          {duration !== null && Number.isFinite(duration) ? <span className="video-duration">{clock(duration)}</span> : null}
        </>
      ) : null}
      <button type="button" className="video-expand" aria-label={`${t('composer.openLarge')}: ${item.filename}`} title={t('composer.openLarge')} onClick={() => { video.current?.pause(); onOpen() }}><ExpandIcon /></button>
    </div>
  )
}

function failureText(item: UploadItem, t: ReturnType<typeof useI18n>['t']): string {
  if (item.failure === 'tooLarge') return item.limit !== null ? t('composer.uploadTooLarge', { limit: formatSize(item.limit) }) : t('composer.uploadRejected')
  if (item.failure === 'rejected') return t('composer.uploadRejected')
  return t('composer.uploadFailed')
}

/**
 * Uploads in flight or failed, with progress, cancel while they run, and retry and remove once they
 * failed. A failed file stays here until it is dealt with, and the form cannot be sent meanwhile.
 */
export function UploadCards({
  items,
  busy = false,
  onRetry,
  onRemove,
}: {
  items: UploadItem[]
  busy?: boolean
  onRetry: (item: UploadItem) => void
  onRemove: (item: UploadItem) => void
}) {
  const { t } = useI18n()
  if (items.length === 0) return null
  return (
    <ul className="attachment-list upload-tray" aria-label={t('composer.uploading')}>
      {items.map((item) => (
        <li key={item.localId} className={`attachment-card ${item.previewUrl ? 'image' : 'file'} ${item.phase}`}>
          {item.previewUrl ? <img className="attachment-thumb tray" src={item.previewUrl} alt="" /> : <span className="file-glyph" aria-hidden="true"><FileIcon /></span>}
          <div>
            <strong>{item.filename}</strong>
            <span>{formatSize(item.byteSize)}{item.mime ? ` · ${item.mime}` : ''}</span>
            {item.phase === 'uploading' ? (
              <>
                <span className="upload-track" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(item.progress * 100)} aria-label={item.filename}>
                  <span className="upload-progress" style={{ width: `${String(Math.round(item.progress * 100))}%` }} />
                </span>
                <span>{t('composer.uploading')} · {Math.round(item.progress * 100)}%</span>
              </>
            ) : null}
            {item.phase === 'failed' ? <span className="attachment-fallback" role="alert">{failureText(item, t)}</span> : null}
          </div>
          {item.phase === 'failed' && item.failure === 'transfer' ? <Button type="button" variant="secondary" disabled={busy} onClick={() => onRetry(item)}>{t('composer.retry')}</Button> : null}
          <IconButton
            label={`${t(item.phase === 'uploading' ? 'composer.cancelUpload' : 'composer.removeFile')}: ${item.filename}`}
            className="attachment-action"
            disabled={busy}
            onClick={() => onRemove(item)}
          >
            <CloseIcon />
          </IconButton>
        </li>
      ))}
    </ul>
  )
}
