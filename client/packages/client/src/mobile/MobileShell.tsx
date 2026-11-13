import { useRef, type MouseEvent } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { useOnline } from './useOnline'
import { PushBar } from './PushBar'
import { TabBar } from './TabBar'
import { RootBar } from './RootBar'
import { useChromeCollapse, useKeyboardViewport } from './useChrome'
import { ResumeBand, SearchRoot, SettingsRoot } from './RootScreens'
import { pushTarget, resolveScreen, rootPath } from './routes'
import { useScreenTitle } from './useScreenTitle'
import { useDemotedHeadings } from './useDemotedHeadings'
import { useLeaveOverlay, useTrackTrail } from '../shell/overlayTrail'
import { useI18n } from '../i18n/I18nProvider'
import { ContextSidebar } from '../shell/ContextSidebar'
import { HostSlot, type ShellSlots } from '../shell/stableSlots'
import { NAV_AREAS } from '../shell/navAreas'
import type { ShellSection } from '../shell/WorkspaceShell'
import './mobile.css'

/**
 * The phone shell. The bottom tabs are the Global Nav and show on roots only. A tab's root is its
 * context list on a full screen; choosing an item pushes the content as another full screen with a
 * top bar (back, title). The desktop shell is untouched and is used above the phone width.
 */
export function MobileShell({ section, workspaceId, slots }: { section: ShellSection; workspaceId: string; slots: ShellSlots }) {
  const { t } = useI18n()
  const location = useLocation()
  const navigate = useNavigate()
  const online = useOnline()

  const screen = resolveScreen(workspaceId, section, location.pathname, location.search)
  const { area, root, vault } = screen
  const sectionLabel = t(NAV_AREAS.find((item) => item.id === area)?.label ?? 'nav.home')
  const title = useScreenTitle(workspaceId, screen.title, sectionLabel)
  const main = useRef<HTMLElement>(null)
  useDemotedHeadings(main)
  const { collapsed, onScroll } = useChromeCollapse(location.pathname + location.search)
  const keyboard = useKeyboardViewport()
  useTrackTrail()
  const leave = useLeaveOverlay()
  // A Thread is a detail of its channel: its bar says so and Back closes it (the channel's own bar is below it).
  const threadBase = /^(.*)\/t\/[^/]+$/.exec(location.pathname)?.[1] ?? null
  // Home is its own start screen; every other root is the context list of the tab.
  const showList = root && area !== 'home'

  /**
   * A link in a root list is a choice. When it would land on the root again (Teams → Organization is the
   * root path itself) the target is marked so that the choice opens its content instead of the list.
   */
  function onListClick(event: MouseEvent<HTMLElement>) {
    if (!root || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
    const link = (event.target as HTMLElement).closest('a[href]')
    if (!(link instanceof HTMLAnchorElement) || link.target === '_blank' || link.hasAttribute('download')) return
    const url = new URL(link.href, window.location.href)
    if (url.origin !== window.location.origin) return
    const target = pushTarget(workspaceId, section, { pathname: url.pathname, search: url.search })
    if (target === url.pathname + url.search) return
    event.preventDefault()
    event.stopPropagation()
    void navigate(target)
  }

  return (
    <>
      <div className={['mobile-shell', root ? 'is-root' : 'is-push', online ? '' : 'is-offline'].filter(Boolean).join(' ')} data-section={area} data-thread={threadBase ? '' : undefined} data-chrome={collapsed || keyboard ? 'collapsed' : undefined} data-keyboard={keyboard ? '' : undefined}>
        {online ? null : <p className="mobile-offline" role="status">{t('mobile.offline')}</p>}
        {root ? (
          <RootBar workspaceId={workspaceId} area={area} title={sectionLabel} />
        ) : threadBase ? (
          <PushBar fallback={threadBase} title={t('thread.title')} backLabel={title} onBack={() => leave(threadBase)} />
        ) : (
          <PushBar fallback={rootPath(workspaceId, vault ? 'home' : area)} title={title} backLabel={vault ? t('nav.home') : sectionLabel} />
        )}
        <main ref={main} className="mobile-main" onClickCapture={onListClick} onScrollCapture={onScroll}>
          {showList && area === 'search' ? (
            <SearchRoot workspaceId={workspaceId} />
          ) : showList && area === 'settings' ? (
            <SettingsRoot workspaceId={workspaceId} />
          ) : showList ? (
            <>
              {area === 'chat' ? <ResumeBand workspaceId={workspaceId} /> : null}
              <ContextSidebar workspaceId={workspaceId} section={area} />
            </>
          ) : (
            <HostSlot host={slots.page} />
          )}
        </main>
        <HostSlot host={slots.detail} />
        {root ? <TabBar workspaceId={workspaceId} active={area} /> : null}
      </div>
    </>
  )
}
