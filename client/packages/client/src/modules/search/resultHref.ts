import type { SavedKind, SearchResult, SearchTab } from '@penfii/contracts'

/** The kinds a search can be narrowed to, in the order the Search sidebar lists them. */
export const SEARCH_TABS: SearchTab[] = ['all', 'message', 'ticket', 'doc', 'file', 'space', 'view', 'person']

/** Where a search result opens. */
export function resultHref(workspaceId: string, result: SearchResult): string | null {
  const base = `/w/${workspaceId}`
  const f = result.fields
  const text = (key: string): string | null => (typeof f[key] === 'string' ? (f[key] as string) : null)
  switch (result.kind) {
    case 'ticket': {
      const project = text('projectId')
      return project ? `${base}/projects/${project}/work?panel=ticket:${result.id}` : null
    }
    case 'message':
    case 'file': {
      const conversation = text('conversationId')
      if (!conversation) {
        // A file on a Ticket opens the Ticket.
        const project = text('projectId')
        const ticket = text('ticketId')
        return result.kind === 'file' && project && ticket ? `${base}/projects/${project}/work?panel=ticket:${ticket}` : null
      }
      const root = text('threadRootId')
      return root ? `${base}/c/${conversation}/t/${root}` : `${base}/c/${conversation}`
    }
    case 'space': {
      const kind = text('spaceKind')
      if (kind === 'PROJECT') return `${base}/projects/${result.id}`
      if (kind === 'TEAM') return `${base}/teams/${result.id}`
      const project = text('projectId')
      return project ? `${base}/projects/${project}/chat/${result.id}` : `${base}/c/${result.id}`
    }
    case 'doc':
      return `${base}/docs?doc=${result.id}`
    case 'view': {
      const target = text('targetKind')
      if (target === 'TEAM' && text('teamId')) return `${base}/teams/${text('teamId')}?v=${result.id}`
      if (target === 'WORKSPACE') return `${base}/my-work?v=${result.id}`
      const project = text('projectId')
      return project ? `${base}/projects/${project}/work?v=${result.id}` : null
    }
    case 'person':
    default:
      return null
  }
}

/**
 * What a result can be starred as: a kind Favorites supports and an id the result really carries.
 * Messages, files and people are not Favorites; a Project is one only for its participants (`participates`),
 * since its inside, and a shortcut to it, is theirs.
 */
export function favoriteTarget(result: SearchResult, participates: (projectId: string) => boolean): { kind: SavedKind; targetId: string } | null {
  switch (result.kind) {
    case 'ticket':
      return { kind: 'TICKET', targetId: result.id }
    case 'doc':
      return { kind: 'DOC', targetId: result.id }
    case 'view':
      return { kind: 'VIEW', targetId: result.id }
    case 'space': {
      const kind = result.fields.spaceKind
      if (kind === 'PROJECT') return participates(result.id) ? { kind: 'PROJECT', targetId: result.id } : null
      if (kind === 'TEAM') return { kind: 'TEAM', targetId: result.id }
      if (kind === 'CHANNEL') return { kind: 'CHANNEL', targetId: result.id }
      return null
    }
    case 'message':
    case 'file':
    case 'person':
      return null
  }
}
