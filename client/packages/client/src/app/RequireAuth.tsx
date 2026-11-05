import type { ReactNode } from 'react'
import { isUnauthorized } from '@penfii/api-client'
import { Navigate, useLocation } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { rememberReturn } from './returnTo'

export function RequireAuth({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  const session = useSession()
  const location = useLocation()
  if (session.isPending) return <p className="center-note">{t('loading')}</p>
  if (session.isError) {
    if (isUnauthorized(session.error)) {
      rememberReturn(location.pathname + location.search)
      return <Navigate to="/login" replace />
    }
    return <p className="center-note">{t('error.generic')}</p>
  }
  if (!session.data) {
    rememberReturn(location.pathname + location.search)
    return <Navigate to="/login" replace />
  }
  return children
}
