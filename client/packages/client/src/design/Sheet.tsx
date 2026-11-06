import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useI18n } from '../i18n/I18nProvider'
import { useFocusLayer, useScrollLock } from './sheetBehavior'
import { useSheetBack } from './sheetHistory'
import './sheet.css'

/** How far the on-screen keyboard covers the bottom of the window, so a sheet with an input stays above it. */
function useKeyboardInset(active: boolean): number {
  const [inset, setInset] = useState(0)
  useEffect(() => {
    const viewport = window.visualViewport
    if (!active || !viewport) return
    const update = () => setInset(Math.max(0, window.innerHeight - viewport.height - viewport.offsetTop))
    update()
    viewport.addEventListener('resize', update)
    viewport.addEventListener('scroll', update)
    return () => {
      viewport.removeEventListener('resize', update)
      viewport.removeEventListener('scroll', update)
      setInset(0)
    }
  }, [active])
  return inset
}

/**
 * A bottom sheet for a short decision. It closes on the backdrop, Escape or the close button, and
 * moves up with the keyboard. Anything longer than a few fields belongs in a full-screen modal.
 */
export function Sheet({ open, title, onClose, children }: { open: boolean; title: string; onClose: () => void; children: ReactNode }) {
  const { t } = useI18n()
  const inset = useKeyboardInset(open)
  const layer = useRef<HTMLDivElement>(null)
  useScrollLock(open)
  useSheetBack(open, onClose)
  useFocusLayer(open, layer, 'root')
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [open, onClose])
  if (!open) return null
  return createPortal(
    <div className="m-sheet-root">
      <button type="button" className="m-sheet-backdrop" aria-label={t('close')} onClick={onClose} tabIndex={-1} />
      <div ref={layer} className="m-sheet" role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} style={{ bottom: inset }}>
        <span className="m-sheet-grip" aria-hidden="true" />
        <header className="m-sheet-head">
          <h2>{title}</h2>
          <button type="button" className="m-sheet-close" onClick={onClose}>{t('close')}</button>
        </header>
        <div className="m-sheet-body">{children}</div>
      </div>
    </div>,
    document.body,
  )
}

/** One tappable row inside a sheet. */
export function SheetRow({ children, onClick, danger = false, selected = false, disabled = false, hint }: { children: ReactNode; onClick: () => void; danger?: boolean; selected?: boolean; /** A choice that is being saved cannot be tapped again. */ disabled?: boolean; hint?: string }) {
  return (
    <button type="button" className={['m-sheet-row', danger ? 'danger' : '', selected ? 'selected' : ''].filter(Boolean).join(' ')} aria-pressed={selected || undefined} disabled={disabled} onClick={onClick}>
      <span>{children}</span>
      {hint ? <small>{hint}</small> : null}
      {selected ? <b aria-hidden="true">✓</b> : null}
    </button>
  )
}
