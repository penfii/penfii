//! JSON contracts for the chat slice. No database types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub locale: Option<String>,
    pub time_zone: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Optional title shown on the profile card. Empty is stored as absent.
    #[serde(default)]
    pub job_title: Option<String>,
    /// Short status glyph. Absent, and cleared, once `status_clear_at` is in the past.
    #[serde(default)]
    pub status_emoji: Option<String>,
    #[serde(default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub status_clear_at: Option<DateTime<Utc>>,
    /// Whether this account has a profile photo. The bytes are a separate request.
    #[serde(default)]
    pub has_avatar: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub api_version: String,
    pub features: Vec<String>,
    pub limits: ServerLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerLimits {
    pub message_body_max_bytes: Option<i64>,
    pub file_max_bytes: Option<i64>,
    pub session_ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub time_zone: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_token: Option<String>,
    /// The Team the new member is meant to join, if any (resolved when the invitation is accepted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptInvitationResponse {
    pub user: User,
    pub workspace_id: Uuid,
    /// What became of the invitation's Team step; absent when the invitation named no Team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<InvitationTeamOutcome>,
}

/// The outcome of an invitation's Team step. `status` is `JOINED`, `ARCHIVED` or `GONE`; the last two
/// mean the person joined the Workspace only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationTeamOutcome {
    pub team_id: Option<Uuid>,
    pub team_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub archived_at: Option<DateTime<Utc>>,
    /// Optional plain text; empty is null.
    pub description: Option<String>,
    /// The department this team is shown under. Empty means the team is not placed. A department grants no access.
    pub department_id: Option<Uuid>,
}

/// A display-only branch of the organization tree. Membership and project access are unchanged by it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Department {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Fixed Ticket Key prefix, upper case, for example `PFI`.
    pub ticket_prefix: String,
    pub estimate_unit: String,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub team_ids: Vec<Uuid>,
    /// Whether the person asking takes part in the Project. Every Workspace member sees the Project in the
    /// list; only participants can open it.
    #[serde(default)]
    pub participating: bool,
    /// Whether the person asking manages the Project (is a Project admin): settings, workflow, trash, channels.
    #[serde(default)]
    pub can_manage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub category: String,
    pub position: i32,
    pub is_initial: bool,
    /// Warning-only limit on the Tickets a viewer sees in this State.
    #[serde(default)]
    pub wip_limit: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketLabel {
    pub id: Uuid,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub channel_id: Option<Uuid>,
    /// Display Key such as `PFI-001`. `id` stays the internal identifier.
    #[serde(alias = "publicKey")]
    pub key: String,
    pub title: String,
    pub description: serde_json::Value,
    pub hierarchy_kind: String,
    pub parent_id: Option<Uuid>,
    pub status_id: Uuid,
    pub assignee_id: Option<Uuid>,
    pub assignee_name: Option<String>,
    pub labels: Vec<TicketLabel>,
    pub priority: Option<String>,
    pub due_on: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub ticket_type: Option<TicketType>,
    #[serde(default)]
    pub parent: Option<TicketRef>,
    #[serde(default)]
    pub child_summary: ChildSummary,
    #[serde(default)]
    pub dependencies: DependencySummary,
    /// Current Sprint. A Subtask shows its parent Task's Sprint.
    #[serde(default)]
    pub sprint: Option<SprintRef>,
    #[serde(default)]
    pub milestones: Vec<MilestoneRef>,
    /// When the Ticket was last moved to the top of the shared order by hand, if ever.
    #[serde(default)]
    pub moved_to_top_at: Option<DateTime<Utc>>,
    /// Task estimate; absent means unestimated.
    #[serde(default)]
    pub estimate: Option<TicketEstimate>,
    /// Planned schedule; absent means no schedule.
    #[serde(default)]
    pub schedule: Option<TicketSchedule>,
    pub revision_no: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A Task estimate. `value` is a decimal string with at most two decimals (`"5"`, `"0.5"`) so a sum
/// never goes through a float. `unit` is the Project unit it was written in (`POINTS` or `HOURS`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketEstimate {
    pub value: String,
    pub unit: String,
}

