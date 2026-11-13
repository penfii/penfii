import { useI18n } from '../i18n/I18nProvider'
import { errorText } from './errorText'

/**
 * The failed state of a query: what went wrong (the database being unavailable is said as such) and a way to try again.
 * Used where a failed read would otherwise look like "loading" for ever or like an empty list.
 */
export function QueryError({ query, compact = false }: { query: { error: unknown; refetch: () => unknown }; compact?: boolean }) {
  const { t } = useI18n()
  return (
    <p className={compact ? 'query-error compact' : 'query-error'} role="alert">
      <span>{errorText(query.error, t('error.generic'), t('error.dbUnavailable'))}</span>{' '}
      <button type="button" className="btn btn-ghost" onClick={() => void query.refetch()}>{t('query.retry')}</button>
    </p>
  )
}
