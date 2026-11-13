import { useEffect, type ReactNode } from 'react'
import { isUnauthorized } from '@penfii/api-client'
import { Navigate, useLocation } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { rememberReturn } from './returnTo'

export function RequireAuth({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  const session = useSession()
  const location = useLocation()
  const signedOut = session.isError ? isUnauthorized(session.error) : !session.isPending && !session.data
  const here = location.pathname + location.search
  // Remembered where the person was going, once, when they are sent to sign in (not while the page renders).
  useEffect(() => {
    if (signedOut) rememberReturn(here)
  }, [signedOut, here])
  if (session.isPending) return <p className="center-note">{t('loading')}</p>
  if (session.isError) {
    if (isUnauthorized(session.error)) {
      return <Navigate to="/login" replace />
    }
    return <p className="center-note">{t('error.generic')}</p>
  }
  if (!session.data) {
    return <Navigate to="/login" replace />
  }
  return children
}
