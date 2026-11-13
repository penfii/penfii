import type { components } from './generated'

/**
 * Types written out of `api/openapi.json` (see `generated.ts`, made by `npm run contract:types`). A model whose
 * schema says everything the client relies on is an alias of its generated type, so a change of the server's
 * schema shows up as a type error here and in the code that uses it. The models still written by hand below
 * are those whose schema is looser than what the server sends (a required list left out, a closed set of
 * values written as a plain string): they stay hand-written until their schema is tightened.
 */
type Schema<Name extends keyof components['schemas']> = components['schemas'][Name]

/** A generated type with some values narrowed to the closed set they really take. Each narrowed key must exist and be a subtype. */
type Narrow<Generated, Narrowed extends { [Key in keyof Narrowed]: Key extends keyof Generated ? (Narrowed[Key] extends Generated[Key] ? Narrowed[Key] : never) : never }> = Omit<Generated, keyof Narrowed> & Narrowed

export type RichTextMark =
  | { type: 'bold' }
  | { type: 'italic' }
  | { type: 'strike' }
  | { type: 'code' }
  | { type: 'link'; attrs: { href: string } }

export type RichTextText = {
  type: 'text'
  text: string
  marks?: RichTextMark[]
}

export type RichTextMention = {
  type: 'mention'
  attrs: {
    occurrenceId: string
    kind: 'USER'
    userId: string
  }
}

export type RichTextInline = RichTextText | RichTextMention

export type RichTextParagraph = {
  type: 'paragraph'
  content?: RichTextInline[]
}

export type RichTextHeading = {
  type: 'heading'
  attrs?: { level?: number }
  content?: RichTextInline[]
}

export type RichTextCodeBlock = {
  type: 'codeBlock'
  attrs?: { language?: string | null }
  content?: RichTextText[]
}

export type RichTextListItem = {
  type: 'listItem'
  content?: RichTextBlock[]
}

export type RichTextBulletList = {
  type: 'bulletList'
  content?: RichTextListItem[]
}

export type RichTextOrderedList = {
  type: 'orderedList'
  attrs?: { start?: number }
  content?: RichTextListItem[]
}

export type RichTextBlockquote = {
  type: 'blockquote'
  content?: RichTextBlock[]
}

export type RichTextBlock =
  | RichTextParagraph
  | RichTextHeading
  | RichTextCodeBlock
  | RichTextBulletList
  | RichTextOrderedList
  | RichTextBlockquote

export type RichTextDoc = {
  type: 'doc'
  content: RichTextBlock[]
}

export type ServerInfo = {
  apiVersion: string
  features: unknown
  limits: {
    messageBodyMaxBytes: number | null
    fileMaxBytes: number | null
    sessionTtlSeconds: number | null
  }
}

/** The signed-in person: what `GET /v1/me` returns, the schema `User`. */
export type Me = User

/** A workspace member as every other member may see them. */
export type WorkspacePerson = {
  userId: string
  displayName: string
  email: string
  role: string
  state: string
  jobTitle: string | null
  timeZone: string | null
  statusEmoji: string | null
  statusText: string | null
  statusClearAt: string | null
  hasAvatar: boolean
  teams: Array<{ id: string; name: string }>
}

export type Workspace = {
  id: string
  name: string
  timeZone: string
  /** The signed-in person's role in this Workspace: OWNER, ADMIN or MEMBER. */
  role?: string
}

/** The last e-mail delivery of an invitation link. It is separate from the invitation: FAILED never means the link does not work. */
export type InvitationEmailStatus = 'NOT_CONFIGURED' | 'SENT' | 'FAILED'

/** What creating or re-issuing an invitation answers. The link token is only ever in this answer. */
export type InvitationCreated = {
  id: string
  email: string
  inviteToken: string
  /** The e-mail already had an open invitation: it got a new link and the previous link stopped working. */
  reissued: boolean
  emailStatus: InvitationEmailStatus
  /** A fixed short phrase, with FAILED. */
  emailError?: string
}

/** What became of an invitation's Team step. ARCHIVED and GONE mean the person joined the Workspace only. */
export type InvitationTeamOutcome = Schema<'InvitationTeamOutcome'>

/** One Project linked to a Team, as the viewer sees it: the Team's role and the viewer's own access only. */
export type TeamProject = Schema<'TeamProject'>

export type Team = {
  id: string
  name: string
  /** Optional plain text. */
  description?: string | null
  /** Set while the Team is archived. */
  archivedAt?: string | null
  /** Department this team is shown under. Null means it is not placed. A department grants no access. */
  departmentId?: string | null
}

/** A display-only branch of the organization. It does not grant access. */
export type Department = {
  id: string
  workspaceId: string
  parentId: string | null
  name: string
  position: number
}

export type Project = {
  id: string
  name: string
  /** Fixed, upper case Ticket Key prefix such as `PFI`. */
  ticketPrefix?: string
  teamIds: string[]
  /** Whether the person takes part in the Project. Everyone in the Workspace finds it; only participants open it. */
  participating?: boolean
  /** Whether the person is a Project admin (settings, workflow, trash, channels). */
  canManage?: boolean
}

export type PrefixStatus = { value: string; valid: boolean; available: boolean }

/** A Prefix suggested for a Project name, and whether a typed one can be used. */
export type PrefixCheck = { suggestion: string | null; prefix: PrefixStatus | null }

export type WorkflowState = {
  id: string
  projectId: string
  name: string
  category: 'NOT_STARTED' | 'IN_PROGRESS' | 'DONE' | 'CANCELED'
  position: number
  isInitial: boolean
  /** Warning-only limit on the Tickets a viewer sees in this State. */
  wipLimit: number | null
}

