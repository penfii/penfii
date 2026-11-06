import type {
  AuditPage,
  InvitationSummary,
  TeamMember,
  TeamRole,
  ProjectRole,
  TrashedTicket,
  WorkspaceMember,
  BulkChange,
  BulkResult,
  CalendarDefinition,
  EstimateInput,
  EstimateUnit,
  ProjectCalendar,
  ProjectCalendarView,
  ProjectSchedule,
  ProjectStatusSummary,
  SprintReport,
  VelocityReport,
  ScheduleInput,
  Channel,
  ChannelMember,
  TicketReference,
  DocAccessRole,
  DocComment,
  DocContent,
  DocNode,
  DocPage,
  DocParticipant,
  DocTemplate,
  DocVersion,
  DocVersionSummary,
  DocVisibility,
  DraftItem,
  DraftPayload,
  FavoriteFolder,
  FavoriteItem,
  InboxFilter,
  InboxPage,
  LaterKind,
  NotificationPreferences,
  ProjectPermissions,
  ReminderInfo,
  SearchResponse,
  SearchTab,
  Subscription,
  WatchTargetKind,
  StoredDraft,
  LaterItem,
  LaterState,
  ProjectPins,
  SavedKind,
  ScopeDataset,
  ScopeLens,
  SprintClosePreview,
  SprintResults,
  ViewLayout,
  Milestone,
  MovePlacement,
  ProjectPlanning,
  ProjectView,
  Sprint,
  TicketDependencies,
  ViewDefinition,
  ProjectBoard,
  ActivityPage,
  PrefixCheck,
  HierarchyKind,
  ProjectMember,
  RevisionDetail,
  RevisionPage,
  Ticket,
  TicketLabel,
  TicketAttachment,
  TicketComment,
  TicketType,
  WorkflowState,
  Conversation,
  ConversationChange,
  Dm,
  DirectoryHit,
  DirectoryMessageHit,
  DirectoryOrganization,
  DirectoryPerson,
  DirectorySearch,
  EditMessageRequest,
  FileObject,
  InvitationCreated,
  ManualUnreadRequest,
  Me,
  Message,
  WorkspacePerson,
  MessageHistory,
  OrganizationDirectory,
  Project,
  ReactionRequest,
  ResolvedReference,
  ReferenceKind,
  ReadRequest,
  ReadResult,
  SendMessageRequest,
  ServerInfo,
  Space,
  Team,
  TeamProject,
  InvitationTeamOutcome,
  Department,
  ThreadPage,
  Workspace,
} from '@penfii/contracts'

export type InvitationPreview = { email: string; workspaceName: string; role: string; accountExists: boolean }
export type AuthProvider = { key: string; displayName: string; kind: string }
export type LinkedIdentity = {
  provider: string
  displayName: string
  email: string | null
  linkedAt: string
  lastLoginAt: string | null
}
export type IdentitiesView = { items: LinkedIdentity[]; hasPassword: boolean; providers: AuthProvider[] }
export type SessionInfo = {
  id: string
  current: boolean
  authMethod: string | null
  deviceLabel: string | null
  createdAt: string
  lastSeenAt: string | null
  lastReauthAt: string | null
  expiresAt: string
}
export type SessionsView = { items: SessionInfo[]; reauthMethods: string[]; reauthWindowSeconds: number }
export type ConsentView = {
  id: string
  clientName: string
  clientId: string
  redirectHost: string
  loopback: boolean
  scopes: { scope: string; highRisk: boolean; change: boolean }[]
  replacesExisting: boolean
  expiresAt: string
}
export type OAuthGrant = { id: string; clientName: string; clientId: string; scopes: string[]; createdAt: string; lastUsedAt: string | null }
export type McpPolicyInput = { mcpEnabled: boolean; allowWrite: boolean; allowDestructive: boolean; allowAdministrative: boolean }
export type McpPolicy = McpPolicyInput & { workspaceId: string; isDefault: boolean; updatedAt: string | null }

export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly details: unknown

  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.details = details
  }
}

export function newIdempotencyKey(): string {
  return crypto.randomUUID()
}

export async function api<T>(path: string, options: RequestOptions = {}): Promise<T> {
  return request(path, options) as Promise<T>
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError
}

export function isNotFound(error: unknown): boolean {
  return isApiError(error) && error.status === 404 && error.code === 'not_found'
}

export function isUnauthorized(error: unknown): boolean {
  return isApiError(error) && error.status === 401
}

export function isVersionConflict(error: unknown): boolean {
  return isApiError(error) && error.code === 'version_conflict'
}

type UnauthorizedHandler = () => void

let unauthorizedHandler: UnauthorizedHandler | null = null

export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler
}

type WriteBlocker = () => boolean

let writeBlocker: WriteBlocker | null = null
let blockedListener: (() => void) | null = null

/**
 * While the blocker answers true no request that changes data leaves the browser: it fails at once with
 * `ApiError(0, 'offline')`, and `onBlocked` is told so the app can say why. One place stops every write, whichever screen or mutation asks for it.
 */
export function setWriteBlocker(blocker: WriteBlocker | null, onBlocked?: () => void): void {
  writeBlocker = blocker
  blockedListener = onBlocked ?? null
}

/** For a write that does not go through `request` (a file upload): throws what `request` would throw. */
export function assertWritable(): void {
  if (!writeBlocker?.()) return
  blockedListener?.()
  throw new ApiError(0, 'offline', 'You are offline')
}

function subscriptionPath(kind: WatchTargetKind, targetId: string): string {
  const base = kind === 'TICKET' ? 'tickets' : kind === 'PROJECT' ? 'projects' : 'messages'
  return `/v1/${base}/${targetId}/subscription`
}

type Query = Record<string, string | number | undefined>

type RequestOptions = {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  body?: unknown
  /** Sent as the body instead of JSON. Used for the profile photo. */
  rawBody?: Blob
  contentType?: string
  idempotencyKey?: string
  query?: Query
  /** A 401 here answers a password the caller sent; it does not say the session ended. */
  answersUnauthorized?: boolean
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function withQuery(path: string, query: Query | undefined): string {
  if (!query) return path
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined) params.set(key, String(value))
  }
  const text = params.toString()
  return text ? `${path}?${text}` : path
}

function pageQuery(limit: number | undefined, before: string | number | null | undefined): Query {
  return { limit, before: before ?? undefined }
}

function extraDetails(data: Record<string, unknown>): unknown {
  const extra: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(data)) {
    if (key === 'code' || key === 'message' || key === 'details') continue
    extra[key] = value
  }
  return Object.keys(extra).length > 0 ? extra : undefined
}

function errorBody(text: string, status: number): { code: string; message: string; details?: unknown } {
  try {
    const data = JSON.parse(text) as unknown
    if (isRecord(data)) {
      return {
        code: typeof data.code === 'string' ? data.code : 'error',
        message: typeof data.message === 'string' ? data.message : `HTTP ${String(status)}`,
        details: data.details ?? extraDetails(data),
      }
    }
  } catch {
    // Non-JSON error bodies stay generic so the UI never treats markup as a message.
  }
  return { code: 'error', message: `HTTP ${String(status)}` }
}

async function request(path: string, options: RequestOptions = {}): Promise<unknown> {
  const method = options.method ?? 'GET'
  if (method !== 'GET') assertWritable()
  const headers = new Headers()
  headers.set('Accept', 'application/json')
  if (options.rawBody) headers.set('Content-Type', options.contentType ?? 'application/octet-stream')
  else if (options.body !== undefined) headers.set('Content-Type', 'application/json')
  if (method !== 'GET') headers.set('X-Penfii-Client', '1')
  if (options.idempotencyKey) headers.set('Idempotency-Key', options.idempotencyKey)

  const response = await fetch(withQuery(path, options.query), {
    method,
    credentials: 'include',
    headers,
    body: options.rawBody ?? (options.body !== undefined ? JSON.stringify(options.body) : undefined),
  })

  // A wrong password on these is an answer to the question asked, not an expired session.
  if (
    response.status === 401 &&
    !options.answersUnauthorized &&
    path !== '/v1/session/login' &&
    path !== '/v1/me/reauth/password'
  ) {
    unauthorizedHandler?.()
  }

  const text = await response.text()
  if (!response.ok) {
    const parsed = errorBody(text, response.status)
    throw new ApiError(response.status, parsed.code, parsed.message, parsed.details)
  }
  if (response.status === 204 || text.trim() === '') return undefined
  return JSON.parse(text) as unknown
}

function limitNumber(value: unknown): number | null {
  if (value === null || value === undefined) return null
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return null
  return value
}

function normalizeServerInfo(payload: unknown): ServerInfo {
  if (!isRecord(payload)) throw new ApiError(500, 'invalid_response', 'Expected server info')
  const limits = isRecord(payload.limits) ? payload.limits : {}
  return {
    apiVersion: typeof payload.apiVersion === 'string' ? payload.apiVersion : '',
    features: payload.features,
    limits: {
      messageBodyMaxBytes: limitNumber(limits.messageBodyMaxBytes),
      fileMaxBytes: limitNumber(limits.fileMaxBytes),
      sessionTtlSeconds: limitNumber(limits.sessionTtlSeconds),
    },
  }
}

function normalizeMe(payload: unknown): Me {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a profile')
  }
  const email = typeof payload.email === 'string' ? payload.email : ''
  const displayName =
    typeof payload.displayName === 'string' && payload.displayName.length > 0
      ? payload.displayName
      : email || payload.id
  return {
    id: payload.id,
    email,
    displayName,
    locale: typeof payload.locale === 'string' ? payload.locale : null,
    timeZone: typeof payload.timeZone === 'string' ? payload.timeZone : null,
    jobTitle: typeof payload.jobTitle === 'string' ? payload.jobTitle : null,
    statusEmoji: typeof payload.statusEmoji === 'string' ? payload.statusEmoji : null,
    statusText: typeof payload.statusText === 'string' ? payload.statusText : null,
    statusClearAt: typeof payload.statusClearAt === 'string' ? payload.statusClearAt : null,
    hasAvatar: payload.hasAvatar === true,
  }
}

