/**
 * Links to places inside Penfii. A link is saved as the URL the person pasted; only how it is drawn
 * changes, so nothing here touches stored text. Only the current origin counts as inside.
 */
export type InternalTarget =
  | { kind: 'channel'; workspaceId: string; conversationId: string; threadRootId: null }
  | { kind: 'thread'; workspaceId: string; conversationId: string; threadRootId: string }
  | { kind: 'project'; workspaceId: string; projectId: string }
  | { kind: 'doc'; workspaceId: string; docId: string }
  | { kind: 'ticket'; workspaceId: string; ticketId: string; projectId: string }

const ID = '[^/?#]+'

/** The target of an in-app URL, or null for anything else (another site, another origin, a page that is not a target). */
export function parseInternalLink(href: string, origin: string): InternalTarget | null {
  let url: URL
  try {
    url = new URL(href)
  } catch {
    return null
  }
  if (url.origin !== origin) return null
  const path = url.pathname.replace(/\/+$/, '')
  const workspace = new RegExp(`^/w/(${ID})(/.*)?$`).exec(path)
  if (!workspace) return null
  const workspaceId = decodeURIComponent(workspace[1])
  const rest = workspace[2] ?? ''
  let match = new RegExp(`^/c/(${ID})/t/(${ID})$`).exec(rest)
  if (match) return { kind: 'thread', workspaceId, conversationId: match[1], threadRootId: match[2] }
  match = new RegExp(`^/c/(${ID})$`).exec(rest)
  if (match) return { kind: 'channel', workspaceId, conversationId: match[1], threadRootId: null }
  match = new RegExp(`^/projects/(${ID})/chat/(${ID})/t/(${ID})$`).exec(rest)
  if (match) return { kind: 'thread', workspaceId, conversationId: match[2], threadRootId: match[3] }
  match = new RegExp(`^/projects/(${ID})/chat/(${ID})$`).exec(rest)
  if (match) return { kind: 'channel', workspaceId, conversationId: match[2], threadRootId: null }
  if (rest === '/docs') {
    const doc = url.searchParams.get('doc')
    return doc && doc !== 'new' ? { kind: 'doc', workspaceId, docId: doc } : null
  }
  match = new RegExp(`^/projects/(${ID})/work$`).exec(rest)
  if (match) {
    const panel = /^ticket:(.+)$/.exec(url.searchParams.get('panel') ?? '')
    return panel ? { kind: 'ticket', workspaceId, projectId: match[1], ticketId: panel[1] } : null
  }
  match = new RegExp(`^/projects/(${ID})$`).exec(rest)
  if (match) return { kind: 'project', workspaceId, projectId: match[1] }
  return null
}

/** True when the link text is the URL itself (nothing the person wrote), so a title may stand in for it. */
export function isBareUrlText(text: string, href: string): boolean {
  const strip = (value: string) => value.trim().replace(/\/+$/, '')
  return strip(text) === strip(href)
}
