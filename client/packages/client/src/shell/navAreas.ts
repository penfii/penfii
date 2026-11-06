import type { ComponentType } from 'react'
import type { MessageKey } from '../i18n/messages'
import { IconChat, IconDocs, IconHome, IconInbox, IconProjects, IconSearch, IconSettings, IconTeams } from './icons'

/** The Global Nav of the product: one definition, read by the desktop rail and the phone's bottom tabs. */
export type NavArea = 'home' | 'inbox' | 'chat' | 'projects' | 'docs' | 'teams' | 'search' | 'settings'

export const NAV_AREAS: ReadonlyArray<{ id: NavArea; /** Path below `/w/:workspaceId`. */ path: string; label: MessageKey; Icon: ComponentType }> = [
  { id: 'home', path: '', label: 'nav.home', Icon: IconHome },
  { id: 'inbox', path: '/inbox', label: 'nav.inbox', Icon: IconInbox },
  { id: 'chat', path: '/chat', label: 'nav.chat', Icon: IconChat },
  { id: 'projects', path: '/projects', label: 'nav.projects', Icon: IconProjects },
  { id: 'docs', path: '/docs', label: 'nav.docs', Icon: IconDocs },
  { id: 'teams', path: '/directory', label: 'nav.teams', Icon: IconTeams },
  { id: 'search', path: '/search', label: 'nav.search', Icon: IconSearch },
  { id: 'settings', path: '/settings', label: 'nav.settings', Icon: IconSettings },
]

/** The area a page belongs to. A conversation opened from anywhere keeps Chat lit. */
export function activeArea(section: string, pathname: string): NavArea {
  if (section === 'chat' || pathname.includes('/c/')) return 'chat'
  return section === 'account' ? 'settings' : (section as NavArea)
}
