import { queryOptions } from '@tanstack/react-query'
import type { LaterState, ProjectSchedule, ScopeLens, WatchTargetKind } from '@penfii/contracts'
import { api } from './api'
import { queryKeys } from './queryKeys'

const SCHEDULE_PAGES_MAX = 5

/** Every page of the Project's schedule list, merged: the Gantt draws one list. */
async function loadSchedule(projectId: string): Promise<ProjectSchedule> {
  let page = await api.getProjectSchedule(projectId, { limit: 1000 })
  const merged: ProjectSchedule = { ...page, items: [...page.items], edges: [...page.edges] }
  for (let count = 1; page.nextCursor && count < SCHEDULE_PAGES_MAX; count += 1) {
    page = await api.getProjectSchedule(projectId, { limit: 1000, cursor: page.nextCursor })
    merged.items.push(...page.items)
    for (const edge of page.edges) if (!merged.edges.some((known) => known.id === edge.id)) merged.edges.push(edge)
    merged.nextCursor = page.nextCursor
  }
  return merged
}

type Scope = { kind: 'TEAM'; teamId: string } | { kind: 'WORKSPACE' }

/**
 * One key and one fetch per server resource. Screens spread these into `useQuery` and add only what
 * differs there (`enabled`, polling), so every screen shares one cache entry for the same thing.
 */