export type TicketLabel = { id: string; name: string; color: string | null }

export type HierarchyKind = 'EPIC' | 'TASK' | 'SUBTASK'

export type TicketType = { id: string; name: string; color: string | null }

/** A related Ticket the viewer can see, for example a parent. */
export type TicketRef = Narrow<Schema<'TicketRef'>, { hierarchyKind: HierarchyKind }>

/** Direct children the viewer can see. Hidden children are never counted. */
export type ChildSummary = Schema<'ChildSummary'>

export type ProjectMember = { userId: string; displayName: string }

export type EstimateUnit = 'NONE' | 'POINTS' | 'HOURS'

/** A Task estimate: a decimal string with at most two decimals, in the Project unit it was written in. */
export type TicketEstimate = Schema<'TicketEstimate'>

/**
 * A planned schedule in working minutes. `plannedEndAt` is never stored: the server derives it from
 * the Project's current calendar and sets it only when one Ticket is read or changed (null in
 * lists, without a calendar, or beyond the five-year horizon).
 */
export type TicketSchedule = {
  plannedStartAt: string | null
  durationMinutes: number | null
  /** A schedule node of duration 0. It is not a Project Milestone. */
  isScheduleMilestone: boolean
  plannedEndAt?: string | null
}

/** An estimate as sent. `unit`, when given, must be the Project's unit. */
export type EstimateInput = Schema<'EstimateInput'>

/** A schedule as sent. `plannedEndAt` is derived by the server and ignored if sent. */
export type ScheduleInput = Schema<'ScheduleInput'>

export type Ticket = {
  id: string
  workspaceId: string
  projectId: string
  channelId: string | null
  key: string
  title: string
  description: RichTextDoc | Record<string, unknown>
  hierarchyKind: HierarchyKind
  parentId: string | null
  statusId: string
  assigneeId: string | null
  assigneeName: string | null
  labels: TicketLabel[]
  priority: string | null
  dueOn: string | null
  ticketType: TicketType | null
  parent: TicketRef | null
  childSummary: ChildSummary
  dependencies: DependencySummary
  /** Current Sprint. A Subtask shows its parent Task's Sprint. */
  sprint: SprintRef | null
  milestones: MilestoneRef[]
  /** When the Ticket was last put at the top of the shared order by hand. */
  movedToTopAt?: string | null
  /** Task only; null is unestimated, a value of "0" is an explicit 0. */
  estimate?: TicketEstimate | null
  schedule?: TicketSchedule | null
  revisionNo: number
  createdAt: string
  updatedAt: string
  /** Set while the Ticket is archived: readable, read-only, out of the default lists. */
  archivedAt?: string | null
}

/** Dependency counts the viewer can see. A Ticket they cannot see is never counted. */
export type DependencySummary = Schema<'DependencySummary'>

export type SprintState = 'PLANNED' | 'ACTIVE' | 'COMPLETED' | 'CANCELED'
export type SprintRef = Narrow<Schema<'SprintRef'>, { state: SprintState }>

export type MilestoneState = 'OPEN' | 'REACHED' | 'CANCELED'
export type MilestoneRef = Narrow<Schema<'MilestoneRef'>, { state: MilestoneState }>

export type BoardColumn = {
  id: string
  name: string
  category: WorkflowState['category']
  position: number
  /** Kept for the single-state column the server sends today. */
  stateId: string
  /** Workflow states shown in this column. A column may map to several states. */
  stateIds: string[]
  /** Tickets the server counted for this viewer before any View filter. */
  ticketCount: number
  wipLimit: number | null
  tickets: Ticket[]
}

export type ProjectBoard = {
  projectId: string
  workflowVersion: number
  /** The Project's estimate unit; Task estimates are written in it. */
  estimateUnit?: EstimateUnit
  canEdit: boolean
  canManageWorkflow: boolean
  states: WorkflowState[]
  columns: BoardColumn[]
}

export type TicketComment = {
  id: string
  ticketId: string
  authorId: string
  authorName: string | null
  body: RichTextDoc | Record<string, unknown>
  createdAt: string
  editedAt: string | null
  attachments: TicketAttachment[]
  /** What the requester may do: only the author edits; the author, a Project admin and a channel manager delete. */
  canEdit: boolean
  canDelete: boolean
}

/** A file on a Ticket's body (`commentId` null) or on one of its comments. Download it through `/v1/attachments/{id}/content`. */
export type TicketAttachment = Schema<'TicketAttachment'>

export type TicketField =
  | 'title'
  | 'description'
  | 'status'
  | 'type'
  | 'priority'
  | 'assignee'
  | 'dueOn'
  | 'labels'
  | 'parent'
  | 'estimate'
  | 'schedule'

/**
 * One changed field with values a person can read. Labels are a list of names, an estimate is
 * `[value, unit]`, a schedule is `[plannedStartAt, durationMinutes, 'milestone' or '']`
 * with an empty string where a part is not set.
 */
export type FieldChange = { field: TicketField; from: string | string[] | null; to: string | string[] | null }

export type TicketActivity = {
  id: string
  ticketId: string
  revisionNo: number | null
  activityKind: string
  actorUserId: string | null
  actorName: string | null
  data: Record<string, unknown>
  /** Name of the MCP client that did this on the person's behalf; absent for work done in Penfii. */
  via?: string
  changes: FieldChange[]
  occurredAt: string
}

