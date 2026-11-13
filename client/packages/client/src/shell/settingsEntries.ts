import type { MessageKey } from '../i18n/messages'
import { ADMIN_SECTIONS, adminSectionLabel, adminSectionPath } from '../modules/admin/adminSections'

/** The Settings entries of the product: one list, read by the desktop settings column and the phone's settings root. */
export type SettingsEntry = { id: string; /** Path below `/w/:workspaceId/settings`. */ path: string; label: MessageKey; /** Lit only on exactly this path, not on the pages below it. */ end?: boolean }
export type SettingsGroup = { id: 'mine' | 'workspace'; heading: MessageKey; entries: readonly SettingsEntry[] }

export function settingsGroups(isManager: boolean): SettingsGroup[] {
  const groups: SettingsGroup[] = [
    {
      id: 'mine',
      heading: 'settings.group.mine',
      entries: [
        { id: 'profile', path: '', label: 'settings.profile' },
        { id: 'account', path: '/account', label: 'settings.account' },
        { id: 'notifications', path: '/notifications', label: 'settings.notifications' },
        { id: 'appearance', path: '/appearance', label: 'settings.appearance' },
        { id: 'apps', path: '/apps', label: 'settings.apps' },
      ],
    },
  ]
  if (isManager) groups.push({ id: 'workspace', heading: 'settings.group.workspace', entries: ADMIN_SECTIONS.map((section) => ({ id: `admin-${section}`, path: adminSectionPath(section), label: adminSectionLabel(section), end: true })) })
  return groups
}
