import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { MoreIcon } from './icons'
import { Sheet, SheetRow } from './Sheet'
import { useDismiss } from './ui'
import { useIsMobile } from './useIsMobile'

export type ActionItem = {
  key: string
  label: string
  onSelect: () => void
  /** A destructive action is set apart and coloured. */
  danger?: boolean
  disabled?: boolean
}

/**
 * The "more actions" button of a row: one button for the actions that are not the row's main purpose,
 * a menu on a desktop and a bottom sheet on a phone. A destructive action sits last and is marked; what
 * it does is still asked for by the caller (a confirmation), the menu only offers it.
 */
export function ActionMenu({ label, items }: { /** Names the button, so one row's menu is not another's: "Actions for Mina". */ label: string; items: ActionItem[] }) {
  const mobile = useIsMobile()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  useDismiss(open && !mobile, () => setOpen(false), root)
  useEffect(() => {
    if (open && !mobile) root.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus()
  }, [open, mobile])
  if (items.length === 0) return null

  function close() {
    setOpen(false)
    trigger.current?.focus()
  }
  function pick(item: ActionItem) {
    setOpen(false)
    item.onSelect()
  }
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (!open || mobile) return
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      close()
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const rows = [...root.current!.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)')]
      const at = rows.indexOf(document.activeElement as HTMLElement)
      const step = event.key === 'ArrowDown' ? 1 : -1
      rows[(at + step + rows.length) % rows.length]?.focus()
    } else if (event.key === 'Tab') {
      setOpen(false)
    }
  }

  return (
    <div className="action-menu" ref={root} onKeyDown={onKeyDown}>
      <button ref={trigger} type="button" className="icon-button" aria-haspopup={mobile ? 'dialog' : 'menu'} aria-expanded={open} aria-label={label} title={label} onClick={() => setOpen((value) => !value)}>
        <MoreIcon />
      </button>
      {open && !mobile ? (
        <div className="action-menu-panel" role="menu" aria-label={label}>
          {items.map((item) => (
            <button key={item.key} type="button" role="menuitem" className={item.danger ? 'action-menu-item danger' : 'action-menu-item'} disabled={item.disabled} onClick={() => pick(item)}>{item.label}</button>
          ))}
        </div>
      ) : null}
      <Sheet open={open && mobile} title={label} onClose={() => setOpen(false)}>
        {items.map((item) => <SheetRow key={item.key} danger={item.danger} disabled={item.disabled} onClick={() => pick(item)}>{item.label}</SheetRow>)}
      </Sheet>
    </div>
  )
}
