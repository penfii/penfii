import type { MessageKey } from '../../i18n/messages'

/**
 * The pages of Workspace administration, in the order the settings column and the in-page strip list
 * them. `general` is the administration's own path; the others are below it.
 */
export const ADMIN_SECTIONS = ['general', 'members', 'teams', 'invitations', 'recovery', 'integrations', 'audit'] as const
export type AdminSection = (typeof ADMIN_SECTIONS)[number]

export const adminSectionPath = (section: AdminSection): string => (section === 'general' ? '/admin' : `/admin/${section}`)
export const adminSectionLabel = (section: AdminSection): MessageKey => `admin.section.${section}` as MessageKey

export function parseAdminSection(value: string | undefined): AdminSection {
  return ADMIN_SECTIONS.find((section) => section === value) ?? 'general'
}