/// A planned schedule in working minutes. The planned end is never stored: `planned_end_at` is
/// derived from the Project's current calendar and is set only when one Ticket is read or
/// changed (it is null in lists, when the Project has no calendar, or when the calendar's
/// horizon is exceeded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketSchedule {
    pub planned_start_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i64>,
    /// A schedule node of duration 0. It is not a Project Milestone.
    #[serde(default)]
    pub is_schedule_milestone: bool,
    #[serde(default)]
    pub planned_end_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketType {
    pub id: Uuid,
    pub name: String,
    pub color: Option<String>,
}

/// Minimal reference to a related Ticket (for example a parent) that a viewer may see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRef {
    pub id: Uuid,
    #[serde(alias = "publicKey")]
    pub key: String,
    pub title: String,
    pub hierarchy_kind: String,
}

/// Counts of the direct children the viewer can see. Hidden children are never counted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildSummary {
    pub total: i64,
    pub done: i64,
    pub canceled: i64,
}

/// Dependency counts the viewer can see. A blocker the viewer cannot see is never counted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencySummary {
    /// Predecessors that are not DONE yet.
    pub open_blockers: i64,
    pub blocked_by: i64,
    pub blocks: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintRef {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    /// True when a Subtask follows its parent Task's Sprint.
    pub inherited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneRef {
    pub id: Uuid,
    pub name: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMember {
    pub user_id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardColumn {
    pub id: Uuid,
    pub name: String,
    pub category: String,
    pub position: i32,
    pub state_id: Uuid,
    /// Workflow states shown in this column. A column may map to several states.
    pub state_ids: Vec<Uuid>,
    /// Tickets this viewer can see in the column, before any View filter. Warning-only WIP uses it.
    #[serde(default)]
    pub ticket_count: i64,
    #[serde(default)]
    pub wip_limit: Option<i32>,
    pub tickets: Vec<Ticket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBoard {
    pub project_id: Uuid,
    pub workflow_version: i64,
    /// The Project's estimate unit (`NONE`, `POINTS` or `HOURS`); Task estimates are written in it.
    #[serde(default)]
    pub estimate_unit: String,
    pub can_edit: bool,
    pub can_manage_workflow: bool,
    pub states: Vec<WorkflowState>,
    pub columns: Vec<BoardColumn>,
}

/// One changed field, expressed with values a person can read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    pub field: String,
    pub from: serde_json::Value,
    pub to: serde_json::Value,
}

/// A revision in the History list. The full snapshot stays on the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSummary {
    pub revision_no: i64,
    pub actor_user_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub changed_fields: Vec<String>,
    pub restore_source_revision_no: Option<i64>,
    /// Name of the MCP client that made this revision for the person, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPage {
    pub items: Vec<RevisionSummary>,
    pub next_before: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorableField {
    pub field: String,
    pub applicable: bool,
    /// Why the field cannot be restored now. Absent when applicable.
    pub reason: Option<String>,
}

/// One revision compared with the previous revision and with the current Ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionDetail {
    pub revision_no: i64,
    pub current_revision_no: i64,
    pub actor_user_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub restore_source_revision_no: Option<i64>,
    pub changes_from_previous: Vec<FieldChange>,
    pub changes_to_current: Vec<FieldChange>,
    pub restorable: Vec<RestorableField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketActivity {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub revision_no: Option<i64>,
    pub activity_kind: String,
    pub actor_user_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub data: serde_json::Value,
    /// Name of the MCP client that did this on the person's behalf, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub changes: Vec<FieldChange>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub items: Vec<TicketActivity>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketComment {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub author_id: Uuid,
    pub author_name: Option<String>,
    pub body: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attachments: Vec<TicketAttachment>,
    /// What the requester may do with it: only its author edits; the author, a Project admin and a
    /// MANAGER of the Ticket's channel delete. Neither while the Ticket is archived or trashed.
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default)]
    pub can_delete: bool,
}

/// A file attached to a Ticket's body (`commentId` absent) or to one of its comments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketAttachment {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub comment_id: Option<Uuid>,
    pub file_id: Uuid,
    pub filename: String,
    pub byte_size: i64,
    pub mime: Option<String>,
    pub state: String,
    pub attached_by: Uuid,
    pub attached_at: DateTime<Utc>,
}

