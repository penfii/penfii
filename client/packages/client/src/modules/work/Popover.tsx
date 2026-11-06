import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { useDismiss } from '../../design/ui'
import { Sheet } from '../../design/Sheet'
import { useIsMobile } from '../../design/useIsMobile'

/**
 * A small panel that opens under its button. It closes on an outside click or Escape and hands
 * focus back to the button, so keyboard users never get lost in the Board toolbar. On a phone the
 * panel is a bottom `Sheet` instead, which brings its own backdrop, Escape, focus and scroll rules.
 */
export function Popover({
  label,
  icon,
  iconOnly = false,
  badge,
  align = 'start',
  wide = false,
  className,
  children,
}: {
  label: string
  /** Shown before the label; with `iconOnly` it replaces it visually and `label` stays the accessible name. */
  icon?: ReactNode
  iconOnly?: boolean
  /** Replaces the default button look, for a trigger that sits inside a row or a cell. */
  className?: string
  badge?: ReactNode
  align?: 'start' | 'end'
  wide?: boolean
  children: (close: () => void) => ReactNode
}) {
  const mobile = useIsMobile()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const button = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [flip, setFlip] = useState(false)
  const close = useCallback(() => setOpen(false), [])
  // A panel that would leave the window on the right opens towards the left instead.
  useLayoutEffect(() => {
    if (!open) {
      setFlip(false)
      return
    }
    const box = panelRef.current?.getBoundingClientRect()
    if (box && box.right > window.innerWidth - 4) setFlip(true)
  }, [open])
  useDismiss(open && !mobile, close, ref)

  useEffect(() => {
    if (!open || mobile) return
    function onKey(event: KeyboardEvent) {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      setOpen(false)
      button.current?.focus()
    }
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [open, mobile])

  return (
    <div className="pop" ref={ref}>
      <button ref={button} type="button" className={className ?? 'btn btn-secondary pop-trigger'} aria-haspopup="dialog" aria-expanded={open} aria-label={iconOnly ? label : undefined} title={iconOnly ? label : undefined} onClick={() => setOpen((value) => !value)}>
        {icon}
        {iconOnly ? null : label}
        {badge}
      </button>
      {open && mobile ? (
        <Sheet open title={label} onClose={() => { close(); button.current?.focus() }}>
          {children(close)}
        </Sheet>
      ) : null}
      {open && !mobile ? (
        <div ref={panelRef} className={['pop-panel', align === 'end' || flip ? 'align-end' : '', wide ? 'wide' : ''].filter(Boolean).join(' ')} role="dialog" aria-label={label}>
          {children(close)}
        </div>
      ) : null}
    </div>
  )
}
