import { lazy, Suspense } from 'react'
import { Link, useLocation, useParams, useSearchParams } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { Favorites, Later } from '../modules/personal/SavedPage'

// Loaded on demand, like its own route, so the page keeps its own chunk.
const DraftsPage = lazy(async () => ({ default: (await import('../modules/personal/DraftsPage')).DraftsPage }))

type Segment = 'favorites' | 'later' | 'drafts'

/**
 * My library: Favorites, Later and Drafts are three separate lists under one push screen. They are
 * not merged: each keeps its own meaning (a shortcut, something to come back to, an unsent message).
 */
export function VaultScreen() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const location = useLocation()
  const [params] = useSearchParams()
  const segment: Segment = location.pathname.endsWith('/drafts') ? 'drafts' : params.get('tab') === 'later' ? 'later' : 'favorites'
  const base = `/w/${workspaceId}`
  const items: Array<[Segment, string, string]> = [
    ['favorites', t('saved.tab.favorites'), `${base}/saved`],
    ['later', t('saved.tab.later'), `${base}/saved?tab=later`],
    ['drafts', t('drafts.title'), `${base}/drafts`],
  ]
  return (
    <div className="mobile-vault">
      <nav className="mobile-segments" aria-label={t('mobile.vault')}>
        {items.map(([id, label, to]) => (
          <Link key={id} to={to} replace aria-current={segment === id ? 'page' : undefined} className={segment === id ? 'active' : undefined}>{label}</Link>
        ))}
      </nav>
      {segment === 'drafts' ? <Suspense fallback={<p className="hint" role="status">{t('loading')}</p>}><DraftsPage /></Suspense> : <div className="project-pad">{segment === 'favorites' ? <Favorites workspaceId={workspaceId} /> : <Later workspaceId={workspaceId} />}</div>}
    </div>
  )
}
