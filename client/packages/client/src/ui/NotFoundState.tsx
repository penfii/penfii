import { Link } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'

/**
 * `escape` is for a page that has no navigation around it (an unknown address, a workspace the
 * person is not in): without a way out they would be stuck on a bare message.
 */
export function NotFoundState({ escape = false }: { escape?: boolean }) {
  const { t } = useI18n()
  return (
    <div className="not-found" role="status" data-state="not-found">
      {t('notFound')}
      {escape ? (
        <p>
          <Link to="/">{t('notFound.home')}</Link>
        </p>
      ) : null}
    </div>
  )
}
