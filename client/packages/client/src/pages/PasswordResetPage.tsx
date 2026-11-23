import { LocaleToggle, useI18n } from '../i18n/I18nProvider'
import { ConfirmEmailForm } from '../modules/identity/ConfirmEmail'
import { ForgotPasswordForm, ResetPasswordForm } from '../modules/identity/PasswordReset'

function Frame({ children }: { children: React.ReactNode }) {
  const { t } = useI18n()
  return (
    <div className="gate">
      <section className="gate-story">
        <h1>{t('app.name')}</h1>
        <p>{t('reset.title')}</p>
      </section>
      <section className="gate-panel">
        <LocaleToggle />
        {children}
      </section>
    </div>
  )
}

/** `/forgot-password`: asks for the reset mail. */
export function ForgotPasswordPage() {
  return <Frame><ForgotPasswordForm /></Frame>
}

/** `/reset-password#<token>`: the page the mailed link opens. */
export function ResetPasswordPage() {
  return <Frame><ResetPasswordForm /></Frame>
}

/** `/confirm-email#<token>`: the page the link to a new e-mail address opens. */
export function ConfirmEmailPage() {
  return <Frame><ConfirmEmailForm /></Frame>
}
