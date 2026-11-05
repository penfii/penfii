import { LocaleToggle, useI18n } from '../i18n/I18nProvider'
import { IconChat, IconDocs, IconProjects } from '../shell/icons'
import { LoginForm } from '../modules/identity'

export function LoginPage() {
  const { t } = useI18n()
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
        <LoginForm />
        <p className="hint">{t('login.lead')}</p>
      </section>
    </div>
  )
}
