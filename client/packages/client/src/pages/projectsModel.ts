import type { FavoriteItem, Project } from '@penfii/contracts'

/** A Project the person takes part in. Everyone in the Workspace finds a Project; only its participants open it. */
export const takesPart = (project: Project): boolean => project.participating !== false

export type MyProjects = {
  /** Participating Projects the person starred, in the order of their favorites. */
  favorites: Project[]
  /** The other participating Projects, by name. */
  others: Project[]
}

/**
 * Participation, favorites and recent visits are three different things. This is only the first two: the Projects a
 * person takes part in, with the starred ones first and no Project twice. A starred Project the person has left (or
 * never joined) is not "mine", so it does not appear here; it stays in Saved.
 */
export function myProjects(projects: Project[], favorites: FavoriteItem[]): MyProjects {
  const mine = projects.filter(takesPart)
  const byId = new Map(mine.map((project) => [project.id, project]))
  const starred = favorites
    .filter((item) => item.kind === 'PROJECT')
    .sort((a, b) => a.position - b.position)
    .flatMap((item) => {
      const project = byId.get(item.targetId)
      return project ? [project] : []
    })
  const seen = new Set<string>()
  const first = starred.filter((project) => (seen.has(project.id) ? false : (seen.add(project.id), true)))
  const others = mine.filter((project) => !seen.has(project.id)).sort((a, b) => a.name.localeCompare(b.name))
  return { favorites: first, others }
}

export type JoinFilter = 'all' | 'in' | 'out'

export function joinFilter(value: string | null): JoinFilter {
  return value === 'in' || value === 'out' ? value : 'all'
}

/** The name or the Ticket prefix contains the text, ignoring case. */
export function matchesProject(project: Project, query: string): boolean {
  const text = query.trim().toLowerCase()
  if (text === '') return true
  return project.name.toLowerCase().includes(text) || (project.ticketPrefix ?? '').toLowerCase().includes(text)
}

/** The Projects the "all Projects" view shows: filtered by participation and by the text, by name. */
export function browseProjects(projects: Project[], join: JoinFilter, query: string): Project[] {
  return projects
    .filter((project) => (join === 'in' ? takesPart(project) : join === 'out' ? !takesPart(project) : true))
    .filter((project) => matchesProject(project, query))
    .sort((a, b) => a.name.localeCompare(b.name))
}

export type SwitcherSections = { favorites: Project[]; recent: Project[]; others: Project[] }

/** How many recently visited Projects the switcher lists. */
export const RECENT_PROJECT_LIMIT = 5

/**
 * What the Project switcher lists, for a search text: the Projects the person takes part in, starred ones first (in the
 * order of their favorites), then the ones really visited lately (newest first), then the rest by name. No Project is
 * listed twice, and a recent visit never stars a Project. Recent ids that are no longer a participating Project (the
 * person left it, it was removed) are simply not listed.
 */
export function switcherSections(projects: Project[], favorites: FavoriteItem[], recentIds: string[], query: string): SwitcherSections {
  const mine = myProjects(projects, favorites)
  const shown = (list: Project[]) => list.filter((project) => matchesProject(project, query))
  const taken = new Set(mine.favorites.map((project) => project.id))
  const byId = new Map(projects.filter(takesPart).map((project) => [project.id, project]))
  const recent: Project[] = []
  for (const id of recentIds) {
    const project = byId.get(id)
    if (!project || taken.has(id) || recent.length >= RECENT_PROJECT_LIMIT) continue
    taken.add(id)
    recent.push(project)
  }
  return { favorites: shown(mine.favorites), recent: shown(recent), others: shown(mine.others.filter((project) => !taken.has(project.id))) }
}

const RECENT_KEY = 'penfii.recentProjects.'

/** A device-local list: the Workspace and the person are part of the key, and nothing about it is sent to the server. */
export function readRecentProjects(workspaceId: string, userId: string): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(`${RECENT_KEY}${workspaceId}.${userId}`) ?? '[]') as unknown
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
  } catch {
    return []
  }
}

export function recordRecentProject(workspaceId: string, userId: string, projectId: string): void {
  try {
    // A few more than are shown: some may stop being participating Projects before they are read.
    const next = [projectId, ...readRecentProjects(workspaceId, userId).filter((id) => id !== projectId)].slice(0, RECENT_PROJECT_LIMIT * 4)
    localStorage.setItem(`${RECENT_KEY}${workspaceId}.${userId}`, JSON.stringify(next))
  } catch {
    // Without storage there is simply no recent list.
  }
}

/**
 * Where the switcher goes: the same kind of page in the target Project when it is one the person can use there, otherwise
 * the target's overview. Nothing that belongs to the current Project (a ticket, a document, a view, a filter) is carried.
 */
export function switchTarget(workspaceId: string, target: Project, section: string): string {
  const base = `/w/${workspaceId}/projects/${target.id}`
  if (section === 'work' || section === 'planning' || section === 'timeline' || section === 'docs' || section === 'chat') return `${base}/${section}`
  if (section === 'settings' && target.canManage === true) return `${base}/settings`
  return base
}