export type ActivityPage = { items: TicketActivity[]; nextCursor: string | null }

/** A History entry. Values stay on the server until one revision is opened. */
export type RevisionSummary = {
  revisionNo: number
  actorUserId: string | null
  actorName: string | null
  recordedAt: string
  changedFields: TicketField[]
  restoreSourceRevisionNo: number | null
  /** Name of the MCP client that made this version for the person, when one did. */
  via?: string
}

export type RevisionPage = { items: RevisionSummary[]; nextBefore: number | null }

export type RestorableField = { field: TicketField; applicable: boolean; reason: string | null }

export type RevisionDetail = {
  revisionNo: number
  currentRevisionNo: number
  actorUserId: string | null
  actorName: string | null
  recordedAt: string
  restoreSourceRevisionNo: number | null
  changesFromPrevious: FieldChange[]
  /** `from` is the value at this revision, `to` the value now. */
  changesToCurrent: FieldChange[]
  restorable: RestorableField[]
}

export type ChannelVisibility = 'PUBLIC' | 'PRIVATE'

export type Channel = {
  id: string
  name: string
  description?: string | null
  visibility: ChannelVisibility
  teamId: string | null
  projectId: string | null
  /** Channel membership role when the caller has one. */
  role?: string | null
  /** Set only when the server sends a boolean. Absent means membership is unknown. */
  joined?: boolean
  /** Whether any member, not only a manager, may put people into the channel. */
  allowMemberInvites?: boolean
  /** Conversation to open. Falls back to `id` when the server omits it. */
  conversationId: string
  unreadCount: number
}

/** One person of a channel. `role` is `MANAGER` or `MEMBER`. */
export type ChannelMember = {
  userId: string
  displayName: string
  role: 'MANAGER' | 'MEMBER'
  joinedAt: string
}

export type ConversationPeer = Schema<'ConversationPeer'>

export type Dm = {
  id: string
  memberIds: string[]
  /** Conversation to open. Falls back to `id` when the server omits it. */
  conversationId: string
  peers: ConversationPeer[]
  lastMessagePreview: string | null
  lastMessageAt: string | null
  /** Null when the server omits the counter. Zero means no message has been sent. */
  lastMessageSeq: number | null
  unreadCount: number
}

export type Space = {
  teams: Team[]
  projects: Project[]
  channels: Channel[]
  dms: Dm[]
}

export type Conversation = {
  id: string
  generation: number | null
  readThroughSeq: number | null
  manualUnreadFromSeq: number | null
  title: string | null
  kind: string | null
  /** Change log cursor at the moment this snapshot was read. The live socket starts just after it. */
  lastChangeSeq: number
}

export type Reaction = {
  emoji: string
  count: number
  reactedByMe: boolean
  userIds: string[]
}

export type Message = {
  id: string
  streamSeq: number
  authorId: string
  authorDisplayName: string
  authorKind: 'HUMAN' | 'INTEGRATION'
  version: number
  deleted: boolean
  body: RichTextDoc | null
  createdAt: string
  editedAt: string | null
  replyCount: number
  replyToId: string | null
  threadRootId: string | null
  reactions: Reaction[]
  attachments: MessageAttachment[]
}

export type CreateWebhookRequest = Schema<'CreateWebhookRequest'>

export type Webhook = {
  id: string
  channelId: string
  name: string
  authMode: string
  actorId: string
  actorDisplayName: string
  revoked: boolean
  createdAt: string
  rotatedAt: string | null
}

export type WebhookCreated = {
  webhook: Webhook
  secret: string
}

export type WebhookList = {
  items: Webhook[]
}

export type MessageAttachment = Schema<'MessageAttachment'>

export type FileObject = {
  id: string
  generation: number
  filename: string
  byteSize: number
  mime: string | null
  state: string
}

export type User = Schema<'User'>
export type RichText = RichTextDoc

export type ReadState = {
  generation: number
  readThroughSeq: number
  manualUnreadFromSeq: number | null
}

export type Draft = {
  id: string
  generation: string
  version: number
  consumedVersion: number
  state: string
  payload: DraftPayload
}

export type MessageVersion = {
  version: number
  body: RichTextDoc | null
  editedAt: string | null
}

export type MessageHistory = {
  versions: MessageVersion[]
}

export type SendMessageRequest = {
  bodySchemaVersion: 1
  body: RichTextDoc
  threadRootId: string | null
  replyToId: string | null
  fileIds?: string[]
}

export type EditMessageRequest = {
  expectedVersion: number
  body: RichTextDoc
}

export type ReactionRequest = Schema<'ReactionRequest'>

export type ReadRequest = {
  generation: number
  readThroughSeq: number
  scope: 'auto' | 'explicit'
  observed: true
}

export type ReadResult = {
  generation: number
  readThroughSeq: number | null
  manualUnreadFromSeq: number | null
}

export type ManualUnreadRequest = {
  fromSeq: number
}

export type ConversationChange = {
  conversationId: string
  changeSeq: number
  messageId: string
  messageVersion: number
  changeKind: string
}

export type ClientHello = {
  v: 1
  type: 'hello'
  afterChangeSeq: number
}

export type ServerFrame =
  | {
      v: 1
      type: 'conversation.change'
      conversationId: string
      changeSeq: number
      messageId: string
      messageVersion: number
      changeKind: string
    }
  | { v: 1; type: 'resync_required' }

export type DraftPayload = {
  schemaVersion: 1
  body: RichTextDoc
}

export type DraftRecord = {
  contextKind: string
  contextId: string
  generation: string
  version: number
  payload: DraftPayload
}

