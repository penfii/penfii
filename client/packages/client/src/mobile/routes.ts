import type { MessageKey } from '../i18n/messages'
import { activeArea, type NavArea } from '../shell/navAreas'

/**
 * The phone's idea of "where am I", in one place. The bottom tabs are `NAV_AREAS`; this module says,
 * for a location, which tab is lit, whether the screen is a **root** (the tab's context list, with the
 * tabs shown) or a **push** (content on a full screen with a back bar), and what the bar is called.
 *
 * A location is a root when its path is the tab's root path and nothing in the query asks for content.
 * Which query keys ask for content is declared per tab below, next to the path, so a page that starts
 * to open its content through a new key is a one-line change here and nowhere else.
 */
const ROOTS: Record<NavArea, { path: string; /** Query keys whose presence means "show this content". */ content: readonly string[] }> = {
  home: { path: '', content: [] },
  inbox: { path: '/inbox', content: ['view', 'type', 'unread', 'box'] },
  chat: { path: '/chat', content: [] },
  projects: { path: '/projects', content: [] },
  docs: { path: '/docs', content: ['doc', 'draft'] },
  teams: { path: '/directory', content: [] },
  search: { path: '/search', content: ['q'] },
  settings: { path: '/settings', content: [] },
}

/** Open a detail over any page (a Ticket panel, a create form). */
const PANEL_KEY = 'panel'
/**
 * Added by the shell when a link inside a root list leads to a location that would still be a root
 * (for example Teams → Organization is the root path itself): the choice must still open its content.
 */
export const PUSH_KEY = 'screen'

/** Pages below the root that are not part of a tab's own tree. */
const VAULT_PATHS = ['/saved', '/drafts']

export type TitleSpec =
  | { kind: 'key'; key: MessageKey }
  | { kind: 'project'; id: string }
  | { kind: 'conversation'; id: string }
  | { kind: 'doc'; id: string; projectId: string | null }
  | { kind: 'sprint'; id: string; projectId: string }

export type Screen = {
  area: NavArea
  root: boolean
  /** The path below `/w/:workspaceId`, without a trailing slash. */
  rest: string
  vault: boolean
  title: TitleSpec | null
}

const SETTINGS_TITLES: Record<string, MessageKey> = {
  '/settings/account': 'settings.account',
  '/settings/notifications': 'settings.notifications',
  '/settings/appearance': 'settings.appearance',
  '/settings/apps': 'settings.apps',
  '/settings/admin': 'admin.section.general',
  '/settings/admin/members': 'admin.section.members',
  '/settings/admin/teams': 'admin.section.teams',
  '/settings/admin/invitations': 'admin.section.invitations',
  '/settings/admin/recovery': 'admin.section.recovery',
  '/settings/admin/integrations': 'admin.section.integrations',
  '/settings/admin/audit': 'admin.section.audit',
}

export function resolveScreen(workspaceId: string, section: string, pathname: string, search: string): Screen {
  const params = new URLSearchParams(search)
  const area = activeArea(section, pathname)
  const rest = pathname.replace(`/w/${workspaceId}`, '').replace(/\/$/, '')
  const rule = ROOTS[area]
  const wantsContent = rule.content.some((key) => Boolean(params.get(key))) || params.has(PANEL_KEY) || params.has(PUSH_KEY)
  const vault = VAULT_PATHS.includes(rest)
  const root = rest === rule.path && !wantsContent && !vault

  let title: TitleSpec | null = null
  const sprintId = params.get('sprintDetail')
  const projectId = /^\/projects\/([^/]+)/.exec(rest)?.[1] ?? null
  const conversationId = /\/(?:c|chat)\/([^/]+)/.exec(rest)?.[1] ?? null
  const docId = params.get('doc')
  if (vault) title = { kind: 'key', key: 'mobile.vault' }
  else if (rest === '/my-work') title = { kind: 'key', key: 'shell.projects.tasks' }
  else if (rest === '/settings' && params.get(PUSH_KEY) === 'profile') title = { kind: 'key', key: 'settings.profile' }
  else if (SETTINGS_TITLES[rest]) title = { kind: 'key', key: SETTINGS_TITLES[rest] }
  else if (docId && docId !== 'new') title = { kind: 'doc', id: docId, projectId }
  else if (sprintId && projectId) title = { kind: 'sprint', id: sprintId, projectId }
  else if (conversationId) title = { kind: 'conversation', id: conversationId }
  else if (projectId) title = { kind: 'project', id: projectId }
  return { area, root, rest, vault, title }
}

/** Where a pushed screen of this area goes back to when there is no history to return to. */
export function rootPath(workspaceId: string, area: NavArea): string {
  return `/w/${workspaceId}${ROOTS[area].path}`
}

/** A link target inside a root list: if it would land on the root again, mark it so it opens its content. */
export function pushTarget(workspaceId: string, section: string, to: { pathname: string; search: string }): string {
  const next = resolveScreen(workspaceId, section, to.pathname, to.search)
  if (!next.root) return to.pathname + to.search
  const params = new URLSearchParams(to.search)
  params.set(PUSH_KEY, '1')
  return `${to.pathname}?${params.toString()}`
}

