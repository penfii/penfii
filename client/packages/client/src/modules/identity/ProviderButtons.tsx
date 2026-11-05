import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'

/** Provider sign-in buttons. The provider URL comes from the server; nothing is built here. */
export function ProviderButtons({
  intent,
  invitationToken,
  returnTo,
}: {
  intent: 'login' | 'invite'
  invitationToken?: string
  returnTo?: string
}) {
  const { t } = useI18n()
  const providers = useQuery(queries.authProviders())
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  if (!providers.data || providers.data.length === 0) return null

  async function go(key: string) {
    setBusy(key)
    setError(null)
    try {
      const url = await api.authStart(key, { intent, invitationToken, returnTo })
      window.location.assign(url)
    } catch {
      setError(t('auth.startFailed'))
      setBusy(null)
    }
  }

  return (
    <div className="provider-buttons">
      {providers.data.map((provider) => (
        <button key={provider.key} type="button" className="secondary" disabled={busy !== null} onClick={() => void go(provider.key)}>
          {t('auth.continueWith', { provider: provider.displayName })}
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
