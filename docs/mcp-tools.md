# MCP tools

What an AI client can do in Penfii through the [MCP gateway](self-hosting.md#the-mcp-gateway-optional).
Every tool acts as the person who approved the client, runs the same `core` code as the web app, and
is checked three times on every call: the OAuth scope the person granted, the Workspace MCP policy,
and the person's own permission on the resource. Listing a tool is not permission to run it (see
[Discovery and permission](#discovery-and-permission)).
The table is generated from the code (`cargo test -p penfii-mcp print_tool_table -- --ignored
--nocapture`) and a test fails when it is out of date.

## Scopes

A scope is `<area>:<intent>`. `manage` includes `write`, `write` includes `read`; `delete` includes
nothing. Approving the default request grants reading only; every scope that changes data is ticked by
the person on the consent screen, and the high-risk ones (`project:manage`, `doc:delete`) also need a
recent sign-in at that moment. A new scope needs a new consent; the old grant is replaced by it.

### Discovery and permission

`tools/list` shows every tool the Workspace policy allows in at least one Workspace the person can
reach, **whatever scopes the connection has**: a client can only ask for a permission it knows a tool
needs. Each tool says what it needs in `securitySchemes` (at the top of its descriptor and in
`_meta`). What the policy blocks (by default the destructive and administrative tools) is not listed.

Calling a tool the connection lacks a scope for does nothing and answers `403` with
`insufficient_scope` in `WWW-Authenticate` and in the result's `_meta["mcp/www_authenticate"]`:
`scope` names everything the call is missing, then what the connection has, so one new consent is
enough, and `error_description` says what is missing. Scope, policy, Workspace membership and the
person's own permission are checked on every call, listed or not.

| Scope | Covers |
|---|---|
| `workspace:read` / `workspace:write` | Workspaces, Teams and Departments (read); creating a Workspace, changing its name and time zone |
| `member:read` / `member:write` | People and invitations (read); inviting people **as MEMBER** |
| `team:write` | Teams, departments and their members |
| `search:read` | Search (each kind also needs the scope of its content: tickets, docs, conversations, files, views, people) |
| `ticket:read` / `ticket:write` | Tickets, comments, revisions, trash and archive |
| `project:read` / `project:write` / `project:manage` | Project data; settings, workflow states, labels, pins, calendar; Project roles and Team links, deleting workflow states |
| `doc:read` / `doc:write` / `doc:delete` | Docs, comments, versions, templates; deleting Docs |
| `conversation:read` / `conversation:write` | Channels, DMs, messages, threads, reactions, read state |
| `file:read` / `file:write` | File details and small downloads; small uploads, attachments |
| `plan:read` / `plan:write` | Boards, views, schedules, sprints, milestones, dependencies, reports |
| `personal:read` / `personal:write` | Inbox, Later, favorites, drafts, subscriptions, notification settings, own profile |
| `admin:read` | Audit log, webhook list, trash/archive/recovery lists |

## Risk classes and the Workspace policy

The risk class describes what an action does, not who may run it. **Read** changes nothing (reading a
channel or the Inbox never marks anything read). **Write** is ordinary collaboration and routine
management, even when only an admin may do it (creating Projects, Teams, departments, MEMBER
invitations, sprints). **Administrative** is sensitive organisation or security administration (Project
roles, Team links, making a private channel public). **Destructive** cannot be undone (deleting a
Doc, a Ticket or Doc comment, a workflow state, an attachment, an upload). The standard Workspace policy allows Read and Write
and turns the other two off; a Workspace owner or admin changes this in the web app (it needs a recent
sign-in). `workspace_mcp_policy` shows the current setting.

## What MCP never does

These stay in the web app; a client's tool approval, an OAuth consent or a `*:manage` scope does not
replace the recent sign-in the web app asks for. `workspace_web_only` lists them with the place in the
web app.

- Account security: password, e-mail, MFA and recovery codes, linked sign-ins, sessions, account recovery, OAuth approval, revoking all MCP access.
- Browser or device registration for Web Push.
- Making someone OWNER or ADMIN, taking those roles away, inviting with those roles (also guarded by a recent sign-in in the web app).
- Removing a member by force, disabling or enabling an account.
- Recovering a Project, Doc or Channel without a manager.
- Deleting a Ticket permanently (trash and restore work).
- Creating, rotating or revoking a Channel webhook.
- Changing the Workspace MCP policy.

Invitation tools never return the invitation link or token. They return the invitation id, e-mail,
role, expiry and a mail status (`SENT`, `FAILED`, `NOT_CONFIGURED`); when the mail was not sent a
person with the right permission copies the link from the web app.

## What a risk class rests on

The class follows what the action does to data, which Core decides, and not who may run it.
Deleting a Ticket or Doc comment is **Destructive**: the Ticket comment's text is overwritten and its
files are removed, the Doc comment's files are removed, and the product keeps no copy to restore.
Deleting a message is **Write**: the earlier text stays in the message history and a repeat is
harmless. Changing who can read a Doc (`doc_set_visibility`, `doc_set_participants`), Team and channel
members and creating Projects are **Write**, as in the web app, where Core's manage rights and
self-promotion rules apply unchanged. Because showing a page to more people cannot be taken back, the
answer of `doc_set_visibility` and `doc_set_participants` has a `change` object (what was added,
removed or widened). Making a private channel public is **Administrative**. A tool that returns a list
returns it as `{"items": [...]}`, since MCP structured results are objects.

## Files

`file_create_small` uploads at most 256 KiB, sent as base64 in the call; `file_download` and
`attachment_download` return at most 256 KiB. Bigger files, and clients that cannot pass file bytes
through a tool call, use the web app. A short-lived upload URL for large files would need a new
authenticated endpoint and is not built. Whether ChatGPT, Claude Code and Codex can pass or show file
bytes has not been verified against the real clients.

## Conversation changes

There is no WebSocket in MCP. `conversation_changes_list` returns what changed after a cursor; poll it.

## Who did it

A tool acts as the person, so what it changes is theirs, as in the web app. Every call is also logged
(`mcp_call_log`: person, client, grant, Workspace, tool, outcome; never a payload), and everything the
call writes (Ticket revisions, Activity, domain events, audit entries) records that call as its command.
So a Ticket's History and Activity say *via &lt;client&gt;* for what a client wrote, for one Ticket or
for each Ticket of `ticket_bulk_update`, and the audit log names the client next to the person (the
entry's data has `via: "mcp"`, `grantId`, `clientPk` and `mcpCallId`). What the web app does names no
client. A failed change writes nothing and a replayed call (the same `idempotencyKey`) writes nothing
again, so neither is credited to a call. No token or secret is stored with any of it.

## Retries

A write that takes `idempotencyKey` returns the first result when the same key comes with the same
request, refuses the same key with a different request (`idempotency_conflict`), and without a key may
write twice. Keys belong to the person. Channel names are not unique: `channel_create` with a name that
is already used makes another channel, so a client retries it with the same key, not by its name.

## Tools

Tool names are `<area>_<action>`. Ids come from the tool named in the description (for example
`workspace_overview` for ids of Projects and Channels, `member_search` for people).
### admin
| Tool | Scope | Risk | What it does |
| `archive_list` | `admin:read` | read | Archived Tickets of a Project the person can read |
| `audit_list` | `admin:read` | read | Audit entries of a Workspace, newest first, paged (limit up to 50, cursor), filtered by `action` |
| `recovery_channels_without_manager` | `admin:read` | read | Channels with no active manager |
| `recovery_docs_without_manager` | `admin:read` | read | Docs with no active manager |
| `recovery_projects_without_admin` | `admin:read` | read | Projects of a Workspace with no active admin (owner/admin per Core) |
| `trash_list` | `admin:read` | read | Trashed Tickets of a Project (key, title, deleted time and who deleted them) for a Project admin |
| `webhook_list` | `admin:read` | read | Webhooks of a channel: id, name, auth mode, acting user, revoked flag and dates |
### conversation
| Tool | Scope | Risk | What it does |
| `channel_create` | `conversation:write` | write | Create a channel (PUBLIC or PRIVATE), optionally linked to a Project the person may use (and narrowed to one of its Teams) |
| `channel_join` | `conversation:write` | write | Join a PUBLIC channel as the connected person |
| `channel_leave` | `conversation:write` | write | Leave a channel as the connected person (userId is that person; the channel keeps its history) |
| `channel_member_add` | `conversation:write` | write | Add a person to a channel when the channel's rules let the connected person do so (managers, or members when allowMemberInvites) |
| `channel_member_remove` | `conversation:write` | write | Remove another member from a channel (channel managers only) |
| `channel_member_set_role` | `conversation:write` | write | Change a channel member's channel role (channel managers only) |
| `channel_members_list` | `conversation:read` | read | List the members of a channel with their channel role |
| `channel_set_visibility` | `conversation:write` | administrative | Change a channel between PUBLIC and PRIVATE (channel managers only) |
| `channel_update` | `conversation:write` | write | Change a channel's description and whether members may invite others (channel managers only) |
| `conversation_changes_list` | `conversation:read` | read | Poll what changed in a conversation (new, edited, deleted messages, reactions) after a changeSeq cursor |
| `conversation_get` | `conversation:read` | read | Read one conversation (channel or DM): kind, last message/change sequence, the channel's settings, DM participants and this person's read state |
| `conversation_mark_read` | `conversation:write` | write | Move the connected person's read position forward |
| `conversation_mark_unread` | `conversation:write` | write | Mark messages from fromSeq on as unread for the connected person (the web app's "mark as unread") |
| `conversation_read_state` | `conversation:read` | read | This person's read position in a conversation (generation, readThroughSeq, manualUnreadFromSeq) |
| `dm_create` | `conversation:write` | write | Open (or return) the direct message between exactly the people in userIds, who must include the connected person |
| `message_delete` | `conversation:write` | write | Delete a message (the author, or a channel manager) |
| `message_edit` | `conversation:write` | write | Replace the body of the connected person's own message |
| `message_history` | `conversation:read` | read | Read the earlier versions of one of the connected person's own messages (or of any message when they audit the channel) |
| `message_list` | `conversation:read` | read | List a conversation's top-level messages, newest first (thread replies are in thread_messages_list) |
| `message_react` | `conversation:write` | write | Add an emoji reaction as the connected person |
| `message_send` | `conversation:write` | write | Post a message in a channel or DM as the connected person (not as a bot); the MCP client is recorded in the audit trail |
| `message_unreact` | `conversation:write` | write | Remove the connected person's own emoji reaction |
| `thread_mark_read` | `conversation:write` | write | Move the connected person's read position in one thread forward |
| `thread_mark_unread` | `conversation:write` | write | Mark a thread from fromSeq on as unread for the connected person |
| `thread_messages_list` | `conversation:read` | read | List a thread: the root message and its replies, oldest first |
| `thread_read_state` | `conversation:read` | read | This person's read position in one thread (generation, readThroughSeq) |
### doc
| Tool | Scope | Risk | What it does |
| `doc_comment_create` | `doc:write` | write | Add a comment (text or rich-text JSON) to a Doc the person can read |
| `doc_comment_delete` | `doc:write` | destructive | Delete a comment (the person's own, or any as the page's manager, as the web app allows) |
| `doc_comment_edit` | `doc:write` | write | Replace the body of a comment the person wrote (comment id from doc_comment_list) |
| `doc_comment_list` | `doc:read` | read | List the comments of a Doc, oldest first, with cursor paging (limit 1 to 50) |
| `doc_create` | `doc:write` | write | Create a Doc page in a Workspace |
| `doc_delete` | `doc:delete` | destructive | Delete a Doc and, in the same step, the pages below it that the person may manage (restricted pages they were never given move up and are not revealed) |
| `doc_file_attach` | `doc:write` | write | Attach a file the person uploaded (finished, not yet used by a message) to a Doc page so that an image or file block can reference /v1/files/{fileId}/content |
| `doc_get` | `doc:read` | read | Read one Doc page: title, rich-text content (JSON), revisionNo (needed by doc_save), visibility, parent, Project, latest Version and whether live editing is open |
| `doc_move` | `doc:write` | write | Move a Doc under another parent (parentId null: top level) and, with beforeId, before that sibling |
| `doc_participants` | `doc:read` | read | List the people with an explicit role (MANAGER, EDITOR, VIEWER) on a Doc |
| `doc_references` | `doc:read` | read | List the Docs that reference this Doc (only sources the person can read), with cursor paging |
| `doc_resolve_references` | `doc:read ticket:read` | read | Resolve Ticket/Doc/person ids to titles and places for the ones the person may see (what a chip in a page shows) |
| `doc_save` | `doc:write` | write | Replace the title and the whole content of a Doc with the given draft if expectedRevision is current (read it with doc_get; change the JSON it returned and send it back) |
| `doc_set_participants` | `doc:write` | write | Replace the participant list of a Doc (the whole list; read doc_participants first) |
| `doc_set_visibility` | `doc:write` | write | Set a Doc's visibility to WORKSPACE, PROJECT (with scopeProjectId, optionally scopeTeamId) or RESTRICTED (only participants, see doc_set_participants) |
| `doc_template_create` | `doc:write` | write | Save a Doc the person can read as a template with a name |
| `doc_template_delete` | `doc:write` | write | Delete a Doc template (its creator or a manager, as the web app allows) |
| `doc_template_list` | `doc:read` | read | List the Doc templates of a Workspace (id, name) for doc_create |
| `doc_tree` | `doc:read` | read | List the Doc pages of a Workspace as a tree (id, title, parentId, projectId, visibility) that the person may see; restricted pages they cannot read are absent |
| `doc_update` | `doc:write` | write | Change a Doc's title and/or the Project it belongs to (projectId null removes it from its Project) |
| `doc_version_create` | `doc:write` | write | Freeze the saved Doc as the next Version (needs the current revisionNo) |
| `doc_version_get` | `doc:read` | read | Read one frozen Version of a Doc with its content |
| `doc_version_list` | `doc:read` | read | List the frozen Versions of a Doc, newest first (versionNo, title, author, time, whether it equals the saved page) |
| `doc_version_restore` | `doc:write` | write | Make an older Version the current page, the way the web app does: its title and content are saved as a new revision (needs the current revisionNo; a stale one is version_conflict) |
### file
| Tool | Scope | Risk | What it does |
| `attachment_download` | `file:read` | read | Read the bytes of a file attached to a message, a Ticket or a Doc comment (attachmentId, not fileId) as base64, when it is at most 262144 bytes; larger files are opened in the web app |
| `file_create_small` | `file:write` | write | Upload a file of at most 262144 bytes, sent as base64 in this call, into a Workspace |
| `file_delete` | `file:write` | destructive | Delete one of YOUR uploads that is not attached to anything yet (fileId from file_create_small) |
| `file_download` | `file:read` | read | Read the bytes of a file the person may open (a file of a Doc they can read, or their own finished upload) as base64, when it is at most 262144 bytes; larger files are opened in the web app |
| `ticket_attachment_add` | `file:write ticket:write` | write | Attach a file you uploaded (file_create_small) to the body of a Ticket |
| `ticket_attachment_list` | `file:read ticket:read` | read | List the files attached to the body of a Ticket (id, fileId, filename, byteSize, mime, state, who attached) |
| `ticket_attachment_remove` | `file:write ticket:write` | destructive | Remove an attachment from the body of a Ticket; the file is deleted with it and cannot be restored |
### member
| Tool | Scope | Risk | What it does |
| `member_search` | `member:read` | read | Find people in a Workspace by name or e-mail fragment |
### org
| Tool | Scope | Risk | What it does |
| `department_create` | `team:write` | write | Create a department, optionally under a parent |
| `department_delete` | `team:write` | write | Delete a department; its Teams and sub-departments move up one level, so no Team, member or Project access is lost |
| `department_list` | `workspace:read` | read | The Workspace's department tree (id, name, parent, position) |
| `department_place_team` | `team:write` | write | Place a Team in a department, or take it out (departmentId null) |
| `department_update` | `team:write` | write | Rename or move a department |
| `invitation_create` | `member:write` | write | Invite an e-mail address to join as MEMBER (optionally into a Team) |
| `invitation_list` | `member:read` | read | Invitations of a Workspace with status, role, expiry and mail status (SENT, FAILED, NOT_CONFIGURED) |
| `invitation_reissue` | `member:write` | write | Give an open MEMBER invitation a new link and expiry (the old link stops working) and mail it when possible |
| `invitation_revoke` | `member:write` | write | Cancel an open invitation; its link stops working |
| `member_directory` | `member:read` | read | Active people of a Workspace as every member sees them (name, e-mail, role) |
| `member_list` | `member:read` | read | The Workspace's member roster with roles and states (needs an owner/admin per Core) |
| `member_offboarding_impact` | `member:read` | read | Projects and channels that would be left without an admin or manager if the person left |
| `team_archive` | `team:write` | write | Archive a Team (reversible with team_restore) |
| `team_create` | `team:write` | write | Create a Team with its first members (at least one, each MANAGER or MEMBER) |
| `team_member_remove` | `team:write` | write | Take a person out of a Team (not out of the Workspace) |
| `team_member_set` | `team:write` | write | Put an active member in a Team, or change their role |
| `team_members` | `member:read` | read | A Team's roster with roles (MANAGER, MEMBER) |
| `team_projects` | `workspace:read` | read | Projects linked to a Team with the Team's participation role |
| `team_restore` | `team:write` | write | Bring an archived Team back |
| `team_update` | `team:write` | write | Rename a Team or change its description |
| `workspace_create` | `workspace:write` | write | Create a Workspace owned by the person, where the instance allows it (usually only the first one; otherwise `workspace_creation_closed`) |
| `workspace_update` | `workspace:write` | write | Change the Workspace name or time zone (owner/admin per Core) |
### personal
| Tool | Scope | Risk | What it does |
| `channel_mute_list` | `personal:read` | read | List the ids of the channels you muted, in all Workspaces |
| `channel_mute_set` | `personal:write` | write | Mute or unmute a channel for yourself (it stays quiet in your Inbox and badges) |
| `draft_delete` | `personal:write` | write | Discard one of your drafts (draftId from draft_list) |
| `draft_list` | `personal:read` | read | List your unsent message drafts in a Workspace (only yours; drafts of conversations you can no longer write to are left out) |
| `draft_save` | `personal:write` | write | Save your draft for a channel, DM or thread compose box |
| `favorite_add` | `personal:write` | write | Add something you can read to your favorites |
| `favorite_folder_create` | `personal:write` | write | Create a folder for your favorites |
| `favorite_folder_delete` | `personal:write` | write | Delete one of your favorite folders; its favorites move to the top level and are kept |
| `favorite_folder_list` | `personal:read` | read | List your favorite folders in a Workspace |
| `favorite_folder_order` | `personal:write` | write | Set the order of your favorite folders: folderIds lists all of them in the new order |
| `favorite_folder_rename` | `personal:write` | write | Rename one of your favorite folders |
| `favorite_list` | `personal:read` | read | List your favorites in a Workspace in your order, with folders |
| `favorite_move` | `personal:write` | write | Put a favorite into one of your folders, or to the top level with folderId null |
| `favorite_order` | `personal:write` | write | Set the order of your favorites: items lists every favorite (kind, targetId) in the new order |
| `favorite_remove` | `personal:write` | write | Remove one thing from your favorites |
| `inbox_list` | `personal:read` | read | List your Inbox (or the archive) in a Workspace, newest first, with unreadCount |
| `inbox_read_all` | `personal:write` | write | Mark all your unread Inbox items in a Workspace read (optionally only one notification group) |
| `inbox_update` | `personal:write` | write | Mark one of your Inbox items read or unread, archive, unarchive, or snooze it (until required) |
| `later_add` | `personal:write` | write | Save a Ticket, message or file you can read to your Later list, with an optional note |
| `later_list` | `personal:read` | read | List what you saved for later in a Workspace (Tickets, messages, files) with note, state, version and reminder |
| `later_reminder_cancel` | `personal:write` | write | Cancel the reminder of a Later item |
| `later_reminder_set` | `personal:write` | write | Remind you of a Later item at dueAt (in the future); a new reminder replaces the old one |
| `later_remove` | `personal:write` | write | Remove one of your Later items (and its reminder) |
| `later_update` | `personal:write` | write | Change the state (ACTIVE, DONE, ARCHIVED) or note of one of your Later items |
| `me_get` | `personal:read` | read | Your own account profile: id, name, e-mail, time zone, job title, status |
| `me_update_profile` | `personal:write` | write | Change your display name, job title, time zone or status |
| `notification_preferences_get` | `personal:read` | read | Your notification level (ALL or DIRECT_ONLY), quiet hours and current version |
| `notification_preferences_set` | `personal:write` | write | Set your notification level and quiet hours (expectedVersion from notification_preferences_get) |
| `subscription_get` | `personal:read` | read | Whether you follow a Ticket, a Project or a thread (its root message), and how (AUTO, EXPLICIT, MUTED) |
| `subscription_list` | `personal:read` | read | List the Tickets, Projects and threads you follow in a Workspace |
| `subscription_set` | `personal:write` | write | Follow or stop following a Ticket, a Project or a thread |
### plan
| Tool | Scope | Risk | What it does |
| `board_get` | `plan:read` | read | The board of a Project: workflow columns with the Tickets the person can see, capped per column (perColumnLimit, default 20; each column says its ticketCount) |
| `dependency_add` | `plan:write` | write | Make ticketId depend on predecessorId (BLOCKS or RELATED) |
| `dependency_list` | `plan:read` | read | Blocking and related Tickets of one Ticket with dependency ids and waiver state |
| `dependency_remove` | `plan:write` | write | Delete a dependency link (the Tickets stay) |
| `dependency_restore` | `plan:write` | write | Make a waived dependency blocking again; a reason is recorded |
| `dependency_waive` | `plan:write` | write | Let the blocked Ticket proceed although its blocker is open; a reason is recorded in history |
| `milestone_create` | `plan:write` | write | Create a Milestone in a Project |
| `milestone_update` | `plan:write` | write | Change name, description, target date or state (OPEN, REACHED, CANCELED) of a Milestone |
| `plan_get` | `plan:read` | read | Sprints (with their ids, dates, state and `version`) and Milestones of a Project: the starting point for every other planning tool |
| `plan_schedule` | `plan:read` | read | Tickets of a Project with their start/due schedule and the blocking dependencies between them, paged with `cursor` |
| `report_project_activity` | `plan:read` | read | Latest Ticket activity of a Project, newest first (limit up to 50; the server keeps the latest 100) |
| `report_status_summary` | `plan:read` | read | Ticket counts of a Project by workflow state category |
| `report_velocity` | `plan:read` | read | Completed work of the Project's closed Sprints, newest first |
| `sprint_archive_cancel` | `plan:write` | write | Cancel the scheduled archive of a closed Sprint |
| `sprint_archive_reschedule` | `plan:write` | write | Set the future time at which a closed Sprint's finished work is archived |
| `sprint_archive_schedule` | `plan:read` | read | The scheduled archive of a closed Sprint's finished work (or none), with its `version` |
| `sprint_cancel` | `plan:write` | write | Cancel a Sprint that has not been closed; its Tickets return to the backlog per Core's rules |
| `sprint_close_preview` | `plan:read` | read | What closing the Sprint would do: finished and unfinished Tasks and the Sprints that could take the rest |
| `sprint_complete` | `plan:write` | write | Close an ACTIVE Sprint: the results are frozen, unfinished Tasks go to carryToSprintId or the backlog, optionally archive the finished work |
| `sprint_create` | `plan:write` | write | Create a Sprint in a Project (dates are inclusive, startsOn <= endsOn) |
| `sprint_report` | `plan:read` | read | The figures of one Sprint (scope, completed, burndown) as the person may see them |
| `sprint_results` | `plan:read` | read | The frozen results of a closed Sprint |
| `sprint_start` | `plan:write` | write | Make a PLANNED Sprint ACTIVE |
| `sprint_update` | `plan:write` | write | Change name, goal, dates, time zone or estimate unit of a Sprint |
| `ticket_assign_sprint` | `plan:write` | write | Assign a Ticket to a Sprint, or remove it (sprintId null) |
| `ticket_set_milestones` | `plan:write` | write | Replace the Milestones of a Ticket with the given list (empty list clears) |
| `view_create` | `plan:write` | write | Save a view (filters, sorting, grouping) in a Project, personal or shared with the Project |
| `view_create_workspace` | `plan:write` | write | Save a personal view for a Workspace or Team scope |
| `view_delete` | `plan:write` | write | Delete a saved view (the Tickets are untouched) |
| `view_list` | `plan:read` | read | Saved views of a Project: shared ones and the person's own, with id, layout, definition and `version` |
| `view_list_workspace` | `plan:read` | read | The person's own saved views of a Workspace or Team scope (targetKind WORKSPACE, or TEAM with teamId) |
| `view_update` | `plan:write` | write | Rename a view or change its layout or definition |
| `work_list` | `plan:read` | read | Tickets across the Projects the person can read in a Workspace or Team (the My Work / team work dataset), by lens ALL, ASSIGNED, CREATED or WATCHING, paged with `cursor` (limit up to 50) |
### project
| Tool | Scope | Risk | What it does |
| `label_create` | `project:write` | write | Create a label in a Project (Project admins) |
| `label_list` | `project:read` | read | List the labels of a Project (ids for ticket_create/ticket_update labelIds) |
| `project_calendar_get` | `project:read` | read | Read the Project's work calendar (time zone, weekly working hours, holidays, version) used to derive planned ends; says when none is set |
| `project_calendar_put` | `project:write` | write | Create (expectedVersion 0) or replace the Project's work calendar (Project admins) |
| `project_create` | `project:write` | write | Create a Project in a Workspace (Workspace owner or admin only) |
| `project_designation_set` | `project:manage` | administrative | Allow a direct EDITOR to waive dependencies and/or manage pins (Project admins) |
| `project_get` | `project:read` | read | Read one Project: name, ticket prefix, estimate unit, its workflow states (with the workflow version), labels and ticket types |
| `project_grant_remove` | `project:manage` | administrative | Remove a person's direct Project role (Project admins) |
| `project_grant_set` | `project:manage` | administrative | Give a person a direct role (ADMIN, EDITOR, VIEWER) in a Project, or change it |
| `project_members_list` | `project:read` | read | List the active people a Ticket in this Project can be assigned to (Project participants) |
| `project_permissions_get` | `project:read` | read | Read the Project's access: direct grants, Team participation and designations |
| `project_pin_add` | `project:write` | write | Pin a Ticket or a link to the Project (people allowed to manage pins) |
| `project_pin_remove` | `project:write` | write | Remove a pin from a Project (people allowed to manage pins) |
| `project_pins_list` | `project:read` | read | List the pinned Tickets and links of a Project |
| `project_prefix_check` | `project:read` | read | Check that a ticket prefix (or one derived from a name) is free in a Workspace before project_create |
| `project_private_channels_list` | `project:read` | read | List the Project's private Channels (names and counts only, never messages) for its admins |
| `project_team_link` | `project:manage` | administrative | Link a Team to a Project so it can be given a role there (Project admins) |
| `project_team_members_list` | `project:read` | read | List the members of a Team linked to the Project (Project admins only) |
| `project_team_participation_set` | `project:manage` | administrative | Let a linked Team's members into the Project with a role, or end it with role null (Project admins) |
| `project_team_unlink` | `project:manage` | administrative | Unlink a Team from a Project; its members lose the access that came through it (Project admins) |
| `project_ticket_types` | `project:read` | read | List the ticket types of a Project (ids for ticket_create typeId) |
| `project_update` | `project:write` | write | Change a Project setting (today: the estimate unit; Project admins) |
| `workflow_state_create` | `project:write` | write | Add a state to a Project's workflow (Project admins) |
| `workflow_state_delete` | `project:manage` | destructive | Delete a workflow state of a Project (Project admin only) |
| `workflow_state_update` | `project:write` | write | Rename, re-categorize, reorder, make initial or set the WIP limit of a workflow state (Project admins) |
### search
| Tool | Scope | Risk | What it does |
| `search` | `search:read` | read | Search a Workspace with Penfii search syntax |
### ticket
| Tool | Scope | Risk | What it does |
| `project_workflow` | `ticket:read` | read | List the workflow states of a Project (id, name, category, position) and the workflow version |
| `ticket_activity_list` | `ticket:read` | read | Read the activity feed of one Ticket, newest first, with who did it and through which client (web or an MCP client) |
| `ticket_archive` | `ticket:write` | write | Archive a Ticket (Project admins): it stays readable but leaves the default lists |
| `ticket_bulk_update` | `ticket:write` | write | Apply one change (state, assignee, priority, labels, sprint, milestones, archive, trash) to up to 100 Tickets |
| `ticket_change_status` | `ticket:write` | write | Move a Ticket to another workflow state |
| `ticket_children_list` | `ticket:read` | read | List the child Tickets (Tasks of an Epic, Subtasks of a Task) the person can see, as compact items |
| `ticket_comment` | `ticket:write` | write | Add a comment to a Ticket the person can read |
| `ticket_comment_delete` | `ticket:write` | destructive | Delete a comment (the author, a Project admin or a Channel manager may) |
| `ticket_comment_edit` | `ticket:write` | write | Replace the text of a comment the person wrote (or may moderate) on a Ticket they can edit |
| `ticket_comments_list` | `ticket:read` | read | Read the comments on one Ticket, oldest first |
| `ticket_create` | `ticket:write` | write | Create a Ticket in a Project the person can edit, with the same fields as the web app (labels, type, estimate, schedule, parent, channel, already-uploaded files) |
| `ticket_get` | `ticket:read` | read | Read one Ticket: title, description, status (statusId; names are in project_workflow), assignee, schedule, relations and the current revisionNo |
| `ticket_list` | `ticket:read` | read | List the live Tickets of a Project the person can read (archived and trashed ones are left out), in board order, with filters by state, assignee, priority, kind, parent, label and title text |
| `ticket_move` | `ticket:write` | write | Change a Ticket's place in the Project's shared order (afterId, beforeId or position first/last) and, optionally, its workflow state |
| `ticket_references_list` | `ticket:read` | read | List the Docs, messages and Tickets that link to this Ticket (backlinks) the person may see |
| `ticket_restore_from_trash` | `ticket:write` | write | Bring a trashed Ticket back (Project admins) |
| `ticket_restore_revision` | `ticket:write` | write | Copy chosen fields from a past revision back onto the Ticket as a new revision (history is kept) |
| `ticket_revision_get` | `ticket:read` | read | Read one past revision of a Ticket: who and when, what changed from the previous revision and compared with the current Ticket (field, from, to), and which fields ticket_restore_revision can bring back |
| `ticket_revisions_list` | `ticket:read` | read | List the saved revisions of one Ticket (newest first): number, who, when, and the revision a restore copied from |
| `ticket_trash` | `ticket:write` | write | Move a Ticket to the Project's trash (Project admins) |
| `ticket_unarchive` | `ticket:write` | write | Bring an archived Ticket back into the default lists (Project admins) |
| `ticket_update` | `ticket:write` | write | Change fields of one Ticket (title, description, assignee, priority, due date, labels, parent, type, estimate, schedule) in one revision; only the fields you pass change, null clears where allowed |
### workspace
| Tool | Scope | Risk | What it does |
| `workspace_list` | `workspace:read` | read | List the Workspaces the signed-in person belongs to and where MCP access is turned on |
| `workspace_mcp_policy` | `workspace:read` | read | Show which kinds of MCP access (write, destructive, administrative) a Workspace administrator has allowed |
| `workspace_overview` | `workspace:read` | read | Show the Teams, Projects, Channels and direct messages the person can see in one Workspace |
| `workspace_web_only` | `workspace:read` | read | List what MCP never does directly (account security, push registration, role changes to OWNER/ADMIN, forced removal, permanent delete, webhook secrets, admin recovery) and where in the web app a person does it |