export const queries = {
  // Session and server
  me: () => queryOptions({ queryKey: queryKeys.me, queryFn: () => api.me(), retry: false }),
  people: (workspaceId: string) => queryOptions({ queryKey: queryKeys.people(workspaceId), queryFn: () => api.listPeople(workspaceId) }),
  departments: (workspaceId: string) => queryOptions({ queryKey: queryKeys.departments(workspaceId), queryFn: () => api.listDepartments(workspaceId) }),
  identities: () => queryOptions({ queryKey: queryKeys.identities, queryFn: () => api.identities(), retry: false }),
  sessions: () => queryOptions({ queryKey: queryKeys.sessions, queryFn: () => api.sessions(), retry: false }),
  oauthGrants: () => queryOptions({ queryKey: queryKeys.oauthGrants, queryFn: () => api.oauthGrants(), retry: false }),
  oauthRequest: (id: string) => queryOptions({ queryKey: queryKeys.oauthRequest(id), queryFn: () => api.consentRequest(id), retry: false }),
  serverInfo: () => queryOptions({ queryKey: queryKeys.serverInfo, queryFn: () => api.serverInfo() }),
  authProviders: () => queryOptions({ queryKey: queryKeys.authProviders, queryFn: () => api.authProviders(), retry: false }),
  invitationPreview: (token: string) =>
    queryOptions({
      queryKey: queryKeys.invitationPreview(token),
      queryFn: () => api.previewInvitation(token),
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
    }),
  notificationPreferences: () => queryOptions({ queryKey: queryKeys.notificationPreferences, queryFn: () => api.getNotificationPreferences() }),
  mutedChannels: () => queryOptions({ queryKey: queryKeys.mutedChannels, queryFn: () => api.getMutedChannels() }),

  // Workspace
  workspaces: () => queryOptions({ queryKey: queryKeys.workspaces, queryFn: () => api.listWorkspaces() }),
  workspaceCreation: () => queryOptions({ queryKey: queryKeys.workspaceCreation, queryFn: () => api.workspaceCreationOpen() }),
  space: (workspaceId: string) => queryOptions({ queryKey: queryKeys.space(workspaceId), queryFn: () => api.getSpace(workspaceId) }),
  organization: (workspaceId: string, organizationId: string) =>
    queryOptions({ queryKey: queryKeys.organization(workspaceId, organizationId), queryFn: () => api.organizationDirectory(workspaceId, organizationId) }),
  mcpPolicy: (workspaceId: string) => queryOptions({ queryKey: queryKeys.mcpPolicy(workspaceId), queryFn: () => api.mcpPolicy(workspaceId), retry: false }),
  adminMembers: (workspaceId: string) => queryOptions({ queryKey: queryKeys.adminMembers(workspaceId), queryFn: () => api.listMembers(workspaceId) }),
  adminInvitations: (workspaceId: string) => queryOptions({ queryKey: queryKeys.adminInvitations(workspaceId), queryFn: () => api.listInvitations(workspaceId) }),
  teamProjects: (workspaceId: string, teamId: string) =>
    queryOptions({ queryKey: queryKeys.teamProjects(workspaceId, teamId), queryFn: () => api.listTeamProjects(workspaceId, teamId), enabled: teamId.length > 0 }),
  adminTeamMembers: (workspaceId: string, teamId: string) =>
    queryOptions({ queryKey: queryKeys.adminTeamMembers(workspaceId, teamId), queryFn: () => api.listTeamMembers(workspaceId, teamId) }),
  inboxUnread: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.inboxUnread(workspaceId), queryFn: () => api.getInbox(workspaceId, { state: 'unread', limit: 1 }) }),
  subscription: (kind: WatchTargetKind, targetId: string) =>
    queryOptions({ queryKey: queryKeys.subscription(kind, targetId), queryFn: () => api.getSubscription(kind, targetId) }),
  directorySearch: (workspaceId: string, query: string, types?: string) =>
    queryOptions({ queryKey: queryKeys.directorySearch(workspaceId, query, types ?? 'all'), queryFn: () => api.searchDirectory(workspaceId, { q: query, types }) }),
  palette: (workspaceId: string, query: string) =>
    queryOptions({ queryKey: queryKeys.palette(workspaceId, query), queryFn: () => api.search(workspaceId, { q: query, kind: 'all', limit: 6 }), retry: false }),

  // Personal
  drafts: (workspaceId: string) => queryOptions({ queryKey: queryKeys.drafts(workspaceId), queryFn: () => api.getDrafts(workspaceId) }),
  favorites: (workspaceId: string) => queryOptions({ queryKey: queryKeys.favorites(workspaceId), queryFn: () => api.getFavorites(workspaceId) }),
  favoriteFolders: (workspaceId: string) => queryOptions({ queryKey: queryKeys.favoriteFolders(workspaceId), queryFn: () => api.getFavoriteFolders(workspaceId) }),
  later: (workspaceId: string, state: LaterState) => queryOptions({ queryKey: queryKeys.later(workspaceId, state), queryFn: () => api.getLater(workspaceId, state) }),

  // Work
  homeWork: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.homeWork(workspaceId), queryFn: () => api.getScopeDataset(workspaceId, { kind: 'WORKSPACE' }, 'ASSIGNED') }),
  scopeDataset: (workspaceId: string, scope: Scope, lens: ScopeLens, projectIds: string[]) =>
    queryOptions({
      queryKey: queryKeys.scopeDataset(workspaceId, scope, lens, projectIds),
      queryFn: () => api.getScopeDataset(workspaceId, scope, lens, projectIds),
    }),
  projectBoard: (projectId: string) => queryOptions({ queryKey: queryKeys.projectBoard(projectId), queryFn: () => api.getProjectBoard(projectId) }),
  projectPlanning: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPlanning(projectId), queryFn: () => api.getProjectPlanning(projectId) }),
  projectLabels: (projectId: string) => queryOptions({ queryKey: queryKeys.projectLabels(projectId), queryFn: () => api.getProjectLabels(projectId) }),
  channelMembers: (channelId: string) => queryOptions({ queryKey: queryKeys.channelMembers(channelId), queryFn: () => api.getChannelMembers(channelId) }),
  projectMembers: (projectId: string) => queryOptions({ queryKey: queryKeys.projectMembers(projectId), queryFn: () => api.getProjectMembers(projectId) }),
  projectTypes: (projectId: string) => queryOptions({ queryKey: queryKeys.projectTypes(projectId), queryFn: () => api.getProjectTicketTypes(projectId) }),
  projectWorkflow: (projectId: string) => queryOptions({ queryKey: queryKeys.projectWorkflow(projectId), queryFn: () => api.getProjectWorkflow(projectId) }),
  projectPermissions: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPermissions(projectId), queryFn: () => api.getProjectPermissions(projectId) }),
  projectTeamMembers: (projectId: string, teamId: string) => queryOptions({ queryKey: queryKeys.projectTeamMembers(projectId, teamId), queryFn: () => api.getProjectTeamMembers(projectId, teamId) }),
  projectPins: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPins(projectId), queryFn: () => api.getProjectPins(projectId) }),
  projectTrash: (projectId: string) => queryOptions({ queryKey: queryKeys.projectTrash(projectId), queryFn: () => api.listProjectTrash(projectId) }),
  projectSchedule: (projectId: string) => queryOptions({ queryKey: queryKeys.projectSchedule(projectId), queryFn: () => loadSchedule(projectId) }),
  projectCalendar: (projectId: string) => queryOptions({ queryKey: queryKeys.projectCalendar(projectId), queryFn: () => api.getProjectCalendar(projectId) }),
  projectVelocity: (projectId: string) => queryOptions({ queryKey: queryKeys.projectVelocity(projectId), queryFn: () => api.getProjectVelocity(projectId) }),
  projectStatusSummary: (projectId: string) =>
    queryOptions({ queryKey: queryKeys.projectStatusSummary(projectId), queryFn: () => api.getProjectStatusSummary(projectId) }),
  sprintReport: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintReport(sprintId), queryFn: () => api.getSprintReport(sprintId) }),
  sprintResults: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintResults(sprintId), queryFn: () => api.getSprintResults(sprintId) }),
  sprintPreview: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintPreview(sprintId), queryFn: () => api.getSprintClosePreview(sprintId) }),
  ticket: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticket(ticketId), queryFn: () => api.getTicket(ticketId) }),
  ticketChildren: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketChildren(ticketId), queryFn: () => api.getTicketChildren(ticketId) }),
  ticketComments: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketComments(ticketId), queryFn: () => api.getTicketComments(ticketId) }),
  ticketAttachments: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketAttachments(ticketId), queryFn: () => api.getTicketAttachments(ticketId) }),
  ticketDependencies: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketDependencies(ticketId), queryFn: () => api.getTicketDependencies(ticketId) }),
  ticketRevision: (ticketId: string, revisionNo: number) =>
    queryOptions({ queryKey: queryKeys.ticketRevision(ticketId, revisionNo), queryFn: () => api.getTicketRevision(ticketId, revisionNo) }),

  // Docs
  docTree: (workspaceId: string, projectId?: string | null) =>
    queryOptions({ queryKey: queryKeys.docTree(workspaceId, projectId), queryFn: () => api.getDocTree(workspaceId, projectId ?? undefined) }),
  doc: (docId: string) => queryOptions({ queryKey: queryKeys.doc(docId), queryFn: () => api.getDoc(docId) }),
  docVersion: (docId: string, versionNo: number) => queryOptions({ queryKey: queryKeys.docVersion(docId, versionNo), queryFn: () => api.getDocVersion(docId, versionNo) }),
  docParticipants: (docId: string) => queryOptions({ queryKey: queryKeys.docParticipants(docId), queryFn: () => api.getDocParticipants(docId) }),
  docComments: (docId: string) => queryOptions({ queryKey: queryKeys.docComments(docId), queryFn: () => api.getDocComments(docId) }),
  docTemplates: (workspaceId: string) => queryOptions({ queryKey: queryKeys.docTemplates(workspaceId), queryFn: () => api.getDocTemplates(workspaceId) }),
}
