import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { lastMethod, markProviderAttempt } from './loginMemory'

/** Provider sign-in buttons. The provider URL comes from the server; nothing is built here. */
export function ProviderButtons({
  intent,
  invitationToken,
  returnTo,
  keepSignedIn = false,
}: {
  intent: 'login' | 'invite'
  invitationToken?: string
  returnTo?: string
  /** The "keep me signed in" choice of a sign-in; an invitation never carries it. */
  keepSignedIn?: boolean
}) {
  const { t } = useI18n()
  const options = useQuery(queries.authOptions())
  const providers = options.data?.providers
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  if (!providers || providers.length === 0) return null
  // A provider that is no longer offered is not shown as the last one used.
  const last = intent === 'login' ? lastMethod() : null

  async function go(key: string) {
    setBusy(key)
    setError(null)
    try {
      const wantsLong = intent === 'login' && keepSignedIn && options.data?.longSession.enabled === true
      const url = await api.authStart(key, { intent, invitationToken, returnTo, ...(wantsLong ? { keepSignedIn: true } : {}) })
      // Pressing the button proves nothing: the method is recorded only when the session that comes back says it was this provider.
      if (intent === 'login') markProviderAttempt(key)
      window.location.assign(url)
    } catch {
      setError(t('auth.startFailed'))
      setBusy(null)
    }
  }

  return (
    <div className="provider-buttons">
      {providers.map((provider) => (
        <button key={provider.key} type="button" className="secondary" disabled={busy !== null} onClick={() => void go(provider.key)}>
          {t('auth.continueWith', { provider: provider.displayName })}
          {last === provider.key ? <span className="last-used">{t('login.lastUsed')}</span> : null}
        </button>
      ))}
      {error ? <p className="form-error">{error}</p> : null}
    </div>
  )
}

const AUTH_ERRORS = new Set([
  'not_linked',
  'invalid_attempt',
  'provider_denied',
  'provider_error',
  'id_token_invalid',
  'email_mismatch',
  'account_exists_sign_in',
  'identity_linked_elsewhere',
  'provider_already_linked',
  'wrong_account',
  'reauth_required',
  'unauthenticated',
])

/** `?auth_error=<code>` set by the provider callback redirect. */
export function useAuthRedirectError(): string | null {
  const { t } = useI18n()
  const code = new URLSearchParams(window.location.search).get('auth_error')
  if (!code) return null
  return AUTH_ERRORS.has(code) ? t(`auth.error.${code}` as 'auth.error.not_linked') : t('auth.error.provider_error')
}
