import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'

/**
 * The one confirmation request per token, shared by every mount of the page: React's development double
 * effect (StrictMode) or a remount must not send the single-use token twice, which would answer the
 * second request "invalid" after the first one succeeded.
 */
const confirming = new Map<string, Promise<unknown>>()

function confirmOnce(token: string): Promise<unknown> {
  let pending = confirming.get(token)
  if (!pending) {
    pending = api.confirmEmailChange(token)
    confirming.set(token, pending)
  }
  return pending
}

/**
 * The page the confirmation link opens (`/confirm-email#<token>`). The token is in the fragment, which
 * a browser never sends to a server; it is read once, removed from the address bar, and goes out in the
 * body of the one request. No sign-in is needed: the link is the proof, and it is often opened on
 * another device.
 */
export function ConfirmEmailForm() {
  const { t } = useI18n()
  const [token] = useState(() => window.location.hash.slice(1))
  const [state, setState] = useState<'working' | 'done' | 'invalid' | 'taken' | 'failed'>(token ? 'working' : 'invalid')

  useEffect(() => {
    if (window.location.hash) window.history.replaceState(null, '', window.location.pathname)
  }, [])

  useEffect(() => {
    if (!token) return
    let live = true
    confirmOnce(token).then(
      () => live && setState('done'),
      (caught: unknown) => {
        if (!live) return
        if (isApiError(caught) && caught.code === 'account_exists') setState('taken')
        else if (isApiError(caught) && caught.status === 409) setState('invalid')
        else setState('failed')
      },
    )
    return () => {
      live = false
    }
  }, [token])

  return (
    <div className="card">
      <h1>{t('confirmEmail.title')}</h1>
      {state === 'working' ? <p role="status">{t('confirmEmail.working')}</p> : null}
      {state === 'done' ? <p role="status">{t('confirmEmail.done')}</p> : null}
      {state === 'invalid' ? <p role="alert">{t('confirmEmail.invalid')}</p> : null}
      {state === 'taken' ? <p role="alert">{t('confirmEmail.taken')}</p> : null}
      {state === 'failed' ? <p role="alert">{t('error.generic')}</p> : null}
      {state === 'done' ? <Link to="/login">{t('reset.backToLogin')}</Link> : <Link to="/account">{t('confirmEmail.toAccount')}</Link>}
    </div>
  )
}
