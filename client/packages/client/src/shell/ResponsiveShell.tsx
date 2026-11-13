import { useCallback, useRef, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useLocation, useParams } from 'react-router'
import { useIsMobile } from '../design/useIsMobile'
import { CommandPalette } from '../modules/attention/CommandPalette'
import { useUserSocket } from '../modules/attention/useUserSocket'
import { ShortcutHelpProvider } from '../modules/help/ShortcutHelp'
import { usePushNavigation } from '../modules/push/registerServiceWorker'
import { RichTextEnvProvider } from '../modules/references/RichTextEnvProvider'
import { HomeShortcuts } from '../mobile/RootScreens'
import { MobileShell } from '../mobile/MobileShell'
import { resolveScreen } from '../mobile/routes'
import { VaultScreen } from '../mobile/VaultScreen'
import { NotFoundState } from '../ui/NotFoundState'
import { useActiveWorkspace } from './account'
import { CreateProvider } from './create'
import { DetailPanel } from './DetailPanel'
import { usePaneWidth } from './usePaneWidth'
import { useSlotElements } from './stableSlots'
import { WorkspaceShell, type ShellSection } from './WorkspaceShell'

/**
 * The application shell. It owns everything that must outlive the window being resized across the phone width: the
 * providers, the realtime socket, the page and the Detail Panel. The desktop frame (`WorkspaceShell`) and the phone
 * frame (`MobileShell`) only draw the chrome around them and say where the page and the panel go. A viewport change is
 * not a navigation, so it does not remount the page (an unsaved document, a ticket description, a form).
 */
export function ResponsiveShell({
  section,
  children,
  workspaceId: workspaceIdProp,
  desktopOnly = false,
}: {
  section: ShellSection
  children: ReactNode
  workspaceId?: string
  /** A page the phone frame does not know (the account page) keeps the desktop frame. */
  desktopOnly?: boolean
}) {
  const params = useParams()
  const workspaceId = params.workspaceId ?? workspaceIdProp ?? ''
  const location = useLocation()
  const phone = useIsMobile() && !desktopOnly
  const slots = useSlotElements()
  const { space, notFound } = useActiveWorkspace(workspaceId)
  useUserSocket()
  usePushNavigation()

  const closeDetail = useRef<() => void>(() => {})
  const bindDetailClose = useCallback((close: () => void) => {
    closeDetail.current = close
  }, [])
  const ticketPane = usePaneWidth('ticket', { threadOpen: false, detailOpen: false, onTooNarrow: () => closeDetail.current() })

  if (!workspaceId || notFound) return <NotFoundState escape />

  // On a phone a root screen shows its list instead of the page, and the vault its own screen.
  const screen = resolveScreen(workspaceId, section, location.pathname, location.search)
  const phoneShowsList = phone && screen.root && screen.area !== 'home'
  const phoneRoot = phone && screen.root && screen.area === 'home'

  return (
    <ShortcutHelpProvider>
      <RichTextEnvProvider workspaceId={workspaceId} space={space.data}>
        <CreateProvider workspaceId={workspaceId} space={space.data}>
          {phone ? (
            <MobileShell section={section} workspaceId={workspaceId} slots={slots} />
          ) : (
            <WorkspaceShell section={section} workspaceId={workspaceId} slots={slots} ticketPane={ticketPane} />
          )}
          {phoneShowsList ? null : createPortal(
            <>
              {phoneRoot ? <HomeShortcuts workspaceId={workspaceId} /> : null}
              {phone && screen.vault ? <VaultScreen /> : children}
            </>,
            slots.page,
          )}
          {createPortal(<DetailPanel ticketPane={phone ? undefined : ticketPane} bindClose={phone ? undefined : bindDetailClose} />, slots.detail)}
          <CommandPalette workspaceId={workspaceId} />
        </CreateProvider>
      </RichTextEnvProvider>
    </ShortcutHelpProvider>
  )
}
