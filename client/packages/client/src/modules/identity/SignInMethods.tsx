import { useState } from 'react'
import { isApiError } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { useReauth } from './reauth'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'

/** Account → sign-in methods and sessions. Every change goes through the one reauth dialog. */
export function SignInMethods() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const identities = useQuery(queries.identities())
  const sessions = useQuery(queries.sessions())
  const { run, dialog } = useReauth()
  const [error, setError] = useState<string | null>(null)

  async function act(action: () => Promise<void>) {
    setError(null)
    try {
      await run(action)
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
    } catch (caught) {
      if (isApiError(caught) && caught.code === 'last_method') setError(t('methods.lastMethod'))
      else setError(t('error.generic'))
    }
  }

  async function link(provider: string) {
    await act(async () => {
      window.location.assign(await api.authStart(provider, { intent: 'link', returnTo: window.location.pathname }))
    })
  }

  const view = identities.data
  const linked = new Set(view?.items.map((item) => item.provider) ?? [])
  const total = (view?.items.length ?? 0) + (view?.hasPassword ? 1 : 0)

  return (
    <>
    <section className="card settings-card account-methods">
      <h2>{t('methods.title')}</h2>
      {dialog}
      {error ? <p className="form-error">{error}</p> : null}
      <ul className="plain-list">
        <li>
          {t('methods.password')}: {view?.hasPassword ? t('methods.set') : t('methods.notSet')}
          {view?.hasPassword && total > 1 ? (
            <button type="button" className="secondary" onClick={() => void act(() => api.removePassword())}>{t('methods.remove')}</button>
          ) : null}
        </li>
        {view?.items.map((item) => (
          <li key={item.provider}>
            {item.displayName}
            {item.email ? ` (${item.email})` : ''}
            {total > 1 ? (
              <button type="button" className="secondary" onClick={() => void act(() => api.unlinkIdentity(item.provider))}>{t('methods.unlink')}</button>
            ) : null}
          </li>
        ))}
      </ul>
      {view?.providers
        .filter((provider) => !linked.has(provider.key))
        .map((provider) => (
          <button key={provider.key} type="button" className="secondary" onClick={() => void link(provider.key)}>
            {t('methods.link', { provider: provider.displayName })}
          </button>
        ))}
    </section>
    <section className="card settings-card account-sessions">
      <h2>{t('sessions.title')}</h2>
      <ul className="plain-list">
        {sessions.data?.items.map((session) => (
          <li key={session.id} className={session.current ? 'is-current' : undefined}>
            <span className="row-text">
              <b>{session.deviceLabel ?? t('sessions.unknownDevice')}</b>
              <small>{new Date(session.lastSeenAt ?? session.createdAt).toLocaleString()}</small>
            </span>
            {session.current ? <span className="current-badge">{t('sessions.current')}</span> : null}
            {session.current ? null : (
              <button
                type="button"
                className="secondary"
                onClick={() => void act(async () => { await api.revokeSession(session.id); await queryClient.invalidateQueries({ queryKey: queryKeys.sessions }) })}
              >
                {t('sessions.revoke')}
              </button>
            )}
          </li>
        ))}
      </ul>
      {(sessions.data?.items.length ?? 0) > 1 ? (
        <button
          type="button"
          className="secondary danger-outline"
          onClick={() => void act(async () => { await api.revokeOtherSessions(); await queryClient.invalidateQueries({ queryKey: queryKeys.sessions }) })}
        >
          {t('sessions.revokeOthers')}
        </button>
      ) : null}
    </section>
    </>
  )
}