export type PutDraftRequest = {
  contextKind: string
  contextId: string
  /** The input session: a time-ordered UUID the client makes when the box goes from empty to typed. */
  generation: string
  expectedVersion?: number
  payload: DraftPayload
}

export type DraftPointer = {
  generation: string
  version: number
}

export type ThreadPage = {
  generation: number | null
  messages: Message[]
}

export type DirectoryPerson = {
  id: string
  displayName: string
  username: string | null
  email: string
  organizationId: string
  organizationName: string
  affiliation: string[]
}

export type DirectoryOrganization = {
  id: string
  name: string
  timeZone: string
  memberCount: number
}

export type DirectoryMessageHit = {
  id: string
  conversationId: string
  authorId: string
  authorName: string
  snippet: string
  createdAt: string
}

export type DirectoryHit = {
  type: 'person' | 'organization' | 'message'
  id: string
  title: string
  subtitle: string
  person?: DirectoryPerson
  organization?: DirectoryOrganization
  message?: DirectoryMessageHit
}

export type DirectorySearch = {
  query: string
  results: DirectoryHit[]
}

export type OrganizationDirectory = {
  organization: DirectoryOrganization
  members: DirectoryPerson[]
}

export type DependencyEdge = {
  id: string
  ticket: TicketRef
  stateName: string
  stateCategory: WorkflowState['category']
  /** Whether this edge lets the waiting Ticket complete (predecessor DONE, or the relation is waived). */
  satisfied: boolean
  createdAt: string
  /** BLOCKS is a forced dependency; RELATED is a plain link that never blocks. */
  kind: 'BLOCKS' | 'RELATED'
  waived: boolean
  waiveReason: string | null
  /** The predecessor was canceled: completion can never satisfy this edge. */
  predecessorCanceled: boolean
  /** The relation no longer fits the Tickets' visibility and needs a person to look at it. */
  needsReview: boolean
}

export type TicketDependencies = {
  ticketId: string
  /** Predecessors this Ticket waits for. */
  blockedBy: DependencyEdge[]
  /** Tickets that wait for this one. */
  blocks: DependencyEdge[]
  /** Plain RELATED links, in both directions. */
  related: DependencyEdge[]
  canEdit: boolean
  /** Whether the viewer may waive or restore a dependency here (a Project admin). */
  canWaive: boolean
}

export type ViewFilter = {
  labelIds: string[]
  assigneeIds: string[]
  unassigned: boolean
  priorities: string[]
  typeIds: string[]
  kinds: HierarchyKind[]
  sprintIds: string[]
  noSprint: boolean
  milestoneIds: string[]
  blocked: boolean | null
  text: string | null
  /** State categories; the same words mean the same thing in every Project. */
  categories: WorkflowState['category'][]
  /** Channels of the Project (Project scope only). */
  channelIds: string[]
  /** Projects of a Team or Workspace scope. */
  projectIds: string[]
  /** Has an estimate, has none, or compares (inside one Project only). Absent in older Views. */
  estimate?: EstimateFilter | null
}

export type EstimateFilter = { op: 'SET' | 'UNSET' | 'GTE' | 'LTE'; value?: string | null }

export type ViewGroup = 'NONE' | 'STATUS' | 'CATEGORY' | 'PROJECT' | 'MILESTONE' | 'ASSIGNEE' | 'PRIORITY' | 'SPRINT'
export type ViewSortField = 'RANK' | 'PRIORITY' | 'DUE_ON' | 'CREATED' | 'UPDATED' | 'TITLE' | 'ESTIMATE'
export type ViewLayout = 'BOARD' | 'LIST' | 'TABLE'
export type TableColumn = 'KEY' | 'TITLE' | 'STATE' | 'ASSIGNEE' | 'PRIORITY' | 'LABELS' | 'PROJECT' | 'CHANNEL' | 'SPRINT' | 'MILESTONE' | 'PARENT' | 'DUE_ON' | 'ESTIMATE'

/** How a View presents what it selects. Display never changes which Tickets are shown. */
export type ViewDisplay = {
  /** Show Subtasks as Tickets of their own. */
  subtasks: boolean
  compact: boolean
  /** Properties a Table shows; empty means the default set. */
  columns: TableColumn[]
}

/** How a View shows Tickets: which ones, grouped how, in what order, presented how (schema v2). */
export type ViewDefinition = {
  filter: ViewFilter
  group: ViewGroup
  sort: { field: ViewSortField; dir: 'ASC' | 'DESC' }
  display: ViewDisplay
}

export type ViewTargetKind = 'PROJECT' | 'TEAM' | 'WORKSPACE'

export type ProjectView = {
  id: string
  /** Null for a Team or Workspace View. */
  projectId: string | null
  targetKind: ViewTargetKind
  teamId: string | null
  name: string
  layout: ViewLayout
  scope: 'PERSONAL' | 'PROJECT'
  ownerId: string
  definition: ViewDefinition
  version: number
  canEdit: boolean
  createdAt: string
  updatedAt: string
}

export type Sprint = {
  id: string
  projectId: string
  name: string
  goal: string | null
  state: SprintState
  startsOn: string
  endsOn: string
  version: number
  startedAt: string | null
  endedAt: string | null
  /** IANA zone the Sprint's days are counted in; fixed when it starts. */
  timeZone?: string
  /** Copied from the Project when created; fixed when the Sprint starts. */
  estimateUnit?: EstimateUnit
}

