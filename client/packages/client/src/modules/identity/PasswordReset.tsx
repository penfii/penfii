import { useEffect, useState, type FormEvent } from 'react'
import { Link } from 'react-router'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { errorText } from '../../ui/errorText'

/** Asks for a reset mail. The answer is the same for every address, so the page says only that a mail is on its way if there is an account. */
export function ForgotPasswordForm() {
  const { t } = useI18n()
  const [email, setEmail] = useState('')
  const [state, setState] = useState<'idle' | 'busy' | 'asked' | 'off'>('idle')
  const [error, setError] = useState<string | null>(null)

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    setState('busy')
    setError(null)
    try {
      await api.requestPasswordReset(email.trim())
      setState('asked')
    } catch (caught) {
      if (isApiError(caught) && caught.status === 404) setState('off')
      else {
        setState('idle')
        setError(isApiError(caught) && caught.status === 400 ? t('reset.badEmail') : t('error.generic'))
      }
    }
  }

  if (state === 'asked') {
    return (
      <div className="card">
        <h1>{t('reset.title')}</h1>
        <p role="status">{t('reset.asked')}</p>
        <Link to="/login">{t('reset.backToLogin')}</Link>
      </div>
    )
  }
  if (state === 'off') {
    return (
      <div className="card">
        <h1>{t('reset.title')}</h1>
        <p>{t('reset.off')}</p>
        <Link to="/login">{t('reset.backToLogin')}</Link>
      </div>
    )
  }
  return (
    <form className="card" onSubmit={(event) => void onSubmit(event)}>
      <h1>{t('reset.title')}</h1>
      <p className="hint">{t('reset.lead')}</p>
      <label>
        {t('login.email')}
        <input type="email" autoComplete="username" required value={email} onChange={(event) => setEmail(event.target.value)} />
      </label>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      <button type="submit" disabled={state === 'busy'}>{t('reset.send')}</button>
      <Link to="/login">{t('reset.backToLogin')}</Link>
    </form>
  )
}

/**
 * Sets the new password. The token is in the fragment of the mailed link (`/reset-password#<token>`),
 * which a browser never sends to a server and never puts in a Referer. It is read once and removed
 * from the address bar at once; after that it lives only in this page's memory and goes out in the
 * body of the one request.
 */
export function ResetPasswordForm() {
  const { t } = useI18n()
  const [token] = useState(() => window.location.hash.slice(1))
  const [password, setPassword] = useState('')
  const [again, setAgain] = useState('')
  const [state, setState] = useState<'idle' | 'busy' | 'done' | 'invalid'>(token ? 'idle' : 'invalid')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (window.location.hash) window.history.replaceState(null, '', window.location.pathname)
  }, [])

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    if (password !== again) {
      setError(t('reset.mismatch'))
      return
    }
    setState('busy')
    setError(null)
    try {
      await api.confirmPasswordReset(token, password)
      setState('done')
    } catch (caught) {
      if (isApiError(caught) && caught.status === 409) setState('invalid')
      else {
        setState('idle')
        setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
      }
    }
  }

  if (state === 'done') {
    return (
      <div className="card">
        <h1>{t('reset.newTitle')}</h1>
        <p role="status">{t('reset.done')}</p>
        <Link to="/login">{t('reset.backToLogin')}</Link>
      </div>
    )
  }
  if (state === 'invalid') {
    return (
      <div className="card">
        <h1>{t('reset.newTitle')}</h1>
        <p role="alert">{t('reset.invalid')}</p>
        <Link to="/forgot-password">{t('reset.askAgain')}</Link>
      </div>
    )
  }
  return (
    <form className="card" onSubmit={(event) => void onSubmit(event)}>
      <h1>{t('reset.newTitle')}</h1>
      <p className="hint">{t('reset.newLead')}</p>
      <label>
        {t('reset.newPassword')}
        <input type="password" autoComplete="new-password" required minLength={15} value={password} onChange={(event) => setPassword(event.target.value)} />
      </label>
      <label>
        {t('reset.again')}
        <input type="password" autoComplete="new-password" required minLength={15} value={again} onChange={(event) => setAgain(event.target.value)} />
      </label>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      <button type="submit" disabled={state === 'busy'}>{t('reset.save')}</button>
    </form>
  )
}
