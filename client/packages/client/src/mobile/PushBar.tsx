import { useNavigate } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'

/** Top bar of a pushed screen: the previous screen's name, the title (the screen's heading) and a spacer that keeps it centred. */
export function PushBar({ fallback, title, backLabel, onBack }: { fallback: string; title: string; backLabel: string; /** A screen that is a detail of another one closes itself (see overlayTrail) instead of stepping back blindly. */ onBack?: () => void }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  function back() {
    if (onBack) return onBack()
    const idx = (window.history.state as { idx?: number } | null)?.idx ?? 0
    if (idx > 0) void navigate(-1)
    else void navigate(fallback, { replace: true })
  }
  return (
    <header className="mobile-bar">
      {/* The name says where it goes and contains the words on the button, so voice control can say them. */}
      <button type="button" className="mobile-back" onClick={back} aria-label={t('mobile.backTo', { name: backLabel })}>
        <span aria-hidden="true">‹</span> {backLabel}
      </button>
      <h1>{title}</h1>
      <span className="mobile-bar-end" />
    </header>
  )
}