export type Milestone = {
  id: string
  projectId: string
  name: string
  description: string | null
  targetOn: string | null
  state: MilestoneState
  /** When it became REACHED. Null if it is not reached, or it was reached before this was recorded. */
  reachedAt?: string | null
  version: number
}

export type ProjectPlanning = {
  canManage: boolean
  sprints: Sprint[]
  milestones: Milestone[]
}

/** Where a dragged Ticket lands in the Project's shared order. At most one is given. */
export type MovePlacement = { afterId?: string; beforeId?: string; position?: 'first' | 'last' }

/** What closing a Sprint would do, shown before the person confirms. */
export type SprintTarget = { id: string; name: string; state: SprintState; startsOn: string }

export type SprintClosePreview = {
  sprintId: string
  sprintVersion: number
  done: TicketRef[]
  canceled: TicketRef[]
  /** Unfinished Tasks the viewer can see. Others are released and never listed or counted. */
  unfinished: TicketRef[]
  targets: SprintTarget[]
  /** The next Sprint, or null (the backlog). */
  defaultTargetId: string | null
}

export type SprintOutcome = 'DONE' | 'CANCELED' | 'NOT_DONE' | 'REMOVED'

export type SprintResult = {
  ticket: TicketRef
  outcome: SprintOutcome | null
  carriedTo: SprintTarget | null
  carriedFrom: SprintTarget | null
  needsNextAssignment: boolean
  inBaseline: boolean
  assignedAt: string
  endedAt: string | null
}

export type SprintResults = {
  sprint: Sprint
  baselineCount: number | null
  baselineRecordedAt: string | null
  results: SprintResult[]
}

/** A Ticket in a Team or Workspace scope with the places it lives in. */
export type ScopedTicket = Ticket & {
  projectName: string
  channelName: string | null
  stateName: string
  stateCategory: WorkflowState['category']
  /** Whether the viewer follows this Ticket (Watch). */
  watching?: boolean
}

export type ScopeProject = { id: string; name: string; canEdit: boolean }

export type ScopeDataset = {
  projects: ScopeProject[]
  tickets: ScopedTicket[]
  /** True when the scope holds more Tickets than were returned. */
  truncated: boolean
}

export type ScopeLens = 'ALL' | 'ASSIGNED' | 'CREATED' | 'WATCHING'

export type BulkChange =
  | { type: 'STATUS'; statusId: string }
  | { type: 'ASSIGNEE'; assigneeId: string | null }
  | { type: 'PRIORITY'; priority: string | null }
  | { type: 'ADD_LABELS' | 'REMOVE_LABELS'; labelIds: string[] }
  | { type: 'SPRINT'; sprintId: string | null }
  | { type: 'ADD_MILESTONES' | 'REMOVE_MILESTONES'; milestoneIds: string[] }
  | { type: 'ARCHIVE' | 'TRASH' }

export type BulkItemResult = { ticketId: string; key: string | null; ok: boolean; code: string | null; message: string | null }
export type BulkResult = { total: number; succeeded: number; failed: number; items: BulkItemResult[] }

export type SavedKind = 'PROJECT' | 'TEAM' | 'TICKET' | 'VIEW' | 'CHANNEL'
/** What Later can hold. */
export type LaterKind = 'TICKET' | 'MESSAGE' | 'FILE'

/** What a personal shortcut points at. A target the viewer can no longer open has no title. */
export type SavedRef = {
  kind: SavedKind | LaterKind
  targetId: string
  available: boolean
  title: string | null
  subtitle: string | null
  projectId: string | null
  teamId: string | null
  viewTarget: ViewTargetKind | null
  layout: ViewLayout | null
  /** For a Message or File: where to open it. */
  conversationId?: string | null
  threadRootId?: string | null
  messageId?: string | null
}

type SavedRefBase = Omit<SavedRef, 'kind'>
export type FavoriteItem = SavedRefBase & { kind: SavedKind; position: number; createdAt: string; folderId?: string | null }
export type FavoriteFolder = { id: string; name: string; position: number }

export type LaterState = 'ACTIVE' | 'DONE' | 'ARCHIVED'
export type ReminderInfo = { id: string; dueAt: string; timeZone: string; state: 'SCHEDULED' | 'FIRED' | 'CANCELED'; version: number; firedAt: string | null }
export type LaterItem = SavedRefBase & {
  kind: LaterKind
  id: string
  note: string | null
  state: LaterState
  version: number
  createdAt: string
  updatedAt: string
  reminder?: ReminderInfo | null
}

export type PinItem = {
  id: string
  kind: 'TICKET' | 'URL'
  title: string
  url: string | null
  ticket: TicketRef | null
  position: number
  pinnedBy: string
  createdAt: string
}

export type ProjectPins = { canManage: boolean; pins: PinItem[] }

/** One unsent message of the caller, for the Drafts list. */
export type DraftItem = {
  id: string
  contextKind: 'CHANNEL_COMPOSE' | 'THREAD_COMPOSE' | 'DM_COMPOSE'
  contextId: string
  conversationId: string
  threadRootId: string | null
  /** Channel name; null for a direct message. */
  title: string | null
  projectId: string | null
  preview: string
  updatedAt: string
}

export type StoredDraft = {
  id: string
  contextKind: string
  contextId: string
  generation: string
  version: number
  consumedVersion: number
  state: string
  payload: DraftPayload
  updatedAt: string
}


// ---- Search ----------------------------------------------------------------------------------------

