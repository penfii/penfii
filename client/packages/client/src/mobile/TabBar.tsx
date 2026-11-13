import { NavLink } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import type { MessageKey } from '../i18n/messages'
import { NAV_AREAS, type NavArea } from '../shell/navAreas'
import { useUnread } from '../modules/attention/useUnread'

/** The six working areas as bottom tabs, each one sixth of the width (so none is cut off, even at 320px). Search and Settings are in the root app bar. */
/** A shorter word for a tab whose name does not fit a sixth of a 320px screen; the full name stays its accessible name. */
const SHORT: Partial<Record<NavArea, MessageKey>> = { inbox: 'mobile.tab.inbox' }
const TAB_AREAS: readonly NavArea[] = ['home', 'inbox', 'chat', 'projects', 'docs', 'teams']

export function TabBar({ workspaceId, active }: { workspaceId: string; active: NavArea }) {
  const { t } = useI18n()
  const unread = useUnread(workspaceId)
  const count = unread.capped ? '99+' : String(unread.count)
  return (
    <nav className="mobile-tabs" aria-label={t('mobile.tabs')}>
      {NAV_AREAS.filter((area) => TAB_AREAS.includes(area.id)).map(({ id, path, label, Icon }) => (
        <NavLink key={id} to={`/w/${workspaceId}${path}`} end={id === 'home'} className={id === active ? 'mobile-tab active' : 'mobile-tab'} aria-current={id === active ? 'page' : undefined}>
          <Icon />
          <span className="mobile-tab-label">
            {SHORT[id] && t(SHORT[id]) !== t(label) ? <><span className="visually-hidden">{t(label)}</span><span aria-hidden="true">{t(SHORT[id])}</span></> : t(label)}
          </span>
          {id === 'inbox' && unread.count > 0 ? (
            <b className="mobile-tab-badge">
              <span aria-hidden="true">{count}</span>
              <span className="visually-hidden">{t('inbox.unreadBadge', { count })}</span>
            </b>
          ) : null}
        </NavLink>
      ))}
    </nav>
  )
}
