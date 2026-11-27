import { LocaleToggle, useI18n } from '../i18n/I18nProvider'
import { IconChat, IconDocs, IconProjects } from '../shell/icons'
import { LoginForm } from '../modules/identity'
import { useSignInGate } from '../modules/identity/useSignInGate'
import { QueryError } from '../ui/QueryError'

export function LoginPage() {
  const { t } = useI18n()
  const gate = useSignInGate()
  // Until the server has said whether a session exists, neither the form nor a redirect: no flash of a sign-in page for someone who is signed in.
  if (gate.phase === 'checking' || gate.phase === 'redirecting') return <p className="center-note" role="status">{t('loading')}</p>
  return (
    <div className="gate">
      <section className="gate-story">
        <div className="gate-brand">
          <span className="gate-brand-mark" aria-hidden="true">P</span>
          <span className="gate-brand-name">{t('app.name')}</span>
        </div>
        <div className="gate-headline">
          <h2>{t('app.tagline')}</h2>
          <span className="gate-rule" aria-hidden="true" />
        </div>
        <div className="pillar-grid">
          <div className="pillar"><span className="pillar-icon"><IconChat /></span><strong>{t('nav.chat')}</strong><span>{t('pillar.chat')}</span></div>
          <div className="pillar"><span className="pillar-icon"><IconProjects /></span><strong>{t('nav.projects')}</strong><span>{t('pillar.projects')}</span></div>
          <div className="pillar"><span className="pillar-icon"><IconDocs /></span><strong>{t('nav.docs')}</strong><span>{t('pillar.docs')}</span></div>
        </div>
      </section>
      <section className="gate-panel">
        <div className="gate-locale"><LocaleToggle /></div>
        {gate.phase === 'failed' ? <QueryError query={{ error: gate.error, refetch: gate.retry }} /> : <LoginForm onSigningIn={gate.setSigningIn} />}
        {gate.phase === 'failed' ? null : <p className="hint">{t('login.lead')}</p>}
      </section>
    </div>
  )
}