export type SearchTab = 'all' | 'message' | 'ticket' | 'file' | 'space' | 'view' | 'person' | 'doc'
export type SearchSegment = { text: string; hit: boolean }
export type SearchResult = {
  kind: 'message' | 'ticket' | 'file' | 'space' | 'view' | 'person' | 'doc'
  id: string
  title: string
  subtitle: string | null
  snippet: SearchSegment[]
  createdAt: string | null
  fields: Record<string, unknown>
}
export type SearchResponse = {
  query: string
  normalized: unknown
  results: SearchResult[]
  nextCursor: string | null
  status: { ready: boolean; unavailableKinds: string[]; stale: boolean }
}

// ---- Inbox, Watch, notification preferences -----------------------------------------------------------

/** The notification types a person can filter by. A notification has one kind; a type is a set of kinds. */
export type InboxType = 'mentions' | 'replies' | 'assignments' | 'reminders' | 'invitations'
/** What the server accepts as `filter`: one type, `all`, or the old `watching` (kept for old links; not a type shown to people). */
export type InboxFilter = 'all' | InboxType | 'watching'
/** `inbox` lists what is not archived; `archived` lists the archive. */
export type InboxView = 'inbox' | 'archived'
export type InboxKind =
  | 'mention.user'
  | 'reply.direct'
  | 'thread.reply'
  | 'dm.message'
  | 'ticket.assigned'
  | 'ticket.commented'
  | 'ticket.status_changed'
  | 'ticket.field_changed'
  | 'dependency.released'
  | 'sprint.started'
  | 'sprint.closed'
  | 'reminder.due'
  | 'channel.invited'
export type InboxLink =
  | { type: 'ticket'; ticketId: string; projectId: string | null }
  | { type: 'message'; messageId: string; conversationId: string | null; threadRootId: string | null }
  | { type: 'project'; projectId: string }
  | { type: 'doc'; docId: string; commentId: string | null }
export type InboxItem = {
  id: string
  kind: InboxKind
  state: 'UNREAD' | 'READ' | 'ARCHIVED'
  snoozedUntil: string | null
  /** When it was archived (only archived items; creation time for items archived before it was recorded). */
  archivedAt: string | null
  createdAt: string
  version: number
  actor: { id: string; name: string } | null
  subject: { kind: 'TICKET' | 'MESSAGE' | 'PROJECT' | 'FILE' | 'DOC'; id: string; title: string }
  params: Record<string, unknown>
  link: InboxLink
}
export type InboxPage = { items: InboxItem[]; nextCursor: string | null; unreadCount: number; unreadCapped: boolean }

/** One browser/device that receives Web Push for the signed-in person. The keys are never returned. */
export type PushSubscriptionInfo = {
  id: string
  endpoint: string
  label: string | null
  userAgent: string | null
  createdAt: string
  lastSuccessAt: string | null
  lastFailureAt: string | null
}
export type PushPublicKey = { configured: boolean; publicKey: string | null }

export type WatchTargetKind = 'TICKET' | 'PROJECT' | 'THREAD'
export type Subscription = { targetKind: WatchTargetKind; targetId: string; mode: 'AUTO' | 'EXPLICIT' | 'OPT_OUT' | 'NONE'; watching: boolean; version: number }

export type DndSettings = {
  enabled: boolean
  schedule: { days: number[]; start: string; end: string; timeZone: string } | null
  snoozeUntil: string | null
}
export type NotificationPreferences = { level: 'ALL' | 'DIRECT_ONLY'; dnd: DndSettings; external: Record<string, unknown>; version: number; dndActive: boolean }

// ---- Project permissions -------------------------------------------------------------------------------

/** What a person may do in a Project, from the least to the most. */
export type ProjectRole = Schema<'ProjectRole'>

export type PermissionMember = {
  userId: string
  displayName: string
  role: ProjectRole
  source: 'DIRECT'
  canWaiveDependencies: boolean
  canManagePins: boolean
  designatable: boolean
}
export type ProjectPermissions = {
  members: PermissionMember[]
  viaGroups: Array<{ groupId: string; name: string; role: string; memberCount: number }>
  /** The Teams linked to the Project; `role` is what the Project gives the Team's members (null: linked only). */
  teams: Array<{ teamId: string; name: string; role: ProjectRole | null; memberCount: number; archivedAt?: string | null }>
  capabilities: Array<'dependency_waiver' | 'pin_management'>
  recentChanges: Array<{
    at: string
    actorName: string | null
    userName: string | null
    userId: string
    before: { canWaiveDependencies: boolean; canManagePins: boolean }
    after: { canWaiveDependencies: boolean; canManagePins: boolean }
  }>
}

// ---- Docs ---------------------------------------------------------------------------------------

/**
 * Who a page is shared with: the whole Workspace, the people of a Project (narrowed to one of its
 * Teams when `scopeTeamId` is set), or only the people invited by name. Where it was made (`projectId`) is separate.
 */
export type DocVisibility = 'WORKSPACE' | 'PROJECT' | 'RESTRICTED'
/** A VIEWER reads the page and does not edit it. */
export type DocAccessRole = 'MANAGER' | 'EDITOR' | 'VIEWER'

/** DocContent v1 as the server stores it: the saved document. A draft is edited in the browser and replaces it on Save. */
export type DocContent = { schema: 'penfii.doc'; version: 1; blocks: unknown[] }

export type DocNode = {
  id: string
  parentId: string | null
  title: string
  position: number
  visibility: DocVisibility
  projectId: string | null
  updatedAt: string
  /** What the person may do with the page: `edit` also moves and renames it, `manage` deletes and shares it. */
  rights: { edit: boolean; manage: boolean }
}