function normalizeWorkspacePerson(payload: unknown): WorkspacePerson {
  if (!isRecord(payload) || typeof payload.userId !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a person')
  }
  const teams = Array.isArray(payload.teams)
    ? payload.teams.flatMap((item) => {
        if (!isRecord(item) || typeof item.id !== 'string' || typeof item.name !== 'string') return []
        return [{ id: item.id, name: item.name }]
      })
    : []
  return {
    userId: payload.userId,
    displayName: typeof payload.displayName === 'string' ? payload.displayName : payload.userId,
    email: typeof payload.email === 'string' ? payload.email : '',
    role: typeof payload.role === 'string' ? payload.role : 'MEMBER',
    state: typeof payload.state === 'string' ? payload.state : 'ACTIVE',
    jobTitle: typeof payload.jobTitle === 'string' ? payload.jobTitle : null,
    timeZone: typeof payload.timeZone === 'string' ? payload.timeZone : null,
    statusEmoji: typeof payload.statusEmoji === 'string' ? payload.statusEmoji : null,
    statusText: typeof payload.statusText === 'string' ? payload.statusText : null,
    statusClearAt: typeof payload.statusClearAt === 'string' ? payload.statusClearAt : null,
    hasAvatar: payload.hasAvatar === true,
    teams,
  }
}

function normalizeWorkspace(payload: unknown): Workspace {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a workspace')
  }
  return {
    id: payload.id,
    name: typeof payload.name === 'string' ? payload.name : payload.id,
    timeZone: typeof payload.timeZone === 'string' ? payload.timeZone : 'UTC',
    ...(typeof payload.role === 'string' ? { role: payload.role } : {}),
  }
}

function itemsOf<T>(payload: unknown): T[] {
  if (!isRecord(payload) || !Array.isArray(payload.items)) throw new ApiError(500, 'invalid_response', 'Expected a list')
  return payload.items as T[]
}

function normalizeList<T>(payload: unknown, keys: string[], map: (value: unknown) => T): T[] {
  let source: unknown = payload
  if (isRecord(payload)) {
    for (const key of keys) {
      if (Array.isArray(payload[key])) {
        source = payload[key]
        break
      }
    }
  }
  if (!Array.isArray(source)) throw new ApiError(500, 'invalid_response', 'Expected a list')
  return source.map((item) => map(item))
}

function normalizeChannel(payload: unknown): Channel {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a channel')
  }
  const visibility = payload.visibility === 'PRIVATE' ? 'PRIVATE' : 'PUBLIC'
  const channel: Channel = {
    id: payload.id,
    name: typeof payload.name === 'string' ? payload.name : payload.id,
    visibility,
    teamId: typeof payload.teamId === 'string' ? payload.teamId : null,
    projectId: typeof payload.projectId === 'string' ? payload.projectId : null,
    conversationId: typeof payload.conversationId === 'string' ? payload.conversationId : payload.id,
    unreadCount: typeof payload.unreadCount === 'number' ? payload.unreadCount : 0,
    description: typeof payload.description === 'string' ? payload.description : null,
    role: typeof payload.role === 'string' ? payload.role : null,
  }
  if (typeof payload.joined === 'boolean') channel.joined = payload.joined
  else if ('role' in payload) channel.joined = typeof payload.role === 'string'
  return channel
}

function normalizeTeam(payload: unknown): Team {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a team')
  }
  return {
    id: payload.id,
    name: typeof payload.name === 'string' ? payload.name : payload.id,
    description: typeof payload.description === 'string' ? payload.description : null,
    archivedAt: typeof payload.archivedAt === 'string' ? payload.archivedAt : null,
    departmentId: typeof payload.departmentId === 'string' ? payload.departmentId : null,
  }
}

function normalizeDepartment(payload: unknown): Department {
  if (!isRecord(payload) || typeof payload.id !== 'string' || typeof payload.name !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a department')
  }
  return {
    id: payload.id,
    workspaceId: typeof payload.workspaceId === 'string' ? payload.workspaceId : '',
    parentId: typeof payload.parentId === 'string' ? payload.parentId : null,
    name: payload.name,
    position: typeof payload.position === 'number' ? payload.position : 0,
  }
}

function normalizeProject(payload: unknown): Project {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a project')
  }
  const teamIds = Array.isArray(payload.teamIds)
    ? payload.teamIds.filter((id): id is string => typeof id === 'string')
    : []
  return {
    id: payload.id,
    name: typeof payload.name === 'string' ? payload.name : payload.id,
    ticketPrefix: typeof payload.ticketPrefix === 'string' ? payload.ticketPrefix : undefined,
    teamIds,
    // Older servers do not say: the Project then opens as it always did.
    participating: typeof payload.participating === 'boolean' ? payload.participating : undefined,
    canManage: typeof payload.canManage === 'boolean' ? payload.canManage : undefined,
  }
}

function normalizeDm(payload: unknown): Dm {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a direct message')
  }
  const listed = Array.isArray(payload.memberIds)
    ? payload.memberIds
    : Array.isArray(payload.userIds)
      ? payload.userIds
      : []
  const memberIds = listed.filter((id): id is string => typeof id === 'string')
  const peers = Array.isArray(payload.peers)
    ? payload.peers.flatMap((item) => {
        if (!isRecord(item) || typeof item.id !== 'string') return []
        return [{
          id: item.id,
          displayName: typeof item.displayName === 'string' ? item.displayName : item.id,
          username: typeof item.username === 'string' ? item.username : null,
        }]
      })
    : []
  return {
    id: payload.id,
    memberIds,
    conversationId: typeof payload.conversationId === 'string' ? payload.conversationId : payload.id,
    peers,
    lastMessagePreview: typeof payload.lastMessagePreview === 'string' ? payload.lastMessagePreview : null,
    lastMessageAt: typeof payload.lastMessageAt === 'string' ? payload.lastMessageAt : null,
    lastMessageSeq: typeof payload.lastMessageSeq === 'number' ? payload.lastMessageSeq : null,
    unreadCount: typeof payload.unreadCount === 'number' ? payload.unreadCount : 0,
  }
}

function normalizeSpace(payload: unknown): Space {
  if (!isRecord(payload)) throw new ApiError(500, 'invalid_response', 'Expected a space')
  return {
    teams: Array.isArray(payload.teams) ? payload.teams.map((item) => normalizeTeam(item)) : [],
    projects: Array.isArray(payload.projects) ? payload.projects.map((item) => normalizeProject(item)) : [],
    channels: Array.isArray(payload.channels) ? payload.channels.map((item) => normalizeChannel(item)) : [],
    dms: Array.isArray(payload.dms) ? payload.dms.map((item) => normalizeDm(item)) : [],
  }
}

function normalizeMessage(payload: unknown): Message {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a message')
  }
  const reactions = Array.isArray(payload.reactions)
    ? payload.reactions.flatMap((item) => {
        if (!isRecord(item) || typeof item.emoji !== 'string') return []
        const userIds = Array.isArray(item.userIds)
          ? item.userIds.filter((id): id is string => typeof id === 'string')
          : []
        return [{
          emoji: item.emoji,
          count: typeof item.count === 'number' ? item.count : userIds.length,
          reactedByMe: item.reactedByMe === true,
          userIds,
        }]
      })
    : []
  return {
    id: payload.id,
    streamSeq: typeof payload.streamSeq === 'number' ? payload.streamSeq : 0,
    authorId: typeof payload.authorId === 'string' ? payload.authorId : '',
    authorDisplayName: typeof payload.authorDisplayName === 'string' ? payload.authorDisplayName : '',
    version: typeof payload.version === 'number' ? payload.version : 0,
    deleted: payload.deleted === true,
    body: payload.deleted === true ? null : ((payload.body as Message['body']) ?? null),
    createdAt: typeof payload.createdAt === 'string' ? payload.createdAt : '',
    editedAt: typeof payload.editedAt === 'string' ? payload.editedAt : null,
    replyCount: typeof payload.replyCount === 'number' ? payload.replyCount : 0,
    replyToId: typeof payload.replyToId === 'string' ? payload.replyToId : null,
    threadRootId: typeof payload.threadRootId === 'string' ? payload.threadRootId : null,
    reactions,
    attachments: normalizeAttachments(payload.attachments),
  }
}

function normalizeFile(payload: unknown): FileObject {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a file')
  }
  return {
    id: payload.id,
    generation: typeof payload.generation === 'number' ? payload.generation : 0,
    filename: typeof payload.filename === 'string' ? payload.filename : '',
    byteSize: typeof payload.byteSize === 'number' ? payload.byteSize : 0,
    mime: typeof payload.mime === 'string' ? payload.mime : null,
    state: typeof payload.state === 'string' ? payload.state : '',
  }
}

function normalizeAttachments(value: unknown): Message['attachments'] {
  if (!Array.isArray(value)) return []
  return value.flatMap((item) => {
    if (!isRecord(item) || typeof item.id !== 'string' || typeof item.fileId !== 'string') return []
    return [{
      id: item.id,
      fileId: item.fileId,
      filename: typeof item.filename === 'string' ? item.filename : '',
      byteSize: typeof item.byteSize === 'number' ? item.byteSize : 0,
      mime: typeof item.mime === 'string' ? item.mime : null,
      state: typeof item.state === 'string' ? item.state : '',
    }]
  })
}

function normalizeMessages(payload: unknown): Message[] {
  return normalizeList(payload, ['messages', 'items'], normalizeMessage)
}

