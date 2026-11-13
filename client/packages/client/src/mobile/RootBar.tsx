import { NavLink } from 'react-router'
import { Avatar } from '../design/ui'
import { IconSearch } from '../shell/icons'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import type { NavArea } from '../shell/navAreas'

/**
 * The app bar of a root screen: the area's name, Search and the account. The bottom tabs hold the six
 * working areas; Search and Settings (with the account actions, logout among them) live here so that
 * they are always one tap away without a seventh and eighth tab.
 */
export function RootBar({ workspaceId, area, title }: { workspaceId: string; area: NavArea; title: string }) {
  const { t } = useI18n()
  const user = useSession().data
  return (
    <header className="mobile-bar is-root">
      <h1>{title}</h1>
      <div className="mobile-bar-end">
        <NavLink to={`/w/${workspaceId}/search`} className="mobile-bar-button" aria-label={t('nav.search')} aria-current={area === 'search' ? 'page' : undefined}>
          <IconSearch />
        </NavLink>
        <NavLink to={`/w/${workspaceId}/settings`} className="mobile-bar-button" aria-label={t('account.title')} aria-current={area === 'settings' ? 'page' : undefined}>
          <Avatar name={user?.displayName || t('account.title')} userId={user?.id} hasAvatar={user?.hasAvatar} />
        </NavLink>
      </div>
    </header>
  )
}