export type DocPage = {
  id: string
  workspaceId: string
  projectId: string | null
  parentId: string | null
  title: string
  visibility: DocVisibility
  scopeProjectId: string | null
  scopeTeamId: string | null
  position: number
  /** The saved content. */
  content: DocContent
  /** The concurrency token Save compares. Unrelated to `latestVersionNo`. */
  revisionNo: number
  /** The number of the latest Version a person created; null while the page has none. */
  latestVersionNo: number | null
  /** The saved page differs from the latest Version (or there is none yet). */
  hasUnversionedChanges: boolean
  materializedAt: string
  updatedAt: string
  updatedBy: string | null
  createdBy: string
  liveEditing: boolean
  myRole: DocAccessRole | null
  rights: { edit: boolean; manage: boolean }
}

/** One entry of History: a Version someone chose to create. */
export type DocVersionSummary = {
  versionNo: number
  title: string
  sourceRevisionNo: number
  createdBy: string
  createdByName: string
  createdAt: string
  /** The saved page equals this Version right now. */
  isCurrent: boolean
}
export type DocVersion = DocVersionSummary & { content: DocContent }

export type DocParticipant = { userId: string; name: string; role: DocAccessRole }

export type DocComment = {
  id: string
  pageId: string
  authorId: string
  authorName: string
  body: RichTextDoc
  createdAt: string
  editedAt: string | null
  attachments?: MessageAttachment[]
}

export type DocTemplate = { id: string; name: string; createdBy: string }

/** `RECORD` is read only: a page written before Databases were removed may still hold one (it resolves as unavailable). */
export type ReferenceKind = 'TICKET' | 'DOC' | 'RECORD' | 'USER'
export type ResolvedReference = {
  kind: ReferenceKind
  id: string
  available: boolean
  title?: string
  context?: string
  /** Where a readable Ticket lives (opens its panel). */
  projectId?: string
}

export type TicketReference = {
  sourceKind: 'DOC'
  sourceId: string
  title: string
  context: { projectId?: string | null; parentTitlePath?: string[] }
  updatedAt: string
}

/** One working interval of a day, `HH:MM` in the calendar's zone (`24:00` ends at midnight). */
export type CalendarInterval = Schema<'CalendarInterval'>
export type CalendarHoliday = Schema<'CalendarHoliday'>
export type CalendarWeekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun'
export type CalendarWeek = Schema<'CalendarWeek'>
export type CalendarDefinition = { version: 1; week: CalendarWeek; holidays: CalendarHoliday[] }

export type ProjectCalendar = {
  projectId: string
  version: number
  timeZone: string
  definition: CalendarDefinition
  updatedAt: string
}

/** A draft to confirm. Nothing is stored until it is saved. */
export type CalendarSuggestion = { timeZone: string; definition: CalendarDefinition }

export type ProjectCalendarView = {
  /** Null until a calendar has been created on purpose. */
  calendar: ProjectCalendar | null
  suggestion: CalendarSuggestion | null
  canManage: boolean
}

/**
 * A sum in the Sprint's unit (a decimal string; with the unit NONE it counts Tasks). A Task without
 * an estimate adds nothing and is counted in `unestimated`, never as 0; one written in another unit
 * is counted in `otherUnit`.
 */
export type ReportAmount = Schema<'ReportAmount'>

/** Why a Sprint has no figures: it never started, or it started before baselines carried estimates. */
export type ReportUnavailable = 'NO_BASELINE' | 'LEGACY_BASELINE'
export type ReportAvailability = { available: boolean; reason: ReportUnavailable | null }

export type PlanAdherence = {
  /** Start estimates of the planned Tasks that were completed in the Sprint. */
  completedFromPlan: ReportAmount
  /** At most one decimal, for example "37.5"; null when the start plan is 0. */
  percent: string | null
  notApplicable: boolean
}

export type BurndownPoint = {
  at: string
  /** The day in the Sprint's zone that this point closes; null for the point at the start. */
  day: string | null
  remaining: ReportAmount
}

export type ScopeChangeKind = 'ADDED' | 'REMOVED' | 'ESTIMATE_CHANGED'

export type ScopeChange = {
  at: string
  kind: ScopeChangeKind
  ticket: TicketRef
  estimateFrom: string | null
  estimateTo: string | null
  otherUnit: boolean
}

export type ReportReason = 'COUNTED' | 'OPEN' | 'NOT_DONE' | 'REOPENED' | 'CANCELED' | 'REMOVED' | 'DONE_BEFORE'

export type ReportTicket = {
  ticket: TicketRef
  inBaseline: boolean
  addedAt: string | null
  startCategory: WorkflowState['category'] | null
  startEstimate: string | null
  endCategory: WorkflowState['category'] | null
  endEstimate: string | null
  otherUnit: boolean
  outcome: 'DONE' | 'NOT_DONE' | 'CANCELED' | 'REMOVED' | null
  /** Counted in the completed amount. */
  counted: boolean
  reason: ReportReason
}

/**
 * The figures of one Sprint for the person asking. Only Tickets they can see are in any sum, count
 * or list. While the Sprint is ACTIVE the end figures are as of `generatedAt`.
 */
export type SprintReport = {
  sprint: Sprint
  availability: ReportAvailability
  provisional: boolean
  generatedAt: string
  scope: 'VISIBLE_TICKETS'
  startPlan: ReportAmount | null
  completed: ReportAmount | null
  remaining: ReportAmount | null
  adherence: PlanAdherence | null
  burndown: BurndownPoint[]
  changes: ScopeChange[]
  tickets: ReportTicket[]
}

