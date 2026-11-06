import { useEffect, useRef } from 'react'
import { NavLink } from 'react-router'
import { useI18n } from '../i18n/I18nProvider'
import { NAV_AREAS, type NavArea } from '../shell/navAreas'
import { useUnread } from '../modules/attention/useUnread'

/** The Global Nav as bottom tabs. The tabs scroll sideways, so the active one is brought into view when it changes. */
export function TabBar({ workspaceId, active }: { workspaceId: string; active: NavArea }) {
  const { t } = useI18n()
  const unread = useUnread(workspaceId)
  const current = useRef<HTMLAnchorElement>(null)
  useEffect(() => {
    // `scrollIntoView` moves only the tab strip when the page itself cannot scroll, and keeps the vertical position.
    current.current?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
  }, [active])
  const count = unread.capped ? '99+' : String(unread.count)
  return (
    <nav className="mobile-tabs" aria-label={t('mobile.tabs')}>
      {NAV_AREAS.map(({ id, path, label, Icon }) => (
        <NavLink key={id} ref={id === active ? current : undefined} to={`/w/${workspaceId}${path}`} end={id === 'home'} className={id === active ? 'mobile-tab active' : 'mobile-tab'} aria-current={id === active ? 'page' : undefined}>
          <Icon />
          <span>{t(label)}</span>
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