/// `PATCH /v1/channels/{id}/members/{user_id}`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetChannelMemberRoleRequest {
    pub role: String,
}

/// One person of a channel. `role` is `MANAGER` or `MEMBER`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMember {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub topic: Option<String>,
    pub visibility: String,
    pub team_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub allow_member_invites: bool,
    pub archived_at: Option<DateTime<Utc>>,
    pub role: Option<String>,
    /// Root messages the caller has not read. Zero when the caller is not a member.
    #[serde(default)]
    pub unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectMessage {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub user_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub last_message_seq: i64,
    pub last_change_seq: i64,
    /// Other members, for the sidebar. Empty on a create response; the space query fills it.
    #[serde(default)]
    pub peers: Vec<ConversationPeer>,
    #[serde(default)]
    pub last_message_preview: Option<String>,
    #[serde(default)]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unread_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPeer {
    pub id: Uuid,
    pub display_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub teams: Vec<Team>,
    pub projects: Vec<Project>,
    pub channels: Vec<Channel>,
    pub dms: Vec<DirectMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadState {
    pub generation: i64,
    pub read_through_seq: i64,
    pub manual_unread_from_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: String,
    pub last_message_seq: i64,
    pub last_change_seq: i64,
    pub replay_floor_seq: i64,
    pub created_at: DateTime<Utc>,
    pub channel: Option<Channel>,
    pub member_ids: Vec<Uuid>,
    pub read: ReadState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub emoji: String,
    pub user_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub stream_seq: i64,
    pub author_id: Uuid,
    #[serde(default)]
    pub author_display_name: String,
    pub thread_root_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    pub version: i64,
    pub body_schema_version: i16,
    pub body: Option<Value>,
    pub deleted: bool,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    /// Replies whose thread root is this message. Zero on replies themselves.
    #[serde(default)]
    pub reply_count: i64,
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachment {
    pub id: Uuid,
    pub file_id: Uuid,
    pub filename: String,
    pub byte_size: i64,
    pub mime: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileObject {
    pub id: Uuid,
    pub generation: i64,
    pub filename: String,
    pub byte_size: i64,
    pub mime: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageVersion {
    pub version: i64,
    pub body_schema_version: i16,
    pub body: Value,
    pub changed_by: Uuid,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageHistory {
    pub message_id: Uuid,
    pub versions: Vec<MessageVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionState {
    pub message_id: Uuid,
    pub emoji: String,
    pub user_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationChange {
    pub conversation_id: Uuid,
    pub change_seq: i64,
    pub message_id: Uuid,
    pub message_version: Option<i64>,
    pub change_kind: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftPayload {
    pub schema_version: i16,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpectedDraft {
    pub generation: Uuid,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: Uuid,
    pub context_kind: String,
    pub context_id: Uuid,
    pub generation: Uuid,
    pub version: i64,
    pub consumed_version: i64,
    pub state: String,
    pub payload: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub time_zone: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvitationRequest {
    pub email: String,
    pub role: String,
    /// Optional Team for the new member (always as `MEMBER`).
    #[serde(default)]
    pub team_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptInvitationRequest {
    pub token: String,
    pub display_name: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    /// Optional plain text, like the description edited later; empty is none.
    #[serde(default)]
    pub description: Option<String>,
    /// The Team's first members, each with an explicit role; at least one. The creator is not added unless listed.
    #[serde(default)]
    pub members: Vec<TeamMemberInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberInput {
    pub user_id: Uuid,
    pub role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
    pub team_ids: Vec<Uuid>,
    /// Chosen Ticket Prefix; omitted means use the suggestion for the name.
    #[serde(default)]
    pub ticket_prefix: Option<String>,
}

/// Answer to "what Prefix fits this name, and is the typed one usable".
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefixCheck {
    pub suggestion: Option<String>,
    pub prefix: Option<PrefixStatus>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefixStatus {
    pub value: String,
    pub valid: bool,
    pub available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChannelRequest {
    pub name: String,
    pub description: Option<String>,
    pub visibility: String,
    pub team_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelRequest {
    pub visibility: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDmRequest {
    pub user_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub body_schema_version: i16,
    pub body: Value,
    pub thread_root_id: Option<Uuid>,
    pub reply_to_id: Option<Uuid>,
    #[serde(default)]
    pub file_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateFileRequest {
    pub filename: String,
    pub byte_size: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditMessageRequest {
    pub expected_version: i64,
    pub body: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionRequest {
    pub emoji: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReadRequest {
    pub generation: i64,
    pub read_through_seq: i64,
    pub scope: String,
    pub observed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualUnreadRequest {
    pub from_seq: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftRequest {
    pub context_kind: String,
    pub context_id: Uuid,
    pub generation: Option<Uuid>,
    pub expected_version: Option<i64>,
    pub payload: DraftPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeqLimitQuery {
    pub before_seq: Option<i64>,
    pub after_seq: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftQuery {
    pub context_kind: String,
    pub context_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub types: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// Result tab: all, message, ticket, file, space, view or person. Absent means the directory
    /// lookup that the mention picker and organization pages use.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    /// Comma separated State ids chosen with the State chip.
    #[serde(default)]
    pub state_ids: Option<String>,
    /// IANA time zone for `before:` and `after:`.
    #[serde(default)]
    pub tz: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySearch {
    pub query: String,
    pub results: Vec<DirectoryHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryHit {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    pub subtitle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person: Option<DirectoryPerson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<DirectoryOrganization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<DirectoryMessageHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryPerson {
    pub id: Uuid,
    pub display_name: String,
    pub username: Option<String>,
    pub email: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub affiliation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryOrganization {
    pub id: Uuid,
    pub name: String,
    pub time_zone: String,
    pub member_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryMessageHit {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub author_id: Uuid,
    pub author_name: String,
    pub snippet: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationDirectory {
    pub organization: DirectoryOrganization,
    pub members: Vec<DirectoryPerson>,
}

/// One dependency edge: the successor waits for the predecessor to be DONE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyEdge {
    pub id: Uuid,
    pub ticket: TicketRef,
    pub state_name: String,
    pub state_category: String,
    /// Whether this edge currently lets the successor complete (predecessor DONE, or waived).
    pub satisfied: bool,
    pub created_at: DateTime<Utc>,
    /// `BLOCKS` is a forced dependency; `RELATED` is a plain link that never blocks.
    #[serde(default = "blocks_kind")]
    pub kind: String,
    /// A waived dependency stays in place but no longer blocks.
    #[serde(default)]
    pub waived: bool,
    #[serde(default)]
    pub waive_reason: Option<String>,
    /// The predecessor was canceled, so this edge can never be satisfied by completion.
    #[serde(default)]
    pub predecessor_canceled: bool,
    /// The relation no longer fits the Tickets' visibility and needs a person to look at it.
    #[serde(default)]
    pub needs_review: bool,
}

fn blocks_kind() -> String {
    "BLOCKS".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketDependencies {
    pub ticket_id: Uuid,
    /// Predecessors this Ticket waits for.
    pub blocked_by: Vec<DependencyEdge>,
    /// Successors that wait for this Ticket.
    pub blocks: Vec<DependencyEdge>,
    /// Plain RELATED links, in both directions.
    #[serde(default)]
    pub related: Vec<DependencyEdge>,
    pub can_edit: bool,
    /// Whether the viewer may waive or restore a dependency of this Ticket.
    #[serde(default)]
    pub can_waive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: Uuid,
    /// The Project of a Project View; null for a Team or Workspace View.
    pub project_id: Option<Uuid>,
    #[serde(default = "project_target")]
    pub target_kind: String,
    #[serde(default)]
    pub team_id: Option<Uuid>,
    pub name: String,
    pub layout: String,
    pub scope: String,
    pub owner_id: Uuid,
    pub definition: serde_json::Value,
    pub version: i64,
    pub can_edit: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sprint {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub goal: Option<String>,
    pub state: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: chrono::NaiveDate,
    pub version: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// IANA zone the Sprint's days are counted in. Fixed when the Sprint starts.
    #[serde(default)]
    pub time_zone: String,
    /// `NONE`, `POINTS` or `HOURS`; copied from the Project when the Sprint is created and fixed
    /// when it starts.
    #[serde(default)]
    pub estimate_unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub target_on: Option<chrono::NaiveDate>,
    pub state: String,
    /// Set when the state becomes REACHED, cleared when it leaves REACHED. Null for a Milestone
    /// that was already reached before this was recorded.
    #[serde(default)]
    pub reached_at: Option<chrono::DateTime<chrono::Utc>>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPlanning {
    pub can_manage: bool,
    pub sprints: Vec<Sprint>,
    pub milestones: Vec<Milestone>,
}

fn project_target() -> String {
    "PROJECT".into()
}

/// What closing a Sprint would do, shown before the person confirms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintClosePreview {
    pub sprint_id: Uuid,
    pub sprint_version: i64,
    pub done: Vec<TicketRef>,
    pub canceled: Vec<TicketRef>,
    /// Unfinished Tasks the viewer can see. Tasks the viewer cannot see are not listed or counted.
    pub unfinished: Vec<TicketRef>,
    /// PLANNED or ACTIVE Sprints of the Project that can take the unfinished Tasks.
    pub targets: Vec<SprintTarget>,
    /// The Sprint chosen when the person makes no choice: the next Sprint, or none (backlog).
    pub default_target_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintTarget {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub starts_on: chrono::NaiveDate,
}

/// One Task's recorded participation in a Sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintResult {
    pub ticket: TicketRef,
    /// DONE, CANCELED, NOT_DONE, REMOVED, or null while the Sprint is still open and for old records.
    pub outcome: Option<String>,
    pub carried_to: Option<SprintTarget>,
    pub carried_from: Option<SprintTarget>,
    pub needs_next_assignment: bool,
    /// True when the Task was in the baseline recorded at start.
    pub in_baseline: bool,
    pub assigned_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintResults {
    pub sprint: Sprint,
    pub baseline_count: Option<i64>,
    pub baseline_recorded_at: Option<DateTime<Utc>>,
    pub results: Vec<SprintResult>,
}

/// A Ticket in a Team or Workspace scope with the places it lives in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedTicket {
    #[serde(flatten)]
    pub ticket: Ticket,
    pub project_name: String,
    pub channel_name: Option<String>,
    pub state_name: String,
    pub state_category: String,
    /// Whether the viewer follows this Ticket (Watch). Drives the 관찰 lens of My Work.
    #[serde(default)]
    pub watching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeProject {
    pub id: Uuid,
    pub name: String,
    pub can_edit: bool,
}

/// The Ticket dataset a Team or Workspace scope shows. The same Tickets a Project Board shows,
/// gathered across Projects the viewer can read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDataset {
    pub projects: Vec<ScopeProject>,
    pub tickets: Vec<ScopedTicket>,
    /// True when the scope holds more Tickets than were returned.
    pub truncated: bool,
}

/// The result of one Ticket in a bulk change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkItemResult {
    pub ticket_id: Uuid,
    pub key: Option<String>,
    pub ok: bool,
    /// Machine-readable reason when `ok` is false (for example `dependency_blocked`).
    pub code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkResult {
    pub total: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub items: Vec<BulkItemResult>,
}

/// What a personal shortcut points at, as far as the viewer may see it. A target the viewer can no
/// longer open has `available = false` and no title, so nothing about it leaks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRef {
    pub kind: String,
    pub target_id: Uuid,
    pub available: bool,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub project_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    /// For a View: PROJECT, TEAM or WORKSPACE.
    pub view_target: Option<String>,
    pub layout: Option<String>,
    /// For a Message or File: where to open it.
    #[serde(default)]
    pub conversation_id: Option<Uuid>,
    #[serde(default)]
    pub thread_root_id: Option<Uuid>,
    #[serde(default)]
    pub message_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteItem {
    #[serde(flatten)]
    pub target: SavedRef,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    /// The folder it sits in; absent at the top level.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaterItem {
    pub id: Uuid,
    #[serde(flatten)]
    pub target: SavedRef,
    pub note: Option<String>,
    pub state: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The time this item will come back, if the person set one.
    #[serde(default)]
    pub reminder: Option<ReminderInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderInfo {
    pub id: Uuid,
    pub due_at: DateTime<Utc>,
    pub time_zone: String,
    pub state: String,
    pub version: i64,
    pub fired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinItem {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub ticket: Option<TicketRef>,
    pub position: i64,
    pub pinned_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPins {
    pub can_manage: bool,
    pub pins: Vec<PinItem>,
}

/// One unsent message of the caller, for the Drafts list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftItem {
    pub id: Uuid,
    pub context_kind: String,
    pub context_id: Uuid,
    pub conversation_id: Uuid,
    pub thread_root_id: Option<Uuid>,
    /// Channel name, or null for a direct message.
    pub title: Option<String>,
    pub project_id: Option<Uuid>,
    pub preview: String,
    pub updated_at: DateTime<Utc>,
}

/// One working interval of a day, `HH:MM` in the calendar's time zone (`24:00` ends at midnight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarInterval {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarHoliday {
    pub date: chrono::NaiveDate,
    pub name: String,
}

/// Working intervals per weekday. An empty list is a day off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarWeek {
    #[serde(default)]
    pub mon: Vec<CalendarInterval>,
    #[serde(default)]
    pub tue: Vec<CalendarInterval>,
    #[serde(default)]
    pub wed: Vec<CalendarInterval>,
    #[serde(default)]
    pub thu: Vec<CalendarInterval>,
    #[serde(default)]
    pub fri: Vec<CalendarInterval>,
    #[serde(default)]
    pub sat: Vec<CalendarInterval>,
    #[serde(default)]
    pub sun: Vec<CalendarInterval>,
}

/// Calendar definition, version 1: weekly working intervals and whole-day holidays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarDefinition {
    pub version: i32,
    pub week: CalendarWeek,
    #[serde(default)]
    pub holidays: Vec<CalendarHoliday>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCalendar {
    pub project_id: Uuid,
    pub version: i64,
    pub time_zone: String,
    pub definition: CalendarDefinition,
    pub updated_at: DateTime<Utc>,
}

/// A draft the person can confirm. It is a proposal only; nothing is stored until it is saved.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSuggestion {
    pub time_zone: String,
    pub definition: CalendarDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCalendarView {
    /// Null until a calendar has been created on purpose.
    pub calendar: Option<ProjectCalendar>,
    /// Present while there is no calendar.
    pub suggestion: Option<CalendarSuggestion>,
    pub can_manage: bool,
}

// ---------------------------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------------------------

/// A sum in the Sprint's unit with what it leaves out. `value` is a decimal string; with the unit
/// `NONE` it counts Tasks. A Task without an estimate adds nothing and is counted in
/// `unestimated` (never as 0); one written in another unit is counted in `other_unit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportAmount {
    pub value: String,
    pub tasks: i64,
    pub unestimated: i64,
    pub other_unit: i64,
}

/// Why a Sprint has no figures: it never started (`NO_BASELINE`), or it started before baselines
/// carried estimates and categories (`LEGACY_BASELINE`). Nothing is rebuilt from today's data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportAvailability {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAdherence {
    /// Start estimates of the planned Tasks that were completed in the Sprint.
    pub completed_from_plan: ReportAmount,
    /// Percent with at most one decimal (`"37.5"`); null when the plan is 0.
    pub percent: Option<String>,
    /// True when the start plan is 0: there is nothing to measure.
    pub not_applicable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BurndownPoint {
    pub at: DateTime<Utc>,
    /// The Sprint-zone day this point closes; null for the point at the start.
    pub day: Option<chrono::NaiveDate>,
    pub remaining: ReportAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeChange {
    pub at: DateTime<Utc>,
    /// `ADDED`, `REMOVED` or `ESTIMATE_CHANGED`.
    pub kind: String,
    pub ticket: TicketRef,
    pub estimate_from: Option<String>,
    pub estimate_to: Option<String>,
    /// An estimate shown here was written in another unit than the Sprint's.
    pub other_unit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTicket {
    pub ticket: TicketRef,
    pub in_baseline: bool,
    pub added_at: Option<DateTime<Utc>>,
    pub start_category: Option<String>,
    pub start_estimate: Option<String>,
    pub end_category: Option<String>,
    pub end_estimate: Option<String>,
    pub other_unit: bool,
    /// `DONE`, `NOT_DONE`, `CANCELED` or `REMOVED`; null while the assignment is open.
    pub outcome: Option<String>,
    /// Counted in the completed amount.
    pub counted: bool,
    /// `COUNTED`, `OPEN`, `NOT_DONE`, `REOPENED`, `CANCELED`, `REMOVED` or `DONE_BEFORE`.
    pub reason: String,
}

/// The figures of one Sprint for the person asking: only Tickets they can see, from the baseline,
/// the assignment history, the Ticket revisions and the State category history. Nothing is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SprintReport {
    pub sprint: Sprint,
    pub availability: ReportAvailability,
    /// True while the Sprint is ACTIVE: the end figures are as of `generated_at`.
    pub provisional: bool,
    pub generated_at: DateTime<Utc>,
    /// Always `VISIBLE_TICKETS`: the figures cover the Tickets the viewer can see.
    pub scope: String,
    pub start_plan: Option<ReportAmount>,
    pub completed: Option<ReportAmount>,
    pub remaining: Option<ReportAmount>,
    pub adherence: Option<PlanAdherence>,
    pub burndown: Vec<BurndownPoint>,
    pub changes: Vec<ScopeChange>,
    pub tickets: Vec<ReportTicket>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VelocityEntry {
    pub sprint: Sprint,
    pub included: bool,
    /// Why a Sprint is listed apart: `NO_BASELINE`, `LEGACY_BASELINE`, `DIFFERENT_UNIT` or
    /// `DIFFERENT_POLICY` (another calculation policy version).
    pub reason: Option<String>,
    pub completed: Option<ReportAmount>,
}

/// The completed amount of the last completed Sprints of a Project that share one unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VelocityReport {
    /// The unit the included Sprints share; `NONE` counts Tasks. Null when no Sprint qualifies.
    pub unit: Option<String>,
    /// The calculation policy version the included Sprints share.
    pub policy_version: Option<i16>,
    pub generated_at: DateTime<Utc>,
    pub scope: String,
    pub sprints: Vec<VelocityEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDistribution {
    pub not_started: i64,
    pub in_progress: i64,
    pub done: i64,
    pub canceled: i64,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStatusSummary {
    pub generated_at: DateTime<Utc>,
    pub scope: String,
    pub today: chrono::NaiveDate,
    pub due_within_days: i32,
    pub distribution: StatusDistribution,
    /// Open Tickets with a blocking predecessor that is not done and not waived.
    pub blocked: i64,
    /// Open Tickets due within `due_within_days` days from `today`, overdue ones included.
    pub due_soon: i64,
}

// ---------------------------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------------------------

/// One Ticket of the Gantt list with its planned schedule (the end derived from the calendar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GanttItem {
    pub ticket: TicketRef,
    pub parent_id: Option<Uuid>,
    pub channel_id: Option<Uuid>,
    pub status_category: String,
    pub assignee_name: Option<String>,
    pub revision_no: i64,
    pub schedule: Option<TicketSchedule>,
}

/// A blocking dependency between two Tickets the viewer can see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GanttEdge {
    pub id: Uuid,
    pub predecessor_id: Uuid,
    pub successor_id: Uuid,
    pub waived: bool,
    /// The predecessor's State category; a canceled one blocks until the relation is removed or waived.
    pub predecessor_category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GanttCalendar {
    pub version: i64,
    pub time_zone: String,
}

/// The Tickets of a Project with their schedules, for the Gantt. Tickets and edges the viewer
/// cannot see are not in it. Paged by Ticket number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSchedule {
    pub calendar: Option<GanttCalendar>,
    pub can_edit: bool,
    pub items: Vec<GanttItem>,
    pub edges: Vec<GanttEdge>,
    pub next_cursor: Option<String>,
}

// ---- Workspace administration -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberTeam {
    pub id: Uuid,
    pub name: String,
}

/// A workspace member as every other member may see them: name, email, teams, title and status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePerson {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub role: String,
    pub state: String,
    pub job_title: Option<String>,
    pub time_zone: Option<String>,
    pub status_emoji: Option<String>,
    pub status_text: Option<String>,
    pub status_clear_at: Option<DateTime<Utc>>,
    pub has_avatar: bool,
    pub teams: Vec<MemberTeam>,
}

/// One row of the member list an owner or admin manages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMember {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub role: String,
    /// `ACTIVE` or `INACTIVE` (removed from the Workspace; sign-in to the account may still work).
    pub state: String,
    pub joined_at: DateTime<Utc>,
    pub deactivated_at: Option<DateTime<Utc>>,
    /// The account itself is disabled (nobody can sign in with it).
    pub account_disabled: bool,
    pub teams: Vec<MemberTeam>,
}

/// An invitation as the managers see it. `status` is `PENDING`, `ACCEPTED`, `EXPIRED` or `REVOKED`;
/// the link token is never listed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationSummary {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
    pub created_by_name: Option<String>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// The Team named by the invitation, with its state now: `ACTIVE`, `ARCHIVED` or `GONE`.
    pub team_id: Option<Uuid>,
    pub team_name: Option<String>,
    pub team_status: Option<String>,
}

/// One Project linked to a Team, as the person asking sees it: the Team's participation role and the
/// person's own access (never another member's).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamProject {
    pub project_id: Uuid,
    pub name: String,
    pub ticket_prefix: String,
    pub archived_at: Option<DateTime<Utc>>,
    /// The Team's participation role (`ADMIN`, `EDITOR`, `VIEWER`); null when only linked.
    pub team_role: Option<String>,
    /// Whether the person asking takes part in the Project. When false the next two are empty.
    pub can_open: bool,
    pub my_role: Option<String>,
    /// Where the person's own access comes from: `DIRECT`, `GROUP`, `TEAM`.
    pub my_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor_user_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub action: String,
    pub outcome: String,
    pub target_kind: Option<String>,
    pub target_id: Option<Uuid>,
    /// A readable name for the target (person, Team, Project, Ticket key or invitation e-mail).
    pub target_label: Option<String>,
    /// The person the entry is about when the target is something else (a Team member, a designation).
    pub subject_label: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPage {
    pub items: Vec<AuditRecord>,
    pub next_cursor: Option<String>,
}

/// A Ticket in a Project's trash, kept until a Workspace owner or admin purges it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedTicket {
    pub id: Uuid,
    pub key: String,
    pub title: String,
    pub hierarchy_kind: String,
    pub trashed_at: DateTime<Utc>,
    pub trashed_by: Option<Uuid>,
    pub trashed_by_name: Option<String>,
}