function normalizeThread(payload: unknown): ThreadPage {
  if (Array.isArray(payload)) return { generation: null, messages: payload.map((item) => normalizeMessage(item)) }
  if (!isRecord(payload)) throw new ApiError(500, 'invalid_response', 'Expected a thread')
  const messages = Array.isArray(payload.messages) ? payload.messages.map((item) => normalizeMessage(item)) : []
  return {
    generation: typeof payload.generation === 'number' ? payload.generation : null,
    messages,
  }
}

function normalizeConversation(payload: unknown): Conversation {
  if (!isRecord(payload) || typeof payload.id !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a conversation')
  }
  const read = isRecord(payload.read) ? payload.read : payload
  if (typeof payload.lastChangeSeq !== 'number') {
    throw new ApiError(500, 'invalid_response', 'Expected a conversation cursor')
  }
  return {
    id: payload.id,
    generation: typeof read.generation === 'number' ? read.generation : null,
    readThroughSeq: typeof read.readThroughSeq === 'number' ? read.readThroughSeq : null,
    manualUnreadFromSeq: typeof read.manualUnreadFromSeq === 'number' ? read.manualUnreadFromSeq : null,
    title: typeof payload.title === 'string' ? payload.title : null,
    kind: typeof payload.kind === 'string' ? payload.kind : null,
    lastChangeSeq: payload.lastChangeSeq,
  }
}

function normalizeHistory(payload: unknown): MessageHistory {
  const versions = normalizeList(payload, ['versions'], (item) => {
    if (!isRecord(item) || typeof item.version !== 'number') {
      throw new ApiError(500, 'invalid_response', 'Expected a message version')
    }
    return {
      version: item.version,
      body: (item.body as Message['body']) ?? null,
      editedAt: typeof item.editedAt === 'string' ? item.editedAt : null,
    }
  })
  return { versions }
}

function normalizeRead(payload: unknown, sentSeq: number): ReadResult {
  if (!isRecord(payload) || typeof payload.generation !== 'number') {
    throw new ApiError(500, 'invalid_response', 'Expected a read result')
  }
  return {
    generation: payload.generation,
    readThroughSeq: typeof payload.readThroughSeq === 'number' ? payload.readThroughSeq : sentSeq,
    manualUnreadFromSeq: typeof payload.manualUnreadFromSeq === 'number' ? payload.manualUnreadFromSeq : null,
  }
}

function normalizeChanges(payload: unknown): ConversationChange[] {
  return normalizeList(payload, ['changes', 'items'], (item) => {
    if (!isRecord(item) || typeof item.changeSeq !== 'number' || typeof item.messageId !== 'string') {
      throw new ApiError(500, 'invalid_response', 'Expected a change')
    }
    return {
      conversationId: typeof item.conversationId === 'string' ? item.conversationId : '',
      changeSeq: item.changeSeq,
      messageId: item.messageId,
      messageVersion: typeof item.messageVersion === 'number' ? item.messageVersion : 0,
      changeKind: typeof item.changeKind === 'string' ? item.changeKind : '',
    }
  })
}

function normalizeInvitation(payload: unknown): InvitationCreated {
  if (!isRecord(payload)) return {}
  return typeof payload.inviteToken === 'string' ? { inviteToken: payload.inviteToken } : {}
}

function normalizePerson(payload: unknown): DirectoryPerson {
  if (
    !isRecord(payload) ||
    typeof payload.id !== 'string' ||
    typeof payload.displayName !== 'string' ||
    typeof payload.email !== 'string'
  ) {
    throw new ApiError(500, 'invalid_response', 'Expected a person')
  }
  return {
    id: payload.id,
    displayName: payload.displayName,
    username: typeof payload.username === 'string' ? payload.username : null,
    email: payload.email,
    organizationId: typeof payload.organizationId === 'string' ? payload.organizationId : '',
    organizationName: typeof payload.organizationName === 'string' ? payload.organizationName : '',
    affiliation: Array.isArray(payload.affiliation)
      ? payload.affiliation.filter((item): item is string => typeof item === 'string')
      : [],
  }
}

function normalizeOrganization(payload: unknown): DirectoryOrganization {
  if (!isRecord(payload) || typeof payload.id !== 'string' || typeof payload.name !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected an organization')
  }
  return {
    id: payload.id,
    name: payload.name,
    timeZone: typeof payload.timeZone === 'string' ? payload.timeZone : '',
    memberCount: typeof payload.memberCount === 'number' ? payload.memberCount : 0,
  }
}

function normalizeMessageHit(payload: unknown): DirectoryMessageHit {
  if (!isRecord(payload) || typeof payload.id !== 'string' || typeof payload.conversationId !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a message')
  }
  return {
    id: payload.id,
    conversationId: payload.conversationId,
    authorId: typeof payload.authorId === 'string' ? payload.authorId : '',
    authorName: typeof payload.authorName === 'string' ? payload.authorName : '',
    snippet: typeof payload.snippet === 'string' ? payload.snippet : '',
    createdAt: typeof payload.createdAt === 'string' ? payload.createdAt : '',
  }
}

function normalizeHit(payload: unknown): DirectoryHit {
  if (!isRecord(payload) || typeof payload.id !== 'string' || typeof payload.title !== 'string') {
    throw new ApiError(500, 'invalid_response', 'Expected a search hit')
  }
  const type = payload.type
  if (type !== 'person' && type !== 'organization' && type !== 'message') {
    throw new ApiError(500, 'invalid_response', 'Expected a search hit')
  }
  return {
    type,
    id: payload.id,
    title: payload.title,
    subtitle: typeof payload.subtitle === 'string' ? payload.subtitle : '',
    person: payload.person ? normalizePerson(payload.person) : undefined,
    organization: payload.organization ? normalizeOrganization(payload.organization) : undefined,
    message: payload.message ? normalizeMessageHit(payload.message) : undefined,
  }
}

function normalizeDirectorySearch(payload: unknown): DirectorySearch {
  if (!isRecord(payload) || !Array.isArray(payload.results)) {
    throw new ApiError(500, 'invalid_response', 'Expected search results')
  }
  return {
    query: typeof payload.query === 'string' ? payload.query : '',
    results: payload.results.map((item) => normalizeHit(item)),
  }
}

function normalizeOrganizationDirectory(payload: unknown): OrganizationDirectory {
  if (!isRecord(payload) || !Array.isArray(payload.members)) {
    throw new ApiError(500, 'invalid_response', 'Expected an organization')
  }
  return {
    organization: normalizeOrganization(payload.organization),
    members: payload.members.map((item) => normalizePerson(item)),
  }
}

