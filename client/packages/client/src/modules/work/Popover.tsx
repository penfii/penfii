import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
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
  truncate = false,
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
  /** The label stays on one line and ends in an ellipsis when its button has no more room (the button must be allowed to shrink). */
  truncate?: boolean
  children: (close: () => void) => ReactNode
}) {
  const mobile = useIsMobile()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const button = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [flip, setFlip] = useState(false)
  const [place, setPlace] = useState<{ top: number; left: number } | null>(null)
  const close = useCallback(() => setOpen(false), [])
  // In a row that scrolls (`data-pop-float`, the Board toolbar) the panel cannot hang below its button: the row would clip it.
  // It is laid out against the window instead and follows its button while the row scrolls or the window changes.
  const floating = open && !mobile && ref.current?.closest('[data-pop-float]') != null
  // A panel that would leave the window on the right opens towards the left instead.
  useLayoutEffect(() => {
    if (!open || floating) {
      setFlip(false)
      return
    }
    const box = panelRef.current?.getBoundingClientRect()
    if (box && box.right > window.innerWidth - 4) setFlip(true)
  }, [open, floating])
  useLayoutEffect(() => {
    if (!floating) {
      setPlace(null)
      return
    }
    const reposition = () => {
      const anchor = button.current?.getBoundingClientRect()
      const panel = panelRef.current?.getBoundingClientRect()
      if (!anchor || !panel) return
      const left = Math.max(4, Math.min(align === 'end' ? anchor.right - panel.width : anchor.left, window.innerWidth - panel.width - 4))
      const below = anchor.bottom + 4
      const top = below + panel.height > window.innerHeight - 4 && anchor.top - panel.height - 4 >= 4 ? anchor.top - panel.height - 4 : below
      setPlace((now) => (now && now.top === top && now.left === left ? now : { top, left }))
    }
    reposition()
    window.addEventListener('resize', reposition)
    document.addEventListener('scroll', reposition, true)
    return () => {
      window.removeEventListener('resize', reposition)
      document.removeEventListener('scroll', reposition, true)
    }
  }, [floating, align])
  const floatStyle: CSSProperties | undefined = floating ? { position: 'fixed', top: place?.top ?? 0, left: place?.left ?? 0, right: 'auto', visibility: place ? undefined : 'hidden' } : undefined
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
        {iconOnly ? null : truncate ? <span className="pop-label" data-fit>{label}</span> : label}
        {badge}
      </button>
      {open && mobile ? (
        <Sheet open title={label} onClose={() => { close(); button.current?.focus() }}>
          {children(close)}
        </Sheet>
      ) : null}
      {open && !mobile ? (
        <div ref={panelRef} style={floatStyle} className={['pop-panel', !floating && (align === 'end' || flip) ? 'align-end' : '', wide ? 'wide' : ''].filter(Boolean).join(' ')} role="dialog" aria-label={label}>
          {children(close)}
        </div>
      ) : null}
    </div>
  )
}
