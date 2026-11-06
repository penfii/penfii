import { useCallback, useEffect, useId, useRef, useState, type ButtonHTMLAttributes, type ReactNode, type RefObject, type SelectHTMLAttributes } from 'react'
import { useI18n } from '../i18n/I18nProvider'
import { CloseIcon } from './icons'
import { useSheetBack } from './sheetHistory'
import { useScrollLock } from './sheetBehavior'
import { useIsMobile } from './useIsMobile'
import './sheet.css'

export function useDismiss(open: boolean, onClose: () => void, ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    if (!open) return
    function onPointer(event: MouseEvent) {
      if (!ref.current?.contains(event.target as Node)) onClose()
    }
    document.addEventListener('mousedown', onPointer)
    return () => document.removeEventListener('mousedown', onPointer)
  }, [open, onClose, ref])
}

/** Up to two letters or digits to stand for a name; brackets, emoji and other marks are skipped ("[QA] Invitee" is QI). */
export function initials(name: string): string {
  const letters = (word: string) => [...word].filter((char) => /[\p{L}\p{N}]/u.test(char))
  const words = name.trim().split(/\s+/).map(letters).filter((word) => word.length > 0)
  if (words.length === 0) return '?'
  if (words.length === 1) return words[0].slice(0, 2).join('').toUpperCase()
  return `${words[0][0]}${words[1][0]}`.toUpperCase()
}

export function Avatar({ name, userId, hasAvatar, photoKey }: { name: string; userId?: string; hasAvatar?: boolean; photoKey?: string | number }) {
  const [failed, setFailed] = useState(false)
  if (userId && hasAvatar && !failed) {
    const version = photoKey === undefined ? '' : `?v=${encodeURIComponent(String(photoKey))}`
    return (
      <span className="avatar has-photo" aria-hidden="true">
        <img src={`/v1/users/${userId}/avatar${version}`} alt="" onError={() => setFailed(true)} />
      </span>
    )
  }
  return (
    <span className="avatar" aria-hidden="true">
      {initials(name)}
    </span>
  )
}

export function Button({
  variant = 'primary',
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary' | 'secondary' | 'ghost' | 'danger' }) {
  return <button {...props} className={['btn', `btn-${variant}`, className].filter(Boolean).join(' ')} />
}

/** A button that shows an icon and is named by `label` (also its tooltip), for repeated controls. */
export function IconButton({
  label,
  className,
  children,
  ...props
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label' | 'title'> & { label: string }) {
  return (
    <button type="button" {...props} className={['icon-button', className].filter(Boolean).join(' ')} aria-label={label} title={label}>
      {children}
    </button>
  )
}

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
      {hint ? <small className="field-hint">{hint}</small> : null}
    </label>
  )
}

export function Select(props: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select {...props} />
}

/**
 * How a dialog is shown on a phone. `sheet` is a short decision at the bottom of the screen;
 * `full` fills the screen, for a dialog that asks for text or a choice from a list. The default
 * is `sheet`; wider screens show every dialog the same way.
 */
export type DialogPresentation = 'sheet' | 'full'

export function Dialog({
  open,
  title,
  onClose,
  children,
  presentation = 'sheet',
  closeWithBack = true,
}: {
  open: boolean
  title: string
  onClose: () => void
  children: ReactNode
  presentation?: DialogPresentation
  /** On a phone the system Back closes the dialog first. A dialog that answers a navigation the router is holding back opts out. */
  closeWithBack?: boolean
}) {
  const { t } = useI18n()
  const mobile = useIsMobile()
  const ref = useRef<HTMLDialogElement>(null)
  useScrollLock(open && mobile)
  useSheetBack(open && closeWithBack, onClose)
  const titleId = useId()
  // What had focus when the dialog opened. A dialog that closes in place gives focus back by itself; one that
  // is removed while open (callers mount one on demand) would drop it on <body>, so it is given back here.
  const opener = useRef<HTMLElement | null>(null)
  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      dialog.showModal()
    }
    if (!open && dialog.open) dialog.close()
  }, [open])
  useEffect(() => {
    return () => {
      const back = opener.current
      setTimeout(() => {
        if (back?.isConnected && (document.activeElement === document.body || document.activeElement === null)) back.focus()
      })
    }
  }, [])
  return (
    <dialog
      ref={ref}
      className="dialog"
      data-presentation={presentation}
      aria-labelledby={titleId}
      onClose={onClose}
      // A sheet closes when its dimmed backdrop is tapped (the click lands on the dialog element itself).
      onClick={(event) => {
        if (mobile && presentation === 'sheet' && event.target === ref.current) onClose()
      }}
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
    >
      <div className="dialog-head">
        <h2 id={titleId}>{title}</h2>
        <button type="button" className="icon-button" onClick={onClose} aria-label={t('close')}>
          <CloseIcon />
        </button>
      </div>
      <div className="dialog-body">{children}</div>
    </dialog>
  )
}

type ConfirmOptions = { title: string; body: string; confirmLabel: string; tone?: 'danger' | 'primary' }

/**
 * One confirmation for every destructive action, in place of the browser's `confirm()`.
 * `confirm(...)` resolves true or false; render `dialog` once in the calling component.
 */
export function useConfirm() {
  const { t } = useI18n()
  const [pending, setPending] = useState<(ConfirmOptions & { settle: (ok: boolean) => void }) | null>(null)
  const opener = useRef<HTMLElement | null>(null)
  const confirm = useCallback((options: ConfirmOptions) => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    return new Promise<boolean>((resolve) => setPending({ ...options, settle: resolve }))
  }, [])
  const answer = (ok: boolean) => {
    pending?.settle(ok)
    setPending(null)
    // An unmounted dialog does not hand focus back by itself.
    opener.current?.focus()
  }
  const dialog = pending === null ? null : (
    <Dialog open title={pending.title} onClose={() => answer(false)}>
      <p className="confirm-body">{pending.body}</p>
      <div className="dialog-actions">
        <Button type="button" variant="secondary" onClick={() => answer(false)}>{t('cancel')}</Button>
        <Button type="button" variant={pending.tone === 'primary' ? 'primary' : 'danger'} onClick={() => answer(true)}>{pending.confirmLabel}</Button>
      </div>
    </Dialog>
  )
  return { confirm, dialog }
}

export function EmptyState({ title, body, children }: { title: string; body: string; children?: ReactNode }) {
  return (
    <section className="empty-state">
      <h2>{title}</h2>
      <p>{body}</p>
      {children ? <div className="empty-actions">{children}</div> : null}
    </section>
  )
}

export function Badge({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'accent' | 'preview' }) {
  return <span className={`badge badge-${tone}`}>{children}</span>
}

export function Tabs({
  tabs,
  value,
  onChange,
}: {
  tabs: Array<{ id: string; label: string }>
  value: string
  onChange: (id: string) => void
}) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={value === tab.id}
          className={value === tab.id ? 'tab active' : 'tab'}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  )
}