export function createPenfiiClient() {
  return {
    serverInfo(): Promise<ServerInfo> {
      return request('/v1/server-info').then(normalizeServerInfo)
    },
    login(body: { email: string; password: string }): Promise<void> {
      return request('/v1/session/login', { method: 'POST', body }).then(() => undefined)
    },
    logout(): Promise<void> {
      return request('/v1/session/logout', { method: 'POST', body: {} }).then(() => undefined)
    },
    me(): Promise<Me> {
      return request('/v1/me').then(normalizeMe)
    },
    updateMe(body: {
      displayName: string
      email: string
      currentPassword?: string
      newPassword?: string | null
    }): Promise<Me> {
      // With a current password, a 401 means that password was wrong, not that the session ended.
      return request('/v1/me', { method: 'PATCH', body, answersUnauthorized: body.currentPassword !== undefined }).then(normalizeMe)
    },
    updateProfile(body: {
      displayName: string
      jobTitle?: string | null
      timeZone?: string | null
      statusEmoji?: string | null
      statusText?: string | null
      statusClearAt?: string | null
    }): Promise<Me> {
      return request('/v1/me/profile', { method: 'PATCH', body }).then(normalizeMe)
    },
    uploadAvatar(file: Blob): Promise<void> {
      return request('/v1/me/avatar', {
        method: 'PUT',
        rawBody: file,
        contentType: file.type || 'image/jpeg',
      }).then(() => undefined)
    },
    deleteAvatar(): Promise<void> {
      return request('/v1/me/avatar', { method: 'DELETE' }).then(() => undefined)
    },
    listPeople(workspaceId: string): Promise<WorkspacePerson[]> {
      return request(`/v1/workspaces/${workspaceId}/people`).then((payload) => {
        if (!isRecord(payload) || !Array.isArray(payload.items)) {
          throw new ApiError(500, 'invalid_response', 'Expected people')
        }
        return payload.items.map(normalizeWorkspacePerson)
      })
    },
    listWorkspaces(): Promise<Workspace[]> {
      return request('/v1/workspaces').then((payload) => normalizeList(payload, ['workspaces', 'items'], normalizeWorkspace))
    },
    // Whether this person may create a workspace now: only while the instance has none (bootstrap).
    workspaceCreationOpen(): Promise<boolean> {
      return request('/v1/workspaces/creation').then((payload) => isRecord(payload) && payload.open === true)
    },
    createWorkspace(body: { name: string; timeZone: string }): Promise<Workspace> {
      return request('/v1/workspaces', { method: 'POST', body }).then(normalizeWorkspace)
    },
    createInvitation(
      workspaceId: string,
      body: { email: string; role: string; teamId?: string | null },
    ): Promise<InvitationCreated> {
      return request(`/v1/workspaces/${workspaceId}/invitations`, { method: 'POST', body }).then(normalizeInvitation)
    },
    listMembers(workspaceId: string): Promise<WorkspaceMember[]> {
      return request(`/v1/workspaces/${workspaceId}/members`).then((payload) => itemsOf<WorkspaceMember>(payload))
    },
    changeMemberRole(workspaceId: string, userId: string, role: 'OWNER' | 'ADMIN' | 'MEMBER'): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/members/${userId}`, { method: 'PATCH', body: { role } }).then(() => undefined)
    },
    removeMember(workspaceId: string, userId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/members/${userId}`, { method: 'DELETE' }).then(() => undefined)
    },
    setAccountDisabled(workspaceId: string, userId: string, disabled: boolean): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/members/${userId}/account`, { method: 'PUT', body: { disabled } }).then(() => undefined)
    },
    listInvitations(workspaceId: string): Promise<InvitationSummary[]> {
      return request(`/v1/workspaces/${workspaceId}/invitations`).then((payload) => itemsOf<InvitationSummary>(payload))
    },
    revokeInvitation(workspaceId: string, invitationId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/invitations/${invitationId}`, { method: 'DELETE' }).then(() => undefined)
    },
    listTeamMembers(workspaceId: string, teamId: string): Promise<TeamMember[]> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/members`).then((payload) => itemsOf<TeamMember>(payload))
    },
    /** Puts a person in the Team with a role, or changes the role of someone already in it. */
    addTeamMember(workspaceId: string, teamId: string, userId: string, role: TeamRole): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/members/${userId}`, { method: 'PUT', body: { role } }).then(() => undefined)
    },
    removeTeamMember(workspaceId: string, teamId: string, userId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/members/${userId}`, { method: 'DELETE' }).then(() => undefined)
    },
    listAudit(workspaceId: string, query: { action?: string; cursor?: string; limit?: number } = {}): Promise<AuditPage> {
      return request(`/v1/workspaces/${workspaceId}/audit`, { query }).then((payload) => {
        if (!isRecord(payload) || !Array.isArray(payload.items)) throw new ApiError(500, 'invalid_response', 'Expected an audit page')
        return { items: payload.items, nextCursor: typeof payload.nextCursor === 'string' ? payload.nextCursor : null } as AuditPage
      })
    },
    listProjectTrash(projectId: string): Promise<TrashedTicket[]> {
      return request(`/v1/projects/${projectId}/trash`).then((payload) => itemsOf<TrashedTicket>(payload))
    },
    trashTicket(ticketId: string): Promise<void> {
      return request(`/v1/tickets/${ticketId}/commands/trash`, { method: 'POST', body: {} }).then(() => undefined)
    },
    restoreTicketFromTrash(ticketId: string): Promise<void> {
      return request(`/v1/tickets/${ticketId}/commands/restore-from-trash`, { method: 'POST', body: {} }).then(() => undefined)
    },
    purgeTicket(ticketId: string): Promise<void> {
      return request(`/v1/tickets/${ticketId}/commands/purge`, { method: 'POST', body: {} }).then(() => undefined)
    },
    authProviders(): Promise<AuthProvider[]> {
      return request('/v1/auth/providers').then((payload) => (isRecord(payload) && Array.isArray(payload.items) ? (payload.items as AuthProvider[]) : []))
    },
    authStart(
      provider: string,
      body: { intent: 'login' | 'invite' | 'link'; invitationToken?: string; returnTo?: string },
    ): Promise<string> {
      return request(`/v1/auth/${encodeURIComponent(provider)}/start`, { method: 'POST', body }).then((payload) => {
        if (!isRecord(payload) || typeof payload.url !== 'string') throw new ApiError(500, 'invalid_response', 'Expected a provider URL')
        return payload.url
      })
    },
    reauthPassword(password: string): Promise<void> {
      return request('/v1/me/reauth/password', { method: 'POST', body: { password } }).then(() => undefined)
    },
    reauthProviderStart(provider: string, returnTo: string): Promise<string> {
      return request(`/v1/me/reauth/${encodeURIComponent(provider)}/start`, { method: 'POST', body: { returnTo } }).then((payload) => {
        if (!isRecord(payload) || typeof payload.url !== 'string') throw new ApiError(500, 'invalid_response', 'Expected a provider URL')
        return payload.url
      })
    },
    identities(): Promise<IdentitiesView> {
      return request('/v1/me/identities') as Promise<IdentitiesView>
    },
    unlinkIdentity(provider: string): Promise<void> {
      return request(`/v1/me/identities/${encodeURIComponent(provider)}`, { method: 'DELETE' }).then(() => undefined)
    },
    removePassword(): Promise<void> {
      return request('/v1/me/password', { method: 'DELETE' }).then(() => undefined)
    },
    sessions(): Promise<SessionsView> {
      return request('/v1/me/sessions') as Promise<SessionsView>
    },
    revokeSession(id: string): Promise<void> {
      return request(`/v1/me/sessions/${id}`, { method: 'DELETE' }).then(() => undefined)
    },
    revokeOtherSessions(): Promise<void> {
      return request('/v1/me/sessions', { method: 'DELETE' }).then(() => undefined)
    },
    consentRequest(id: string): Promise<ConsentView> {
      return request(`/v1/oauth/requests/${id}`) as Promise<ConsentView>
    },
    consentDecision(id: string, body: { approve: boolean; scopes?: string[] }): Promise<string> {
      return request(`/v1/oauth/requests/${id}/decision`, { method: 'POST', body }).then((payload) => {
        if (!isRecord(payload) || typeof payload.redirectTo !== 'string') throw new ApiError(500, 'invalid_response', 'Expected a redirect')
        return payload.redirectTo
      })
    },
    oauthGrants(): Promise<OAuthGrant[]> {
      return request('/v1/me/oauth-grants').then((payload) => (isRecord(payload) && Array.isArray(payload.items) ? (payload.items as OAuthGrant[]) : []))
    },
    revokeOAuthGrant(id: string): Promise<void> {
      return request(`/v1/me/oauth-grants/${id}`, { method: 'DELETE' }).then(() => undefined)
    },
    revokeAllOAuthGrants(): Promise<void> {
      return request('/v1/me/oauth-grants', { method: 'DELETE' }).then(() => undefined)
    },
    mcpPolicy(workspaceId: string): Promise<McpPolicy> {
      return request(`/v1/workspaces/${workspaceId}/mcp-policy`) as Promise<McpPolicy>
    },
    setMcpPolicy(workspaceId: string, body: McpPolicyInput): Promise<McpPolicy> {
      return request(`/v1/workspaces/${workspaceId}/mcp-policy`, { method: 'PUT', body }) as Promise<McpPolicy>
    },
    // What the link is for, before anyone signs in: the invited e-mail, the workspace, and whether that e-mail has an account.
    previewInvitation(token: string): Promise<InvitationPreview> {
      return request('/v1/invitations/preview', { method: 'POST', body: { token } }).then((payload) => {
        if (!isRecord(payload) || typeof payload.email !== 'string') throw new ApiError(500, 'invalid_response', 'Expected an invitation')
        return {
          email: payload.email,
          workspaceName: typeof payload.workspaceName === 'string' ? payload.workspaceName : '',
          role: typeof payload.role === 'string' ? payload.role : 'MEMBER',
          accountExists: payload.accountExists === true,
        }
      })
    },
    acceptInvitation(body: { token: string; displayName: string; password?: string }): Promise<{ workspaceId: string | null; team: InvitationTeamOutcome | null }> {
      return request('/v1/invitations/accept', { method: 'POST', body }).then((payload) => {
        const team = isRecord(payload) && isRecord(payload.team) ? payload.team : null
        return {
          workspaceId: isRecord(payload) && typeof payload.workspaceId === 'string' ? payload.workspaceId : null,
          team:
            team && typeof team.teamName === 'string' && (team.status === 'JOINED' || team.status === 'ARCHIVED' || team.status === 'GONE')
              ? { teamId: typeof team.teamId === 'string' ? team.teamId : null, teamName: team.teamName, status: team.status }
              : null,
        }
      })
    },
    createTeam(workspaceId: string, body: { name: string; description?: string; members: Array<{ userId: string; role: TeamRole }> }): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams`, { method: 'POST', body }).then(normalizeTeam)
    },
    updateWorkspace(workspaceId: string, body: { name?: string; timeZone?: string }): Promise<Workspace> {
      return request(`/v1/workspaces/${workspaceId}`, { method: 'PATCH', body }) as Promise<Workspace>
    },
    renameTeam(workspaceId: string, teamId: string, name: string): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}`, { method: 'PATCH', body: { name } }).then(normalizeTeam)
    },
    /** Name and/or description; a description of `null` or empty clears it. */
    updateTeam(workspaceId: string, teamId: string, body: { name?: string; description?: string | null }): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}`, { method: 'PATCH', body }).then(normalizeTeam)
    },
    archiveTeam(workspaceId: string, teamId: string): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/archive`, { method: 'POST' }).then(normalizeTeam)
    },
    restoreTeam(workspaceId: string, teamId: string): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/restore`, { method: 'POST' }).then(normalizeTeam)
    },
    listTeamProjects(workspaceId: string, teamId: string): Promise<TeamProject[]> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/projects`).then((payload) => itemsOf<TeamProject>(payload))
    },
    listDepartments(workspaceId: string): Promise<Department[]> {
      return request(`/v1/workspaces/${workspaceId}/departments`).then((payload) => {
        if (!isRecord(payload) || !Array.isArray(payload.items)) {
          throw new ApiError(500, 'invalid_response', 'Expected departments')
        }
        return payload.items.map(normalizeDepartment)
      })
    },
    createDepartment(workspaceId: string, body: { name: string; parentId?: string | null }): Promise<Department> {
      return request(`/v1/workspaces/${workspaceId}/departments`, { method: 'POST', body }).then(normalizeDepartment)
    },
    updateDepartment(workspaceId: string, departmentId: string, body: { name: string; parentId: string | null }): Promise<Department> {
      return request(`/v1/workspaces/${workspaceId}/departments/${departmentId}`, { method: 'PATCH', body }).then(normalizeDepartment)
    },
    deleteDepartment(workspaceId: string, departmentId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/departments/${departmentId}`, { method: 'DELETE' }).then(() => undefined)
    },
    placeTeam(workspaceId: string, teamId: string, departmentId: string | null): Promise<Team> {
      return request(`/v1/workspaces/${workspaceId}/teams/${teamId}/department`, { method: 'PUT', body: { departmentId } }).then(normalizeTeam)
    },
    /** Gives a person a direct role in the Project, or changes it. */
    setProjectGrant(projectId: string, userId: string, role: ProjectRole): Promise<void> {
      return request(`/v1/projects/${projectId}/grants/${userId}`, { method: 'PUT', body: { role } }).then(() => undefined)
    },
    removeProjectGrant(projectId: string, userId: string): Promise<void> {
      return request(`/v1/projects/${projectId}/grants/${userId}`, { method: 'DELETE' }).then(() => undefined)
    },
    /** A link records that the Team takes part in the Project; it gives its members no access. */
    linkProjectTeam(projectId: string, teamId: string): Promise<void> {
      return request(`/v1/projects/${projectId}/teams/${teamId}`, { method: 'PUT' }).then(() => undefined)
    },
    unlinkProjectTeam(projectId: string, teamId: string): Promise<void> {
      return request(`/v1/projects/${projectId}/teams/${teamId}`, { method: 'DELETE' }).then(() => undefined)
    },
    /** Lets the members of a linked Team into the Project with `role`; `null` ends it and keeps the link. */
    setProjectTeamRole(projectId: string, teamId: string, role: ProjectRole | null): Promise<void> {
      return request(`/v1/projects/${projectId}/teams/${teamId}/participation`, { method: 'PUT', body: { role } }).then(() => undefined)
    },
    createProject(workspaceId: string, body: { name: string; teamIds: string[]; ticketPrefix?: string }): Promise<Project> {
      return request(`/v1/workspaces/${workspaceId}/projects`, { method: 'POST', body }).then(normalizeProject)
    },
    checkProjectPrefix(workspaceId: string, query: { name?: string; prefix?: string }): Promise<PrefixCheck> {
      return request(`/v1/workspaces/${workspaceId}/project-prefix`, { query }) as Promise<PrefixCheck>
    },
    getSpace(workspaceId: string): Promise<Space> {
      return request(`/v1/workspaces/${workspaceId}/space`).then(normalizeSpace)
    },
    getProjectBoard(projectId: string): Promise<ProjectBoard> {
      return request(`/v1/projects/${projectId}/board`) as Promise<ProjectBoard>
    },
    getTicket(ticketId: string): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}`) as Promise<Ticket>
    },
    getTicketRevisions(ticketId: string, page: { limit?: number; before?: number | null } = {}): Promise<RevisionPage> {
      return request(`/v1/tickets/${ticketId}/revisions`, { query: pageQuery(page.limit, page.before) }) as Promise<RevisionPage>
    },
    getTicketRevision(ticketId: string, revisionNo: number): Promise<RevisionDetail> {
      return request(`/v1/tickets/${ticketId}/revisions/${String(revisionNo)}`) as Promise<RevisionDetail>
    },
    getTicketActivity(ticketId: string, page: { limit?: number; before?: string | null } = {}): Promise<ActivityPage> {
      return request(`/v1/tickets/${ticketId}/activity`, { query: pageQuery(page.limit, page.before) }) as Promise<ActivityPage>
    },
    getTicketChildren(ticketId: string): Promise<Ticket[]> {
      return request(`/v1/tickets/${ticketId}/children`) as Promise<Ticket[]>
    },
    getProjectMembers(projectId: string): Promise<ProjectMember[]> {
      return request(`/v1/projects/${projectId}/members`) as Promise<ProjectMember[]>
    },
    /** The active members of a Team linked to the Project, by name, for a Project admin choosing people. */
    getProjectTeamMembers(projectId: string, teamId: string): Promise<ProjectMember[]> {
      return request(`/v1/projects/${projectId}/teams/${teamId}/members`) as Promise<ProjectMember[]>
    },
    getProjectTicketTypes(projectId: string): Promise<TicketType[]> {
      return request(`/v1/projects/${projectId}/ticket-types`) as Promise<TicketType[]>
    },
    restoreTicket(
      ticketId: string,
      body: { expectedRevision: number; sourceRevision: number; fields: string[] },
      idempotencyKey: string,
    ): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}/commands/restore`, { method: 'POST', body, idempotencyKey }) as Promise<Ticket>
    },
    getProjectLabels(projectId: string): Promise<TicketLabel[]> {
      return request(`/v1/projects/${projectId}/labels`) as Promise<TicketLabel[]>
    },
    createProjectLabel(projectId: string, body: { name: string; color?: string | null }): Promise<TicketLabel> {
      return request(`/v1/projects/${projectId}/labels`, { method: 'POST', body }) as Promise<TicketLabel>
    },
    getProjectWorkflow(projectId: string): Promise<{ projectId: string; version: number; states: WorkflowState[] }> {
      return request(`/v1/projects/${projectId}/workflow`) as Promise<{ projectId: string; version: number; states: WorkflowState[] }>
    },
    createTicket(projectId: string, body: {
      title: string
      description?: unknown
      hierarchyKind?: HierarchyKind
      parentId?: string | null
      channelId?: string | null
      statusId?: string | null
      assigneeId?: string | null
      priority?: string | null
      dueOn?: string | null
      labelIds?: string[]
      typeId?: string | null
      estimate?: EstimateInput | null
      schedule?: ScheduleInput | null
      fileIds?: string[]
    }, idempotencyKey: string): Promise<Ticket> {
      return request(`/v1/projects/${projectId}/tickets`, { method: 'POST', body, idempotencyKey }) as Promise<Ticket>
    },
    updateTicket(ticketId: string, body: {
      expectedRevision: number
      title?: string
      description?: unknown
      assigneeId?: string | null
      priority?: string | null
      dueOn?: string | null
      labelIds?: string[]
      parentId?: string | null
      typeId?: string | null
      /** `null` clears the estimate; omitted leaves it. */
      estimate?: EstimateInput | null
      /** `null` clears the schedule; a schedule replaces the whole schedule. */
      schedule?: ScheduleInput | null
    }, idempotencyKey: string): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}`, { method: 'PATCH', body, idempotencyKey }) as Promise<Ticket>
    },
    moveTicket(
      ticketId: string,
      body: MovePlacement & { expectedRevision?: number; statusId?: string },
      idempotencyKey: string,
    ): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}/commands/move`, { method: 'POST', body, idempotencyKey }) as Promise<Ticket>
    },
    updateWorkflowState(
      projectId: string,
      stateId: string,
      body: {
        expectedWorkflowVersion: number
        wipLimit?: number | null
        name?: string
        category?: WorkflowState['category']
        position?: number
        isInitial?: boolean
      },
    ): Promise<{ version: number; state: WorkflowState }> {
      return request(`/v1/projects/${projectId}/workflow/states/${stateId}`, { method: 'PATCH', body }) as Promise<{ version: number; state: WorkflowState }>
    },
    createWorkflowState(
      projectId: string,
      body: { expectedWorkflowVersion: number; name: string; category: WorkflowState['category']; position: number; isInitial?: boolean },
    ): Promise<{ version: number; state: WorkflowState }> {
      return request(`/v1/projects/${projectId}/workflow/states`, { method: 'POST', body }) as Promise<{ version: number; state: WorkflowState }>
    },
    deleteWorkflowState(projectId: string, stateId: string, expectedWorkflowVersion: number): Promise<{ version: number }> {
      return request(`/v1/projects/${projectId}/workflow/states/${stateId}?expectedWorkflowVersion=${String(expectedWorkflowVersion)}`, { method: 'DELETE' }) as Promise<{ version: number }>
    },
    getProjectViews(projectId: string): Promise<ProjectView[]> {
      return request(`/v1/projects/${projectId}/views`) as Promise<ProjectView[]>
    },
    createProjectView(
      projectId: string,
      body: { name: string; scope: 'PERSONAL' | 'PROJECT'; layout?: ViewLayout; definition: ViewDefinition },
      idempotencyKey: string,
    ): Promise<ProjectView> {
      return request(`/v1/projects/${projectId}/views`, { method: 'POST', body, idempotencyKey }) as Promise<ProjectView>
    },
    /** The caller's own Views for a Team or for the whole Workspace. */
    getWideViews(workspaceId: string, target: { kind: 'TEAM'; teamId: string } | { kind: 'WORKSPACE' }): Promise<ProjectView[]> {
      const query = target.kind === 'TEAM' ? { targetKind: 'TEAM', teamId: target.teamId } : { targetKind: 'WORKSPACE' }
      return request(`/v1/workspaces/${workspaceId}/views`, { query }) as Promise<ProjectView[]>
    },
    createWideView(
      workspaceId: string,
      body: { targetKind: 'TEAM' | 'WORKSPACE'; teamId?: string; name: string; layout: ViewLayout; definition: ViewDefinition },
      idempotencyKey: string,
    ): Promise<ProjectView> {
      return request(`/v1/workspaces/${workspaceId}/views`, { method: 'POST', body, idempotencyKey }) as Promise<ProjectView>
    },
    updateProjectView(viewId: string, body: { expectedVersion: number; name?: string; layout?: ViewLayout; definition?: ViewDefinition }): Promise<ProjectView> {
      return request(`/v1/views/${viewId}`, { method: 'PATCH', body }) as Promise<ProjectView>
    },
    deleteProjectView(viewId: string, expectedVersion: number): Promise<void> {
      return request(`/v1/views/${viewId}`, { method: 'DELETE', query: { expectedVersion } }).then(() => undefined)
    },
    getTicketDependencies(ticketId: string): Promise<TicketDependencies> {
      return request(`/v1/tickets/${ticketId}/dependencies`) as Promise<TicketDependencies>
    },
    addTicketDependency(ticketId: string, predecessorId: string, idempotencyKey: string, kind: 'BLOCKS' | 'RELATED' = 'BLOCKS'): Promise<TicketDependencies> {
      return request(`/v1/tickets/${ticketId}/dependencies`, { method: 'POST', body: { predecessorId, kind }, idempotencyKey }) as Promise<TicketDependencies>
    },
    waiveDependency(dependencyId: string, reason: string, idempotencyKey: string): Promise<TicketDependencies> {
      return request(`/v1/dependencies/${dependencyId}/commands/waive`, { method: 'POST', body: { reason }, idempotencyKey }) as Promise<TicketDependencies>
    },
    restoreDependency(dependencyId: string, reason: string, idempotencyKey: string): Promise<TicketDependencies> {
      return request(`/v1/dependencies/${dependencyId}/commands/restore`, { method: 'POST', body: { reason }, idempotencyKey }) as Promise<TicketDependencies>
    },
    removeDependency(dependencyId: string): Promise<void> {
      return request(`/v1/dependencies/${dependencyId}`, { method: 'DELETE' }).then(() => undefined)
    },
    /** Project admin: the estimate unit. NONE to a unit is always allowed; other changes need no ACTIVE Sprint. */
    setProjectEstimateUnit(projectId: string, estimateUnit: EstimateUnit): Promise<{ id: string; estimateUnit: EstimateUnit }> {
      return request(`/v1/projects/${projectId}`, { method: 'PATCH', body: { estimateUnit } }) as Promise<{ id: string; estimateUnit: EstimateUnit }>
    },
    getProjectCalendar(projectId: string): Promise<ProjectCalendarView> {
      return request(`/v1/projects/${projectId}/calendar`) as Promise<ProjectCalendarView>
    },
    /** `expectedVersion` 0 creates the calendar; otherwise it is the version that was read. */
    putProjectCalendar(projectId: string, body: { expectedVersion: number; timeZone: string; definition: CalendarDefinition }): Promise<ProjectCalendar> {
      return request(`/v1/projects/${projectId}/calendar`, { method: 'PUT', body }) as Promise<ProjectCalendar>
    },
    getProjectPlanning(projectId: string): Promise<ProjectPlanning> {
      return request(`/v1/projects/${projectId}/planning`) as Promise<ProjectPlanning>
    },
    createSprint(
      projectId: string,
      body: { name: string; goal?: string | null; startsOn: string; endsOn: string; timeZone?: string; estimateUnit?: EstimateUnit },
      idempotencyKey: string,
    ): Promise<Sprint> {
      return request(`/v1/projects/${projectId}/sprints`, { method: 'POST', body, idempotencyKey }) as Promise<Sprint>
    },
    sprintCommand(sprintId: string, command: 'start' | 'complete' | 'cancel', expectedVersion: number, idempotencyKey: string, carryToSprintId?: string | null): Promise<Sprint> {
      const body = command === 'complete' && carryToSprintId ? { expectedVersion, carryToSprintId } : { expectedVersion }
      return request(`/v1/sprints/${sprintId}/commands/${command}`, { method: 'POST', body, idempotencyKey }) as Promise<Sprint>
    },
    getSprintClosePreview(sprintId: string): Promise<SprintClosePreview> {
      return request(`/v1/sprints/${sprintId}/close-preview`) as Promise<SprintClosePreview>
    },
    /** Figures computed on request for the Tickets the caller can see; nothing is cached on the server. */
    getSprintReport(sprintId: string): Promise<SprintReport> {
      return request(`/v1/sprints/${sprintId}/report`) as Promise<SprintReport>
    },
    /** The Tickets of a Project with their schedules, for the Gantt; one page of up to 1000. */
    getProjectSchedule(projectId: string, query: { channelId?: string; cursor?: string; limit?: number } = {}): Promise<ProjectSchedule> {
      return request(`/v1/projects/${projectId}/schedule`, { query }) as Promise<ProjectSchedule>
    },
    getProjectVelocity(projectId: string, limit?: number): Promise<VelocityReport> {
      return request(`/v1/projects/${projectId}/velocity`, limit ? { query: { limit } } : {}) as Promise<VelocityReport>
    },
    getProjectStatusSummary(projectId: string): Promise<ProjectStatusSummary> {
      return request(`/v1/projects/${projectId}/status-summary`) as Promise<ProjectStatusSummary>
    },
    getSprintResults(sprintId: string): Promise<SprintResults> {
      return request(`/v1/sprints/${sprintId}/results`) as Promise<SprintResults>
    },
    /** `projectIds` narrows the Tickets on the server before the 1000-Ticket cap. */
    getScopeDataset(workspaceId: string, scope: { kind: 'TEAM'; teamId: string } | { kind: 'WORKSPACE' }, lens: ScopeLens = 'ALL', projectIds: string[] = []): Promise<ScopeDataset> {
      const base = scope.kind === 'TEAM' ? { scope: 'TEAM', teamId: scope.teamId, lens } : { scope: 'WORKSPACE', lens }
      const query = projectIds.length > 0 ? { ...base, projects: projectIds.join(',') } : base
      return request(`/v1/workspaces/${workspaceId}/work`, { query }) as Promise<ScopeDataset>
    },
    bulkTickets(workspaceId: string, ticketIds: string[], change: BulkChange, idempotencyKey: string): Promise<BulkResult> {
      return request(`/v1/workspaces/${workspaceId}/tickets/bulk`, { method: 'POST', body: { ticketIds, change }, idempotencyKey }) as Promise<BulkResult>
    },
    getDrafts(workspaceId: string): Promise<DraftItem[]> {
      return request(`/v1/workspaces/${workspaceId}/drafts`) as Promise<DraftItem[]>
    },
    getDraft(contextKind: string, contextId: string): Promise<StoredDraft | null> {
      return request('/v1/drafts', { query: { contextKind, contextId } }).then((body) => (body ?? null) as StoredDraft | null)
    },
    saveDraft(body: { contextKind: string; contextId: string; expectedVersion?: number; payload: DraftPayload }): Promise<StoredDraft> {
      return request('/v1/drafts', { method: 'PUT', body }) as Promise<StoredDraft>
    },
    deleteDraft(draftId: string): Promise<void> {
      return request(`/v1/drafts/${draftId}`, { method: 'DELETE' }).then(() => undefined)
    },
    getFavorites(workspaceId: string): Promise<FavoriteItem[]> {
      return request(`/v1/workspaces/${workspaceId}/favorites`) as Promise<FavoriteItem[]>
    },
    addFavorite(workspaceId: string, kind: SavedKind, targetId: string): Promise<FavoriteItem> {
      return request(`/v1/workspaces/${workspaceId}/favorites`, { method: 'PUT', body: { kind, targetId } }) as Promise<FavoriteItem>
    },
    removeFavorite(workspaceId: string, kind: SavedKind, targetId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/favorites/${kind}/${targetId}`, { method: 'DELETE' }).then(() => undefined)
    },
    orderFavorites(workspaceId: string, items: Array<{ kind: SavedKind; targetId: string }>): Promise<FavoriteItem[]> {
      return request(`/v1/workspaces/${workspaceId}/favorites/order`, { method: 'PUT', body: { items } }) as Promise<FavoriteItem[]>
    },
    getLater(workspaceId: string, state: LaterState = 'ACTIVE'): Promise<LaterItem[]> {
      return request(`/v1/workspaces/${workspaceId}/later`, { query: { state } }) as Promise<LaterItem[]>
    },
    addLater(workspaceId: string, targetId: string, note?: string | null, kind: LaterKind = 'TICKET'): Promise<LaterItem> {
      return request(`/v1/workspaces/${workspaceId}/later`, { method: 'POST', body: { kind, targetId, note: note ?? null } }) as Promise<LaterItem>
    },
    setReminder(laterId: string, body: { dueAt: string; timeZone: string }): Promise<ReminderInfo> {
      return request(`/v1/later/${laterId}/reminder`, { method: 'PUT', body }) as Promise<ReminderInfo>
    },
    cancelReminder(laterId: string): Promise<void> {
      return request(`/v1/later/${laterId}/reminder`, { method: 'DELETE' }).then(() => undefined)
    },
    getDocTree(workspaceId: string, projectId?: string): Promise<DocNode[]> {
      return (request(`/v1/workspaces/${workspaceId}/docs/tree`, { query: { projectId } }) as Promise<{ nodes: DocNode[] }>).then((r) => r.nodes)
    },
    createDoc(
      workspaceId: string,
      body: { title: string; parentId?: string | null; projectId?: string | null; visibility: DocVisibility; scopeProjectId?: string | null; scopeTeamId?: string | null; participants?: Array<{ userId: string; role: DocAccessRole }>; templateId?: string },
    ): Promise<DocPage> {
      return request(`/v1/workspaces/${workspaceId}/docs`, { method: 'POST', body }) as Promise<DocPage>
    },
    getDoc(docId: string): Promise<DocPage> {
      return request(`/v1/docs/${docId}`) as Promise<DocPage>
    },
    updateDoc(docId: string, body: { title?: string; projectId?: string | null }): Promise<DocPage> {
      return request(`/v1/docs/${docId}`, { method: 'PATCH', body }) as Promise<DocPage>
    },
    saveDoc(docId: string, body: { expectedRevisionNo: number; title: string; content: DocContent }): Promise<DocPage> {
      return request(`/v1/docs/${docId}/content`, { method: 'PUT', body }) as Promise<DocPage>
    },
    createDocVersion(docId: string, expectedRevisionNo: number): Promise<DocVersionSummary> {
      return request(`/v1/docs/${docId}/versions`, { method: 'POST', body: { expectedRevisionNo } }) as Promise<DocVersionSummary>
    },
    getDocVersions(docId: string, before?: number): Promise<{ items: DocVersionSummary[]; nextBefore: number | null }> {
      return request(`/v1/docs/${docId}/versions`, { query: { before } }) as Promise<{ items: DocVersionSummary[]; nextBefore: number | null }>
    },
    getDocVersion(docId: string, versionNo: number): Promise<DocVersion> {
      return request(`/v1/docs/${docId}/versions/${versionNo}`) as Promise<DocVersion>
    },
    moveDoc(docId: string, body: { parentId: string | null; beforeId?: string | null }): Promise<void> {
      return request(`/v1/docs/${docId}/move`, { method: 'POST', body }).then(() => undefined)
    },
    deleteDoc(docId: string): Promise<void> {
      return request(`/v1/docs/${docId}`, { method: 'DELETE' }).then(() => undefined)
    },
    setDocVisibility(docId: string, visibility: DocVisibility, scope: { scopeProjectId?: string | null; scopeTeamId?: string | null } = {}): Promise<DocPage> {
      return request(`/v1/docs/${docId}/visibility`, { method: 'PUT', body: { visibility, ...scope } }) as Promise<DocPage>
    },
    getDocParticipants(docId: string): Promise<DocParticipant[]> {
      return request(`/v1/docs/${docId}/participants`) as Promise<DocParticipant[]>
    },
    setDocParticipants(docId: string, participants: Array<{ userId: string; role: DocAccessRole }>): Promise<DocParticipant[]> {
      return request(`/v1/docs/${docId}/participants`, { method: 'PUT', body: { participants } }) as Promise<DocParticipant[]>
    },
    getDocComments(docId: string, cursor?: string): Promise<{ items: DocComment[]; nextCursor: string | null }> {
      return request(`/v1/docs/${docId}/comments`, { query: { cursor } }) as Promise<{ items: DocComment[]; nextCursor: string | null }>
    },
    createDocComment(docId: string, body: unknown, fileIds: string[] = []): Promise<DocComment> {
      return request(`/v1/docs/${docId}/comments`, { method: 'POST', body: fileIds.length > 0 ? { body, fileIds } : { body } }) as Promise<DocComment>
    },
    editDocComment(docId: string, commentId: string, body: unknown): Promise<DocComment> {
      return request(`/v1/docs/${docId}/comments/${commentId}`, { method: 'PATCH', body: { body } }) as Promise<DocComment>
    },
    deleteDocComment(docId: string, commentId: string): Promise<void> {
      return request(`/v1/docs/${docId}/comments/${commentId}`, { method: 'DELETE' }).then(() => undefined)
    },
    getDocTemplates(workspaceId: string): Promise<DocTemplate[]> {
      return request(`/v1/workspaces/${workspaceId}/doc-templates`) as Promise<DocTemplate[]>
    },
    createDocTemplate(workspaceId: string, name: string, fromDocId: string): Promise<DocTemplate> {
      return request(`/v1/workspaces/${workspaceId}/doc-templates`, { method: 'POST', body: { name, fromDocId } }) as Promise<DocTemplate>
    },
    deleteDocTemplate(workspaceId: string, templateId: string): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/doc-templates/${templateId}`, { method: 'DELETE' }).then(() => undefined)
    },
    attachDocFile(docId: string, fileId: string): Promise<void> {
      return request(`/v1/docs/${docId}/files`, { method: 'POST', body: { fileId } }).then(() => undefined)
    },
    resolveReferences(workspaceId: string, items: Array<{ kind: ReferenceKind; id: string }>): Promise<ResolvedReference[]> {
      return (request(`/v1/workspaces/${workspaceId}/references/resolve`, { method: 'POST', body: { items } }) as Promise<{ items: ResolvedReference[] }>).then((r) => r.items)
    },
    getTicketReferences(ticketId: string, cursor?: string): Promise<{ items: TicketReference[]; nextCursor: string | null }> {
      return request(`/v1/tickets/${ticketId}/references`, { query: { cursor, limit: 20 } }) as Promise<{ items: TicketReference[]; nextCursor: string | null }>
    },
    getDocReferences(docId: string, cursor?: string): Promise<{ items: TicketReference[]; nextCursor: string | null }> {
      return request(`/v1/docs/${docId}/references`, { query: { cursor, limit: 20 } }) as Promise<{ items: TicketReference[]; nextCursor: string | null }>
    },
    getFavoriteFolders(workspaceId: string): Promise<FavoriteFolder[]> {
      return (request(`/v1/workspaces/${workspaceId}/favorite-folders`) as Promise<{ items: FavoriteFolder[] }>).then((r) => r.items)
    },
    createFavoriteFolder(workspaceId: string, name: string): Promise<FavoriteFolder> {
      return request(`/v1/workspaces/${workspaceId}/favorite-folders`, { method: 'POST', body: { name } }) as Promise<FavoriteFolder>
    },
    renameFavoriteFolder(folderId: string, name: string): Promise<FavoriteFolder> {
      return request(`/v1/favorite-folders/${folderId}`, { method: 'PATCH', body: { name } }) as Promise<FavoriteFolder>
    },
    deleteFavoriteFolder(folderId: string): Promise<{ movedToTopLevel: number }> {
      return request(`/v1/favorite-folders/${folderId}`, { method: 'DELETE' }) as Promise<{ movedToTopLevel: number }>
    },
    orderFavoriteFolders(workspaceId: string, ids: string[]): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/favorite-folders/order`, { method: 'PUT', body: { ids } }).then(() => undefined)
    },
    moveFavorite(workspaceId: string, kind: SavedKind, targetId: string, folderId: string | null): Promise<void> {
      return request(`/v1/workspaces/${workspaceId}/favorites/${kind}/${targetId}/folder`, { method: 'PUT', body: { folderId } }).then(() => undefined)
    },
    getInbox(workspaceId: string, query: { filter?: InboxFilter; state?: 'all' | 'unread'; cursor?: string; limit?: number }): Promise<InboxPage> {
      return request(`/v1/workspaces/${workspaceId}/inbox`, { query }) as Promise<InboxPage>
    },
    inboxCommand(itemId: string, command: 'read' | 'unread' | 'archive' | 'unarchive' | 'snooze', body: { expectedVersion: number; until?: string }): Promise<{ id: string; state: string; version: number }> {
      return request(`/v1/inbox/${itemId}/commands/${command}`, { method: 'POST', body }) as Promise<{ id: string; state: string; version: number }>
    },
    inboxReadAll(workspaceId: string, filter?: InboxFilter): Promise<{ changed: number }> {
      return request(`/v1/workspaces/${workspaceId}/inbox/commands/read-all`, { method: 'POST', body: filter ? { filter } : {} }) as Promise<{ changed: number }>
    },
    getSubscription(kind: WatchTargetKind, targetId: string): Promise<Subscription> {
      return request(`${subscriptionPath(kind, targetId)}`) as Promise<Subscription>
    },
    watch(kind: WatchTargetKind, targetId: string): Promise<Subscription> {
      return request(subscriptionPath(kind, targetId), { method: 'PUT' }) as Promise<Subscription>
    },
    unwatch(kind: WatchTargetKind, targetId: string): Promise<Subscription> {
      return request(subscriptionPath(kind, targetId), { method: 'DELETE' }) as Promise<Subscription>
    },
    getSubscriptions(workspaceId: string, kind?: WatchTargetKind): Promise<{ items: Subscription[] }> {
      return request(`/v1/workspaces/${workspaceId}/subscriptions`, { query: { kind } }) as Promise<{ items: Subscription[] }>
    },
    getNotificationPreferences(): Promise<NotificationPreferences> {
      return request('/v1/me/notification-preferences') as Promise<NotificationPreferences>
    },
    saveNotificationPreferences(body: Pick<NotificationPreferences, 'level' | 'dnd' | 'external'> & { expectedVersion: number }): Promise<NotificationPreferences> {
      return request('/v1/me/notification-preferences', { method: 'PUT', body }) as Promise<NotificationPreferences>
    },
    getMutedChannels(): Promise<{ channelIds: string[] }> {
      return request('/v1/me/muted-channels') as Promise<{ channelIds: string[] }>
    },
    muteChannel(channelId: string, muted: boolean): Promise<void> {
      return request(`/v1/channels/${channelId}/mute`, { method: muted ? 'PUT' : 'DELETE' }).then(() => undefined)
    },
    getProjectPermissions(projectId: string): Promise<ProjectPermissions> {
      return request(`/v1/projects/${projectId}/permissions`) as Promise<ProjectPermissions>
    },
    setDesignation(projectId: string, userId: string, body: { canWaiveDependencies: boolean; canManagePins: boolean }): Promise<unknown> {
      return request(`/v1/projects/${projectId}/members/${userId}/designation`, { method: 'PUT', body })
    },
    search(
      workspaceId: string,
      query: { q: string; kind: SearchTab; stateIds?: string; cursor?: string; limit?: number; tz?: string },
    ): Promise<SearchResponse> {
      return request(`/v1/workspaces/${workspaceId}/search`, { query }) as Promise<SearchResponse>
    },
    updateLater(laterId: string, body: { expectedVersion: number; state?: LaterState; note?: string | null }): Promise<LaterItem> {
      return request(`/v1/later/${laterId}`, { method: 'PATCH', body }) as Promise<LaterItem>
    },
    removeLater(laterId: string): Promise<void> {
      return request(`/v1/later/${laterId}`, { method: 'DELETE' }).then(() => undefined)
    },
    getProjectPins(projectId: string): Promise<ProjectPins> {
      return request(`/v1/projects/${projectId}/pins`) as Promise<ProjectPins>
    },
    addProjectPin(projectId: string, body: { ticketId?: string; url?: string; title?: string | null }): Promise<ProjectPins> {
      return request(`/v1/projects/${projectId}/pins`, { method: 'POST', body }) as Promise<ProjectPins>
    },
    removePin(pinId: string): Promise<void> {
      return request(`/v1/pins/${pinId}`, { method: 'DELETE' }).then(() => undefined)
    },
    assignSprint(ticketId: string, sprintId: string | null, idempotencyKey: string): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}/commands/assign-sprint`, { method: 'POST', body: { sprintId }, idempotencyKey }) as Promise<Ticket>
    },
    createMilestone(
      projectId: string,
      body: { name: string; description?: string | null; targetOn?: string | null },
      idempotencyKey: string,
    ): Promise<Milestone> {
      return request(`/v1/projects/${projectId}/milestones`, { method: 'POST', body, idempotencyKey }) as Promise<Milestone>
    },
    updateMilestone(milestoneId: string, body: { expectedVersion: number; name?: string; state?: Milestone['state'] }): Promise<Milestone> {
      return request(`/v1/milestones/${milestoneId}`, { method: 'PATCH', body }) as Promise<Milestone>
    },
    setTicketMilestones(ticketId: string, milestoneIds: string[], idempotencyKey: string): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}/commands/set-milestones`, { method: 'POST', body: { milestoneIds }, idempotencyKey }) as Promise<Ticket>
    },
    changeTicketStatus(ticketId: string, body: { expectedRevision: number; statusId: string }, idempotencyKey: string): Promise<Ticket> {
      return request(`/v1/tickets/${ticketId}/commands/change-status`, { method: 'POST', body, idempotencyKey }) as Promise<Ticket>
    },
    getTicketComments(ticketId: string): Promise<TicketComment[]> {
      return request(`/v1/tickets/${ticketId}/comments`) as Promise<TicketComment[]>
    },
    createTicketComment(ticketId: string, body: unknown, fileIds: string[] = []): Promise<TicketComment> {
      return request(`/v1/tickets/${ticketId}/comments`, { method: 'POST', body: { body, fileIds } }) as Promise<TicketComment>
    },
    editTicketComment(ticketId: string, commentId: string, body: unknown): Promise<TicketComment> {
      return request(`/v1/tickets/${ticketId}/comments/${commentId}`, { method: 'PATCH', body: { body } }) as Promise<TicketComment>
    },
    deleteTicketComment(ticketId: string, commentId: string): Promise<void> {
      return request(`/v1/tickets/${ticketId}/comments/${commentId}`, { method: 'DELETE' }).then(() => undefined)
    },
    getTicketAttachments(ticketId: string): Promise<TicketAttachment[]> {
      return request(`/v1/tickets/${ticketId}/attachments`) as Promise<TicketAttachment[]>
    },
    attachTicketFile(ticketId: string, fileId: string): Promise<TicketAttachment> {
      return request(`/v1/tickets/${ticketId}/attachments`, { method: 'POST', body: { fileId } }) as Promise<TicketAttachment>
    },
    deleteTicketAttachment(ticketId: string, attachmentId: string): Promise<void> {
      return request(`/v1/tickets/${ticketId}/attachments/${attachmentId}`, { method: 'DELETE' }).then(() => undefined)
    },
    createChannel(
      workspaceId: string,
      body: {
        name: string
        visibility: 'PUBLIC' | 'PRIVATE'
        description?: string | null
        teamId?: string | null
        projectId?: string | null
      },
    ): Promise<Channel> {
      return request(`/v1/workspaces/${workspaceId}/channels`, { method: 'POST', body }).then(normalizeChannel)
    },
    joinChannel(channelId: string): Promise<void> {
      return request(`/v1/channels/${channelId}/join`, { method: 'POST', body: {} }).then(() => undefined)
    },
    getChannelMembers(channelId: string): Promise<ChannelMember[]> {
      return request(`/v1/channels/${channelId}/members`).then((payload) => itemsOf<ChannelMember>(payload))
    },
    addChannelMember(channelId: string, userId: string): Promise<ChannelMember> {
      return request(`/v1/channels/${channelId}/members/${userId}`, { method: 'PUT', body: {} }) as Promise<ChannelMember>
    },
    /** Makes a member of a private channel its manager or a plain member. */
    setChannelMemberRole(channelId: string, userId: string, role: 'MANAGER' | 'MEMBER'): Promise<ChannelMember> {
      return request(`/v1/channels/${channelId}/members/${userId}`, { method: 'PATCH', body: { role } }) as Promise<ChannelMember>
    },
    /** The caller leaves a private channel; nobody can take another person out. */
    leaveChannel(channelId: string, userId: string): Promise<void> {
      return request(`/v1/channels/${channelId}/members/${userId}`, { method: 'DELETE' }) as Promise<void>
    },
    updateChannel(
      channelId: string,
      body: { visibility?: 'PUBLIC' | 'PRIVATE'; description?: string | null },
    ): Promise<Channel> {
      return request(`/v1/channels/${channelId}`, { method: 'PATCH', body }).then(normalizeChannel)
    },
    createDm(workspaceId: string, body: { userIds: string[] }): Promise<Dm> {
      return request(`/v1/workspaces/${workspaceId}/dms`, { method: 'POST', body }).then(normalizeDm)
    },
    getConversation(conversationId: string): Promise<Conversation> {
      return request(`/v1/conversations/${conversationId}`).then(normalizeConversation)
    },
    listMessages(conversationId: string, query: { beforeSeq?: number; limit: number }): Promise<Message[]> {
      return request(`/v1/conversations/${conversationId}/messages`, { query }).then(normalizeMessages)
    },
    listThread(rootId: string, query: { afterSeq?: number; limit: number }): Promise<ThreadPage> {
      return request(`/v1/messages/${rootId}/thread`, { query }).then(normalizeThread)
    },
    createFile(workspaceId: string, body: { filename: string; byteSize: number }): Promise<FileObject> {
      return request(`/v1/workspaces/${workspaceId}/files`, { method: 'POST', body }).then(normalizeFile)
    },
    deleteFile(fileId: string): Promise<void> {
      return request(`/v1/files/${fileId}`, { method: 'DELETE' }).then(() => undefined)
    },
    sendMessage(conversationId: string, body: SendMessageRequest, idempotencyKey: string): Promise<Message> {
      return request(`/v1/conversations/${conversationId}/messages`, {
        method: 'POST',
        body,
        idempotencyKey,
      }).then(normalizeMessage)
    },
    editMessage(messageId: string, body: EditMessageRequest, idempotencyKey: string): Promise<Message> {
      return request(`/v1/messages/${messageId}`, { method: 'PATCH', body, idempotencyKey }).then(normalizeMessage)
    },
    deleteMessage(messageId: string, idempotencyKey: string): Promise<void> {
      return request(`/v1/messages/${messageId}`, { method: 'DELETE', idempotencyKey }).then(() => undefined)
    },
    messageHistory(messageId: string): Promise<MessageHistory> {
      return request(`/v1/messages/${messageId}/history`).then(normalizeHistory)
    },
    putReaction(messageId: string, body: ReactionRequest, idempotencyKey: string): Promise<void> {
      return request(`/v1/messages/${messageId}/reactions`, { method: 'PUT', body, idempotencyKey }).then(
        () => undefined,
      )
    },
    deleteReaction(messageId: string, body: ReactionRequest, idempotencyKey: string): Promise<void> {
      return request(`/v1/messages/${messageId}/reactions`, { method: 'DELETE', body, idempotencyKey }).then(
        () => undefined,
      )
    },
    markConversationRead(conversationId: string, body: ReadRequest, idempotencyKey: string): Promise<ReadResult> {
      return request(`/v1/conversations/${conversationId}/read`, { method: 'POST', body, idempotencyKey }).then(
        (payload) => normalizeRead(payload, body.readThroughSeq),
      )
    },
    markConversationUnread(
      conversationId: string,
      body: ManualUnreadRequest,
      idempotencyKey: string,
    ): Promise<void> {
      return request(`/v1/conversations/${conversationId}/manual-unread`, {
        method: 'POST',
        body,
        idempotencyKey,
      }).then(() => undefined)
    },
    getThreadRead(rootId: string): Promise<ReadResult> {
      return request(`/v1/threads/${rootId}/read`).then((payload) => normalizeRead(payload, 0))
    },
    markThreadRead(rootId: string, body: ReadRequest, idempotencyKey: string): Promise<ReadResult> {
      return request(`/v1/threads/${rootId}/read`, { method: 'POST', body, idempotencyKey }).then((payload) =>
        normalizeRead(payload, body.readThroughSeq),
      )
    },
    markThreadUnread(rootId: string, body: ManualUnreadRequest, idempotencyKey: string): Promise<void> {
      return request(`/v1/threads/${rootId}/manual-unread`, { method: 'POST', body, idempotencyKey }).then(
        () => undefined,
      )
    },
    listChanges(conversationId: string, afterSeq: number): Promise<ConversationChange[]> {
      return request(`/v1/conversations/${conversationId}/changes`, { query: { afterSeq } }).then(normalizeChanges)
    },
    searchDirectory(
      workspaceId: string,
      query: { q?: string; types?: string; limit?: number },
    ): Promise<DirectorySearch> {
      return request(`/v1/workspaces/${workspaceId}/search`, { query }).then(normalizeDirectorySearch)
    },
    organizationDirectory(workspaceId: string, organizationId: string): Promise<OrganizationDirectory> {
      return request(`/v1/workspaces/${workspaceId}/organizations/${organizationId}`).then(normalizeOrganizationDirectory)
    },
  }
}

export type PenfiiClient = ReturnType<typeof createPenfiiClient>
