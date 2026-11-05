import { useCallback, useState, type FormEvent, type ReactNode } from 'react'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'

/**
 * Runs an action; when the server answers `reauth_required`, shows one dialog with the methods
 * the account can use, and retries the action after the person proved themselves.
 */
export function useReauth(): { run: <T>(action: () => Promise<T>) => Promise<T | undefined>; dialog: ReactNode } {
  const { t } = useI18n()
  const [pending, setPending] = useState<{ methods: string[]; retry: () => void; cancel: () => void } | null>(null)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const run = useCallback(
    async <T,>(action: () => Promise<T>): Promise<T | undefined> => {
      try {
        return await action()
      } catch (caught) {
        if (!(isApiError(caught) && caught.code === 'reauth_required')) throw caught
        const details = caught.details as { methods?: string[] } | undefined
        const methods = details?.methods ?? []
        return await new Promise<T | undefined>((resolve, reject) => {
          setPassword('')
          setError(null)
          setPending({
            methods,
            retry: () => {
              setPending(null)
              action().then(resolve, reject)
            },
            cancel: () => {
              setPending(null)
              resolve(undefined)
            },
          })
        })
      }
    },
    [],
  )

  async function submit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await api.reauthPassword(password)
      pending?.retry()
    } catch (caught) {
      setError(isApiError(caught) && caught.status === 401 ? t('reauth.wrongPassword') : t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  async function provider(key: string) {
    setError(null)
    try {
      window.location.assign(await api.reauthProviderStart(key, window.location.pathname + window.location.search))
    } catch {
      setError(t('error.generic'))
    }
  }

  const dialog = pending ? (
    <div className="reauth-overlay">
    <div className="card reauth-dialog" role="dialog" aria-modal="true" aria-label={t('reauth.title')}>
      <h2>{t('reauth.title')}</h2>
      <p>{t('reauth.lead')}</p>
      {pending.methods.includes('password') ? (
        <form onSubmit={(event) => void submit(event)}>
          <label>
            {t('login.password')}
            <input type="password" autoComplete="current-password" autoFocus required value={password} onChange={(event) => setPassword(event.target.value)} />
          </label>
          <button type="submit" disabled={busy}>{t('reauth.confirm')}</button>
        </form>
      ) : null}
      {pending.methods
        .filter((method) => method.startsWith('provider:'))
        .map((method) => (
          <button key={method} type="button" className="secondary" onClick={() => void provider(method.slice('provider:'.length))}>
            {t('auth.continueWith', { provider: method.slice('provider:'.length) })}
          </button>
        ))}
      {pending.methods.length === 0 ? <p className="form-error">{t('reauth.noMethod')}</p> : null}
      {error ? <p className="form-error">{error}</p> : null}
      <button type="button" className="secondary" onClick={pending.cancel}>{t('reauth.cancel')}</button>
    </div>
    </div>
  ) : null

  return { run, dialog }
}
