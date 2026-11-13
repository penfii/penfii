import { useState, type FormEvent } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, isDbUnavailable } from '@penfii/api-client'
import type { Me } from '@penfii/contracts'
import { Link, useNavigate } from 'react-router'
import { api } from '../../api'
import { peekReturn, takeReturn } from '../../app/returnTo'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { ProviderButtons, useAuthRedirectError } from './ProviderButtons'

/** Whole minutes (at least 1) until the lock ends, from the server's `retryAfterSeconds`. */
function retryMinutes(error: { details?: unknown }): number {
  const seconds = (error.details as { retryAfterSeconds?: unknown } | undefined)?.retryAfterSeconds
  return typeof seconds === 'number' && seconds > 0 ? Math.max(1, Math.ceil(seconds / 60)) : 15
}

export function LoginForm() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const redirectError = useAuthRedirectError()
  // The link appears only while the server sends e-mail; without it an operator sets the password (`admin-cli`).
  const info = useQuery({ ...queries.serverInfo(), retry: false })
  const canReset = Array.isArray(info.data?.features) && info.data.features.includes('password-reset')

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.login({ email, password })
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
      void navigate(takeReturn('/', queryClient.getQueryData<Me>(queryKeys.me)?.id))
    } catch (caught) {
      if (isApiError(caught) && caught.status === 401) setError(t('login.failed'))
      else if (isApiError(caught) && caught.status === 429) setError(t('login.tooManyAttempts', { minutes: String(retryMinutes(caught)) }))
      else if (isDbUnavailable(caught)) setError(t('error.dbUnavailable'))
      else setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="card" onSubmit={(event) => void onSubmit(event)}>
      <h1>{t('login.title')}</h1>
      {peekReturn()?.startsWith('/invite/') ? <p className="hint">{t('login.forInvitation')}</p> : null}
      <label>
        {t('login.email')}
        <input type="email" autoComplete="username" required value={email} onChange={(event) => setEmail(event.target.value)} />
      </label>
      <label>
        {t('login.password')}
        <input
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
      </label>
      {error ?? redirectError ? <p className="form-error">{error ?? redirectError}</p> : null}
      <button type="submit" disabled={busy}>
        {t('login.submit')}
      </button>
      {canReset ? <Link to="/forgot-password">{t('login.forgot')}</Link> : null}
      <ProviderButtons intent="login" returnTo={peekReturn()} />
    </form>
  )
}