export type VelocityReason = ReportUnavailable | 'DIFFERENT_UNIT' | 'DIFFERENT_POLICY'

export type VelocityEntry = { sprint: Sprint; included: boolean; reason: VelocityReason | null; completed: ReportAmount | null }

export type VelocityReport = {
  /** The unit the included Sprints share; null when none qualifies. */
  unit: EstimateUnit | null
  /** The calculation policy version the included Sprints share. */
  policyVersion?: number | null
  generatedAt: string
  scope: 'VISIBLE_TICKETS'
  sprints: VelocityEntry[]
}

export type StatusDistribution = Schema<'StatusDistribution'>

export type ProjectStatusSummary = Schema<'ProjectStatusSummary'>

/** A Ticket of the Gantt list. `revisionNo` is what a schedule edit must quote. */
export type GanttItem = {
  ticket: TicketRef
  parentId: string | null
  channelId: string | null
  statusCategory: WorkflowState['category']
  assigneeName: string | null
  revisionNo: number
  /** Null means the Ticket is unplanned. `plannedEndAt` is derived from the work calendar. */
  schedule: TicketSchedule | null
}

/** A blocking dependency between two Tickets the viewer can see. */
export type GanttEdge = {
  id: string
  predecessorId: string
  successorId: string
  waived: boolean
  predecessorCategory: WorkflowState['category']
}

export type ProjectSchedule = {
  calendar: { version: number; timeZone: string } | null
  canEdit: boolean
  items: GanttItem[]
  edges: GanttEdge[]
  nextCursor: string | null
}

// ---- Workspace administration ----

export type WorkspaceMember = {
  userId: string
  displayName: string
  email: string
  role: 'OWNER' | 'ADMIN' | 'MEMBER'
  /** `INACTIVE` means removed from the Workspace. */
  state: 'ACTIVE' | 'INACTIVE'
  joinedAt: string
  deactivatedAt: string | null
  /** The account itself is disabled. */
  accountDisabled: boolean
  teams: Array<{ id: string; name: string }>
}

export type InvitationStatus = 'PENDING' | 'ACCEPTED' | 'EXPIRED' | 'REVOKED'

export type InvitationSummary = {
  id: string
  email: string
  role: string
  status: InvitationStatus
  expiresAt: string
  createdAt: string
  createdBy: string | null
  createdByName: string | null
  acceptedAt: string | null
  revokedAt: string | null
  teamId: string | null
  teamName: string | null
  /** The invited Team's state now; null when the invitation names no Team. */
  teamStatus: 'ACTIVE' | 'ARCHIVED' | 'GONE' | null
  /** The last e-mail delivery of the link; null when none was recorded. It never changes `status`. */
  emailStatus: InvitationEmailStatus | null
  emailAttemptedAt: string | null
  emailError: string | null
}

export type TeamRole = 'MANAGER' | 'MEMBER'
export type TeamMember = Schema<'TeamMember'>

export type AuditRecord = {
  id: string
  occurredAt: string
  actorUserId: string | null
  actorName: string | null
  action: string
  outcome: string
  targetKind: string | null
  targetId: string | null
  targetLabel: string | null
  subjectLabel: string | null
  data: Record<string, unknown>
}

export type AuditPage = { items: AuditRecord[]; nextCursor: string | null }

export type TrashedTicket = {
  id: string
  key: string
  title: string
  hierarchyKind: string
  trashedAt: string
  trashedBy: string | null
  trashedByName: string | null
  /** Also archived: a restore brings it back archived. */
  archived: boolean
  /** When the automatic purge takes it; null when the automatic purge is off. */
  purgeAt?: string | null
  /** While children or dependencies keep it from being purged, automatically or by hand. */
  purgeBlocked?: 'children' | 'dependencies' | null
}

export type ArchivedTicket = {
  id: string
  key: string
  title: string
  hierarchyKind: string
  archivedAt: string
  archivedBy: string | null
  archivedByName: string | null
  /** Also in the trash: restore it from there before unarchiving. */
  trashed: boolean
}

/** When the finished work of a completed Sprint is archived, if that was scheduled for later. */
export type SprintArchiveSchedule = Schema<'SprintArchiveSchedule'>

/** What to archive when a Sprint is completed. `AT` carries a future instant. */
export type SprintArchiveChoice = { mode: 'NONE' } | { mode: 'IMMEDIATE' } | { mode: 'AT'; at: string }

/** A Project that would be left without its last active direct admin, or without any admin, if the person left. */
export type ProjectImpact = Schema<'ProjectImpact'>
/** A private channel that would be left without an active manager if the person left. */
export type ChannelImpact = Schema<'ChannelImpact'>
export type OffboardingImpact = Schema<'OffboardingImpact'>
export type FormerAdmin = Schema<'FormerAdmin'>
/** A Project nobody who can act administers; a Workspace owner or admin may name an admin for it. */
export type ProjectWithoutAdmin = Schema<'ProjectWithoutAdmin'>
export type AdminRecovery = Schema<'AdminRecovery'>
export type FormerManager = Schema<'FormerManager'>
/** A private channel nobody who can act manages; a Workspace owner or admin may name a manager for it. */
export type ChannelWithoutManager = Schema<'ChannelWithoutManager'>
export type ManagerRecovery = Schema<'ManagerRecovery'>
/** A private channel of a Project as its admin sees it, readable or not: names and counts, never messages. */
export type ManagedChannel = Schema<'ManagedChannel'>
