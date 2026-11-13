import { queryOptions } from '@tanstack/react-query'
import type { LaterState, ProjectSchedule, ScopeLens, WatchTargetKind } from '@penfii/contracts'
import { withSignal } from '@penfii/api-client'
import { api } from './api'
import { queryKeys } from './queryKeys'

const SCHEDULE_PAGES_MAX = 5

/** Every page of the Project's schedule list, merged: the Gantt draws one list. */
async function loadSchedule(projectId: string, signal: AbortSignal): Promise<ProjectSchedule> {
  let page = await withSignal(signal, () => api.getProjectSchedule(projectId, { limit: 1000 }))
  const merged: ProjectSchedule = { ...page, items: [...page.items], edges: [...page.edges] }
  for (let count = 1; page.nextCursor && count < SCHEDULE_PAGES_MAX; count += 1) {
    const cursor = page.nextCursor
    page = await withSignal(signal, () => api.getProjectSchedule(projectId, { limit: 1000, cursor }))
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
  me: () => queryOptions({ queryKey: queryKeys.me, queryFn: ({ signal }) => withSignal(signal, () => api.me()), retry: false }),
  people: (workspaceId: string) => queryOptions({ queryKey: queryKeys.people(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listPeople(workspaceId)) }),
  departments: (workspaceId: string) => queryOptions({ queryKey: queryKeys.departments(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listDepartments(workspaceId)) }),
  identities: () => queryOptions({ queryKey: queryKeys.identities, queryFn: ({ signal }) => withSignal(signal, () => api.identities()), retry: false }),
  sessions: () => queryOptions({ queryKey: queryKeys.sessions, queryFn: ({ signal }) => withSignal(signal, () => api.sessions()), retry: false }),
  oauthGrants: () => queryOptions({ queryKey: queryKeys.oauthGrants, queryFn: ({ signal }) => withSignal(signal, () => api.oauthGrants()), retry: false }),
  oauthRequest: (id: string) => queryOptions({ queryKey: queryKeys.oauthRequest(id), queryFn: ({ signal }) => withSignal(signal, () => api.consentRequest(id)), retry: false }),
  serverInfo: () => queryOptions({ queryKey: queryKeys.serverInfo, queryFn: ({ signal }) => withSignal(signal, () => api.serverInfo()) }),
  authProviders: () => queryOptions({ queryKey: queryKeys.authProviders, queryFn: ({ signal }) => withSignal(signal, () => api.authProviders()), retry: false }),
  invitationPreview: (token: string) =>
    queryOptions({
      queryKey: queryKeys.invitationPreview(token),
      queryFn: ({ signal }) => withSignal(signal, () => api.previewInvitation(token)),
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
    }),
  notificationPreferences: () => queryOptions({ queryKey: queryKeys.notificationPreferences, queryFn: ({ signal }) => withSignal(signal, () => api.getNotificationPreferences()) }),
  mutedChannels: () => queryOptions({ queryKey: queryKeys.mutedChannels, queryFn: ({ signal }) => withSignal(signal, () => api.getMutedChannels()) }),

  // Workspace
  workspaces: () => queryOptions({ queryKey: queryKeys.workspaces, queryFn: ({ signal }) => withSignal(signal, () => api.listWorkspaces()) }),
  workspaceCreation: () => queryOptions({ queryKey: queryKeys.workspaceCreation, queryFn: ({ signal }) => withSignal(signal, () => api.workspaceCreationOpen()) }),
  space: (workspaceId: string) => queryOptions({ queryKey: queryKeys.space(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getSpace(workspaceId)) }),
  organization: (workspaceId: string, organizationId: string) =>
    queryOptions({ queryKey: queryKeys.organization(workspaceId, organizationId), queryFn: ({ signal }) => withSignal(signal, () => api.organizationDirectory(workspaceId, organizationId)) }),
  mcpPolicy: (workspaceId: string) => queryOptions({ queryKey: queryKeys.mcpPolicy(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.mcpPolicy(workspaceId)), retry: false }),
  adminMembers: (workspaceId: string) => queryOptions({ queryKey: queryKeys.adminMembers(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listMembers(workspaceId)) }),
  adminInvitations: (workspaceId: string) => queryOptions({ queryKey: queryKeys.adminInvitations(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listInvitations(workspaceId)) }),
  adminProjectsWithoutAdmin: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.adminProjectsWithoutAdmin(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listProjectsWithoutAdmin(workspaceId)) }),
  adminChannelsWithoutManager: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.adminChannelsWithoutManager(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.listChannelsWithoutManager(workspaceId)) }),
  teamProjects: (workspaceId: string, teamId: string) =>
    queryOptions({ queryKey: queryKeys.teamProjects(workspaceId, teamId), queryFn: ({ signal }) => withSignal(signal, () => api.listTeamProjects(workspaceId, teamId)), enabled: teamId.length > 0 }),
  adminTeamMembers: (workspaceId: string, teamId: string) =>
    queryOptions({ queryKey: queryKeys.adminTeamMembers(workspaceId, teamId), queryFn: ({ signal }) => withSignal(signal, () => api.listTeamMembers(workspaceId, teamId)) }),
  inboxUnread: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.inboxUnread(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getInbox(workspaceId, { state: 'unread', limit: 1 })) }),
  subscription: (kind: WatchTargetKind, targetId: string) =>
    queryOptions({ queryKey: queryKeys.subscription(kind, targetId), queryFn: ({ signal }) => withSignal(signal, () => api.getSubscription(kind, targetId)) }),
  directorySearch: (workspaceId: string, query: string, types?: string) =>
    queryOptions({ queryKey: queryKeys.directorySearch(workspaceId, query, types ?? 'all'), queryFn: ({ signal }) => withSignal(signal, () => api.searchDirectory(workspaceId, { q: query, types })) }),
  palette: (workspaceId: string, query: string) =>
    queryOptions({ queryKey: queryKeys.palette(workspaceId, query), queryFn: ({ signal }) => withSignal(signal, () => api.search(workspaceId, { q: query, kind: 'all', limit: 6 })), retry: false }),

  // Personal
  drafts: (workspaceId: string) => queryOptions({ queryKey: queryKeys.drafts(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getDrafts(workspaceId)) }),
  favorites: (workspaceId: string) => queryOptions({ queryKey: queryKeys.favorites(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getFavorites(workspaceId)) }),
  favoriteFolders: (workspaceId: string) => queryOptions({ queryKey: queryKeys.favoriteFolders(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getFavoriteFolders(workspaceId)) }),
  later: (workspaceId: string, state: LaterState) => queryOptions({ queryKey: queryKeys.later(workspaceId, state), queryFn: ({ signal }) => withSignal(signal, () => api.getLater(workspaceId, state)) }),

  // Work
  homeWork: (workspaceId: string) =>
    queryOptions({ queryKey: queryKeys.homeWork(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getScopeDataset(workspaceId, { kind: 'WORKSPACE' }, 'ASSIGNED')) }),
  scopeDataset: (workspaceId: string, scope: Scope, lens: ScopeLens, projectIds: string[]) =>
    queryOptions({
      queryKey: queryKeys.scopeDataset(workspaceId, scope, lens, projectIds),
      queryFn: ({ signal }) => withSignal(signal, () => api.getScopeDataset(workspaceId, scope, lens, projectIds)),
    }),
  projectBoard: (projectId: string) => queryOptions({ queryKey: queryKeys.projectBoard(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectBoard(projectId)) }),
  projectPlanning: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPlanning(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectPlanning(projectId)) }),
  projectLabels: (projectId: string) => queryOptions({ queryKey: queryKeys.projectLabels(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectLabels(projectId)) }),
  projectPrivateChannels: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPrivateChannels(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.listProjectPrivateChannels(projectId)) }),
  channelMembers: (channelId: string) => queryOptions({ queryKey: queryKeys.channelMembers(channelId), queryFn: ({ signal }) => withSignal(signal, () => api.getChannelMembers(channelId)) }),
  projectMembers: (projectId: string) => queryOptions({ queryKey: queryKeys.projectMembers(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectMembers(projectId)) }),
  projectTypes: (projectId: string) => queryOptions({ queryKey: queryKeys.projectTypes(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectTicketTypes(projectId)) }),
  projectWorkflow: (projectId: string) => queryOptions({ queryKey: queryKeys.projectWorkflow(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectWorkflow(projectId)) }),
  projectPermissions: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPermissions(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectPermissions(projectId)) }),
  projectTeamMembers: (projectId: string, teamId: string) => queryOptions({ queryKey: queryKeys.projectTeamMembers(projectId, teamId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectTeamMembers(projectId, teamId)) }),
  projectPins: (projectId: string) => queryOptions({ queryKey: queryKeys.projectPins(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectPins(projectId)) }),
  projectTrash: (projectId: string) => queryOptions({ queryKey: queryKeys.projectTrash(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.listProjectTrash(projectId)) }),
  projectArchive: (projectId: string) => queryOptions({ queryKey: queryKeys.projectArchive(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.listProjectArchive(projectId)) }),
  sprintArchiveSchedule: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintArchiveSchedule(sprintId), queryFn: ({ signal }) => withSignal(signal, () => api.getSprintArchiveSchedule(sprintId)) }),
  projectSchedule: (projectId: string) => queryOptions({ queryKey: queryKeys.projectSchedule(projectId), queryFn: ({ signal }) => loadSchedule(projectId, signal) }),
  projectCalendar: (projectId: string) => queryOptions({ queryKey: queryKeys.projectCalendar(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectCalendar(projectId)) }),
  projectVelocity: (projectId: string) => queryOptions({ queryKey: queryKeys.projectVelocity(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectVelocity(projectId)) }),
  projectStatusSummary: (projectId: string) =>
    queryOptions({ queryKey: queryKeys.projectStatusSummary(projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getProjectStatusSummary(projectId)) }),
  sprintReport: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintReport(sprintId), queryFn: ({ signal }) => withSignal(signal, () => api.getSprintReport(sprintId)) }),
  sprintResults: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintResults(sprintId), queryFn: ({ signal }) => withSignal(signal, () => api.getSprintResults(sprintId)) }),
  sprintPreview: (sprintId: string) => queryOptions({ queryKey: queryKeys.sprintPreview(sprintId), queryFn: ({ signal }) => withSignal(signal, () => api.getSprintClosePreview(sprintId)) }),
  ticket: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticket(ticketId), queryFn: ({ signal }) => withSignal(signal, () => api.getTicket(ticketId)) }),
  ticketChildren: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketChildren(ticketId), queryFn: ({ signal }) => withSignal(signal, () => api.getTicketChildren(ticketId)) }),
  ticketComments: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketComments(ticketId), queryFn: ({ signal }) => withSignal(signal, () => api.getTicketComments(ticketId)) }),
  ticketAttachments: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketAttachments(ticketId), queryFn: ({ signal }) => withSignal(signal, () => api.getTicketAttachments(ticketId)) }),
  ticketDependencies: (ticketId: string) => queryOptions({ queryKey: queryKeys.ticketDependencies(ticketId), queryFn: ({ signal }) => withSignal(signal, () => api.getTicketDependencies(ticketId)) }),
  ticketRevision: (ticketId: string, revisionNo: number) =>
    queryOptions({ queryKey: queryKeys.ticketRevision(ticketId, revisionNo), queryFn: ({ signal }) => withSignal(signal, () => api.getTicketRevision(ticketId, revisionNo)) }),

  // Docs
  docTree: (workspaceId: string, projectId?: string | null) =>
    queryOptions({ queryKey: queryKeys.docTree(workspaceId, projectId), queryFn: ({ signal }) => withSignal(signal, () => api.getDocTree(workspaceId, projectId ?? undefined)) }),
  doc: (docId: string) => queryOptions({ queryKey: queryKeys.doc(docId), queryFn: ({ signal }) => withSignal(signal, () => api.getDoc(docId)) }),
  docVersion: (docId: string, versionNo: number) => queryOptions({ queryKey: queryKeys.docVersion(docId, versionNo), queryFn: ({ signal }) => withSignal(signal, () => api.getDocVersion(docId, versionNo)) }),
  docParticipants: (docId: string) => queryOptions({ queryKey: queryKeys.docParticipants(docId), queryFn: ({ signal }) => withSignal(signal, () => api.getDocParticipants(docId)) }),
  docComments: (docId: string) => queryOptions({ queryKey: queryKeys.docComments(docId), queryFn: ({ signal }) => withSignal(signal, () => api.getDocComments(docId)) }),
  docTemplates: (workspaceId: string) => queryOptions({ queryKey: queryKeys.docTemplates(workspaceId), queryFn: ({ signal }) => withSignal(signal, () => api.getDocTemplates(workspaceId)) }),
}
