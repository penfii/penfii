import { useEffect, useRef, useState } from 'react'
import { Button, Dialog } from './ui'
import { useI18n } from '../i18n/I18nProvider'

const VIEW = 240
const OUT = 512

/**
 * Adjust a profile photo before it is uploaded: drag to position, zoom, then a square JPEG.
 * The preview and the file use the same placement so what is saved is what was framed.
 */
export function AvatarCropDialog({
  file,
  onCancel,
  onConfirm,
}: {
  file: File
  onCancel: () => void
  onConfirm: (blob: Blob) => void
}) {
  const { t } = useI18n()
  const [url, setUrl] = useState<string | null>(null)
  const [natural, setNatural] = useState<{ w: number; h: number } | null>(null)
  const [zoom, setZoom] = useState(1)
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  const drag = useRef<{ x: number; y: number; ox: number; oy: number } | null>(null)
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    const next = URL.createObjectURL(file)
    setUrl(next)
    return () => URL.revokeObjectURL(next)
  }, [file])

  const place = natural
    ? placement(natural.w, natural.h, zoom, offset, VIEW)
    : null

  async function confirm() {
    if (!url || !natural || busy) return
    setBusy(true)
    setFailed(false)
    try {
      const blob = await renderJpeg(url, natural.w, natural.h, zoom, offset)
      onConfirm(blob)
    } catch {
      setFailed(true)
      setBusy(false)
    }
  }

  return (
    <Dialog open title={t('profile.cropTitle')} onClose={onCancel} presentation="full">
      <p className="hint">{t('profile.cropHint')}</p>
      <div
        className="crop-frame"
        onPointerDown={(event) => {
          drag.current = { x: event.clientX, y: event.clientY, ox: offset.x, oy: offset.y }
          event.currentTarget.setPointerCapture(event.pointerId)
        }}
        onPointerMove={(event) => {
          const start = drag.current
          if (!start) return
          setOffset({ x: start.ox + event.clientX - start.x, y: start.oy + event.clientY - start.y })
        }}
        onPointerUp={() => {
          drag.current = null
        }}
      >
        {url ? (
          <img
            src={url}
            alt=""
            draggable={false}
            onLoad={(event) => setNatural({ w: event.currentTarget.naturalWidth, h: event.currentTarget.naturalHeight })}
            style={place ? { position: 'absolute', left: place.x, top: place.y, width: place.w, height: place.h } : undefined}
          />
        ) : null}
      </div>
      <label className="field">
        <span>{t('profile.zoom')}</span>
        <input type="range" min={1} max={3} step={0.01} value={zoom} aria-label={t('profile.zoom')} onChange={(event) => setZoom(Number(event.target.value))} />
      </label>
      {failed ? <p className="board-error" role="alert">{t('profile.photoError')}</p> : null}
      <div className="dialog-actions">
        <Button type="button" variant="ghost" onClick={onCancel}>{t('profile.cropCancel')}</Button>
        <Button type="button" disabled={!natural || busy} onClick={() => void confirm()}>{t('profile.cropSave')}</Button>
      </div>
    </Dialog>
  )
}

function placement(width: number, height: number, zoom: number, offset: { x: number; y: number }, view: number) {
  const scale = Math.max(view / width, view / height) * zoom
  const w = width * scale
  const h = height * scale
  return { x: (view - w) / 2 + offset.x, y: (view - h) / 2 + offset.y, w, h }
}

function renderJpeg(url: string, width: number, height: number, zoom: number, offset: { x: number; y: number }): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.onload = () => {
      const canvas = document.createElement('canvas')
      canvas.width = OUT
      canvas.height = OUT
      const ctx = canvas.getContext('2d')
      if (!ctx) {
        reject(new Error('canvas'))
        return
      }
      const factor = OUT / VIEW
      const box = placement(width, height, zoom, { x: offset.x * factor, y: offset.y * factor }, OUT)
      ctx.drawImage(image, box.x, box.y, box.w, box.h)
      let quality = 0.9
      const finish = (blob: Blob | null) => {
        if (!blob) {
          reject(new Error('blob'))
          return
        }
        if (blob.size > 512 * 1024 && quality > 0.5) {
          quality -= 0.1
          canvas.toBlob(finish, 'image/jpeg', quality)
          return
        }
        if (blob.size > 512 * 1024) reject(new Error('too-large'))
        else resolve(blob)
      }
      canvas.toBlob(finish, 'image/jpeg', quality)
    }
    image.onerror = () => reject(new Error('image'))
    image.src = url
  })
}
