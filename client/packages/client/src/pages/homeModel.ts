import type { DocNode, Dm, Channel, Project, ScopedTicket } from '@penfii/contracts'

/** Tickets that still need doing. */
export function isOpenTicket(ticket: ScopedTicket): boolean {
  return ticket.stateCategory === 'NOT_STARTED' || ticket.stateCategory === 'IN_PROGRESS'
}

/** The calendar day of a moment in the viewer's own zone, `YYYY-MM-DD`. */
export function localDay(value: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${String(value.getFullYear())}-${pad(value.getMonth() + 1)}-${pad(value.getDate())}`
}

/** My open Tickets, the ones that are due first, then the ones touched most recently. */
export function toDoList(tickets: ScopedTicket[]): ScopedTicket[] {
  return tickets
    .filter(isOpenTicket)
    .sort((a, b) => {
      if (a.dueOn && b.dueOn && a.dueOn !== b.dueOn) return a.dueOn < b.dueOn ? -1 : 1
      if (a.dueOn && !b.dueOn) return -1
      if (!a.dueOn && b.dueOn) return 1
      return b.updatedAt.localeCompare(a.updatedAt)
    })
}

/** My open Tickets that are due today or whose planned schedule covers today. */
export function todayList(tickets: ScopedTicket[], today: string): ScopedTicket[] {
  return tickets.filter((ticket) => {
    if (!isOpenTicket(ticket)) return false
    if (ticket.dueOn === today) return true
    const start = ticket.schedule?.plannedStartAt
    if (!start) return false
    const from = localDay(new Date(start))
    const end = ticket.schedule?.plannedEndAt
    const to = end ? localDay(new Date(end)) : from
    return from <= today && today <= to
  })
}

export type RecentItem = { id: string; title: string; to: string; at: string | null; hint?: string }

/** Documents, most recently changed first. */
export function recentDocs(nodes: DocNode[], workspaceId: string, limit: number): RecentItem[] {
  return [...nodes]
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .slice(0, limit)
    .map((node) => ({ id: node.id, title: node.title, to: `/w/${workspaceId}/docs?doc=${node.id}`, at: node.updatedAt }))
}

/**
 * Conversations by their last message. Direct messages carry that time; a Channel carries only an unread
 * count, so a Channel with unread messages follows the dated ones.
 */
export function recentChats(dms: Dm[], channels: Channel[], workspaceId: string, limit: number): RecentItem[] {
  const dated = dms
    .filter((dm) => dm.lastMessageAt)
    .sort((a, b) => (b.lastMessageAt ?? '').localeCompare(a.lastMessageAt ?? ''))
    .map((dm): RecentItem => ({
      id: dm.id,
      title: dm.peers.map((peer) => peer.displayName).join(', '),
      to: `/w/${workspaceId}/c/${dm.conversationId}`,
      at: dm.lastMessageAt,
      hint: dm.lastMessagePreview ?? undefined,
    }))
  const unread = channels
    .filter((channel) => channel.unreadCount > 0)
    .map((channel): RecentItem => ({
      id: channel.id,
      title: `# ${channel.name}`,
      to: channel.projectId ? `/w/${workspaceId}/projects/${channel.projectId}/chat/${channel.conversationId}` : `/w/${workspaceId}/c/${channel.conversationId}`,
      at: null,
    }))
  return [...dated, ...unread].slice(0, limit)
}

/** Projects, in the order of the latest change to my Tickets in them. */
export function recentProjects(tickets: ScopedTicket[], projects: Project[], workspaceId: string, limit: number): RecentItem[] {
  const latest = new Map<string, string>()
  for (const ticket of tickets) {
    const seen = latest.get(ticket.projectId)
    if (!seen || ticket.updatedAt > seen) latest.set(ticket.projectId, ticket.updatedAt)
  }
  return projects
    .filter((project) => latest.has(project.id) && project.participating !== false)
    .sort((a, b) => (latest.get(b.id) ?? '').localeCompare(latest.get(a.id) ?? ''))
    .slice(0, limit)
    .map((project) => ({ id: project.id, title: project.name, to: `/w/${workspaceId}/projects/${project.id}`, at: latest.get(project.id) ?? null }))
}
