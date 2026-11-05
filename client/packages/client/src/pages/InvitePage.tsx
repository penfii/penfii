import { useParams } from 'react-router'
import { LocaleToggle, useI18n } from '../i18n/I18nProvider'
import { InviteForm, InviteJoined } from '../modules/identity'
import { NotFoundState } from '../ui/NotFoundState'

/** `/invite/joined` is where a provider sign-in lands after accepting; `/invite/:token` is the invitation link. */
export function InvitePage() {
  const { t } = useI18n()
  const { token } = useParams()
  return (
    <div className="gate">
      <section className="gate-story">
        <h1>{t('app.name')}</h1>
        <p>{t('invite.title')}</p>
      </section>
      <section className="gate-panel">
        <LocaleToggle />
        {token === 'joined' ? <InviteJoined /> : token ? <InviteForm token={token} /> : <NotFoundState />}
      </section>
    </div>
  )
}
