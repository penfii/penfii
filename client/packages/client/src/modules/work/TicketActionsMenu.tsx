import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useI18n } from '../../i18n/I18nProvider'
import { MoreIcon } from '../../design/icons'
import { useDismiss } from '../../design/ui'
import { useFavorite, useLater, usePin } from '../personal/SavedButtons'

/**
 * The ticket panel's other object actions (Favorite, Later, Pin) behind one "More" button, for a
 * panel too narrow to show them in a row. Watch is the one that stays visible beside it. Each row says what it will do and whether it is on; the
 * result is announced by the shared notice, exactly as with the separate buttons.
 */
export function TicketActionsMenu({ workspaceId, ticketId, projectId }: { workspaceId: string; ticketId: string; projectId: string }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const favorite = useFavorite(workspaceId, 'TICKET', ticketId)
  const later = useLater(workspaceId, ticketId)
  const pin = usePin(projectId, ticketId)
  useDismiss(open, () => setOpen(false), root)

  useEffect(() => {
    if (open) root.current?.querySelector<HTMLElement>('[role="menuitemcheckbox"]:not(:disabled)')?.focus()
  }, [open])

  const items = [
    { key: 'favorite', state: favorite, label: favorite.on ? t('saved.favorite.remove') : t('saved.favorite.add') },
    { key: 'later', state: later, label: later.on ? t('saved.later.remove') : t('saved.later.add') },
    ...(pin.allowed ? [{ key: 'pin', state: pin, label: pin.on ? t('saved.pin.remove') : t('saved.pin.add') }] : []),
  ]

  function close() {
    setOpen(false)
    trigger.current?.focus()
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (!open) return
    if (event.key === 'Escape') {
      // Only the menu closes; the panel's own Escape (close the ticket) must not also fire.
      event.preventDefault()
      event.stopPropagation()
      close()
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const rows = [...root.current!.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]:not(:disabled)')]
      const at = rows.indexOf(document.activeElement as HTMLElement)
      const step = event.key === 'ArrowDown' ? 1 : -1
      rows[(at + step + rows.length) % rows.length]?.focus()
    } else if (event.key === 'Tab') {
      setOpen(false)
    }
  }

  return (
    <div className="ticket-more" ref={root} onKeyDown={onKeyDown}>
      <button ref={trigger} type="button" className="icon-button" aria-haspopup="menu" aria-expanded={open} aria-label={t('work.panel.more')} title={t('work.panel.more')} onClick={() => setOpen((value) => !value)}>
        <MoreIcon />
      </button>
      {open ? (
        <div className="ticket-more-menu" role="menu" aria-label={t('work.panel.more')}>
          {items.map(({ key, state, label }) => (
            <button key={key} type="button" role="menuitemcheckbox" aria-checked={state.on} className="ticket-more-item" disabled={state.pending} onClick={() => { state.toggle(); close() }}>
              <span className="ticket-more-check" aria-hidden="true">{state.on ? '✓' : ''}</span>
              {label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}
