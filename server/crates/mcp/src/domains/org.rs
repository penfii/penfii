//! MCP operations: Workspace, Teams, Departments, members and invitations (FEA-169).
//!
//! Not here, on purpose (see `web_only`): changing a member's role, forced removal, enabling or
//! disabling an account, and invitations that hand out OWNER or ADMIN. An invitation's secret link
//! is never returned: only the invitation's id, target, role, expiry and mail status.

use penfii_contracts::TeamMemberInput;
use penfii_core::oauth::{Principal, RiskClass};
use penfii_core::{access, admin, conversation, organization, project_access};
use serde_json::{Value, json};

use super::{Handled, cap_arrays, items_of, page};
use crate::McpContext;
use crate::catalog::{
    Operation, Toolset, WorkspaceResolver, object_schema, uuid_property, workspace_id_property,
};
use crate::tools::{Done, Failure, announce, done, nullable, string_arg, to_value, uuid_arg};

const WS_R: &[&str] = &["workspace:read"];
const WS_W: &[&str] = &["workspace:write"];
const MEMBER_R: &[&str] = &["member:read"];
const MEMBER_W: &[&str] = &["member:write"];
const TEAM_W: &[&str] = &["team:write"];
const ARG: WorkspaceResolver = WorkspaceResolver::Argument;

fn ws_only() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property()}),
        &["workspaceId"],
    )
}
fn team_id_prop() -> Value {
    uuid_property("Team id from workspace_overview.")
}
fn schema_paged() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "Default 20."},
               "cursor": {"type": "string", "description": "nextCursor of the previous page."}}),
        &["workspaceId"],
    )
}
fn schema_team() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "teamId": team_id_prop()}),
        &["workspaceId", "teamId"],
    )
}
fn schema_offboarding() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "userId": uuid_property("Person id from member_search or member_list.")}),
        &["workspaceId", "userId"],
    )
}
fn schema_workspace_create() -> Value {
    object_schema(
        json!({"name": {"type": "string", "minLength": 1},
               "timeZone": {"type": "string", "description": "IANA zone such as Asia/Seoul."}}),
        &["name", "timeZone"],
    )
}
fn schema_workspace_update() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "name": {"type": "string"},
               "timeZone": {"type": "string"}}),
        &["workspaceId"],
    )
}
fn schema_team_create() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "name": {"type": "string", "minLength": 1},
               "description": {"type": "string"},
               "members": {"type": "array", "minItems": 1, "description": "First members; the creator is not added unless listed.",
                   "items": {"type": "object", "properties": {
                       "userId": uuid_property("Person id."),
                       "role": {"type": "string", "enum": ["MANAGER", "MEMBER"]}},
                       "required": ["userId", "role"], "additionalProperties": false}}}),
        &["workspaceId", "name", "members"],
    )
}
fn schema_team_update() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "teamId": team_id_prop(),
               "name": {"type": "string"},
               "description": {"type": ["string", "null"], "description": "null clears it."}}),
        &["workspaceId", "teamId"],
    )
}
fn schema_team_member_set() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "teamId": team_id_prop(),
               "userId": uuid_property("An active Workspace member."),
               "role": {"type": "string", "enum": ["MANAGER", "MEMBER"]}}),
        &["workspaceId", "teamId", "userId", "role"],
    )
}
fn schema_team_member() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "teamId": team_id_prop(),
               "userId": uuid_property("The Team member.")}),
        &["workspaceId", "teamId", "userId"],
    )
}
fn schema_department_create() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "name": {"type": "string", "minLength": 1},
               "parentId": {"type": ["string", "null"], "format": "uuid", "description": "Parent department; null for the top level."}}),
        &["workspaceId", "name"],
    )
}
fn schema_department_update() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "departmentId": uuid_property("Department id from department_list."),
               "name": {"type": "string", "minLength": 1},
               "parentId": {"type": ["string", "null"], "format": "uuid"}}),
        &["workspaceId", "departmentId", "name"],
    )
}
fn schema_department() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "departmentId": uuid_property("Department id from department_list.")}),
        &["workspaceId", "departmentId"],
    )
}
fn schema_team_place() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(), "teamId": team_id_prop(),
               "departmentId": {"type": ["string", "null"], "format": "uuid", "description": "null takes the Team out of any department."}}),
        &["workspaceId", "teamId", "departmentId"],
    )
}
fn schema_invitation_create() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "email": {"type": "string", "format": "email"},
               "teamId": uuid_property("Optional Team the new member joins on accepting (as MEMBER).")}),
        &["workspaceId", "email"],
    )
}
fn schema_invitation() -> Value {
    object_schema(
        json!({"workspaceId": workspace_id_property(),
               "invitationId": uuid_property("Invitation id from invitation_list.")}),
        &["workspaceId", "invitationId"],
    )
}

pub static OPS: &[Operation] = &[
    Operation::new(
        Toolset::Org,
        "member.list",
        "member_list",
        "List Workspace members",
        "The Workspace's member roster with roles and states (needs an owner/admin per Core). Paged (limit up to 50, cursor). Read-only. Changing roles or removing people is done in the web app (workspace_web_only).",
        MEMBER_R,
        RiskClass::Read,
        ARG,
        schema_paged,
    ),
    Operation::new(
        Toolset::Org,
        "member.directory",
        "member_directory",
        "List people in a Workspace",
        "Active people of a Workspace as every member sees them (name, e-mail, role). Paged. Read-only. To find one person use member_search.",
        MEMBER_R,
        RiskClass::Read,
        ARG,
        schema_paged,
    ),
    Operation::new(
        Toolset::Org,
        "member.offboarding_impact",
        "member_offboarding_impact",
        "What removing a member would affect",
        "Projects and channels that would be left without an admin or manager if the person left. Read-only; removal itself is web-only.",
        MEMBER_R,
        RiskClass::Read,
        ARG,
        schema_offboarding,
    ),
    Operation::new(
        Toolset::Org,
        "team.projects",
        "team_projects",
        "Projects of a Team",
        "Projects linked to a Team with the Team's participation role. Read-only.",
        WS_R,
        RiskClass::Read,
        ARG,
        schema_team,
    ),
    Operation::new(
        Toolset::Org,
        "team.members",
        "team_members",
        "Members of a Team",
        "A Team's roster with roles (MANAGER, MEMBER). Read-only.",
        MEMBER_R,
        RiskClass::Read,
        ARG,
        schema_team,
    ),
    Operation::new(
        Toolset::Org,
        "department.list",
        "department_list",
        "List departments",
        "The Workspace's department tree (id, name, parent, position). Read-only.",
        WS_R,
        RiskClass::Read,
        ARG,
        ws_only,
    ),
    Operation::new(
        Toolset::Org,
        "invitation.list",
        "invitation_list",
        "List invitations",
        "Invitations of a Workspace with status, role, expiry and mail status (SENT, FAILED, NOT_CONFIGURED). Never includes links. Needs an owner/admin per Core. Read-only.",
        MEMBER_R,
        RiskClass::Read,
        ARG,
        ws_only,
    ),
    Operation::new(
        Toolset::Org,
        "workspace.create",
        "workspace_create",
        "Create a Workspace",
        "Create a Workspace owned by the person, where the instance allows it (usually only the first one; otherwise `workspace_creation_closed`). Needs a connection that is not limited to chosen Workspaces.",
        WS_W,
        RiskClass::Write,
        WorkspaceResolver::None,
        schema_workspace_create,
    ),
    Operation::new(
        Toolset::Org,
        "workspace.update",
        "workspace_update",
        "Update Workspace settings",
        "Change the Workspace name or time zone (owner/admin per Core).",
        WS_W,
        RiskClass::Write,
        ARG,
        schema_workspace_update,
    ),
    Operation::new(
        Toolset::Org,
        "team.create",
        "team_create",
        "Create a Team",
        "Create a Team with its first members (at least one, each MANAGER or MEMBER). Owner/admin per Core.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team_create,
    ),
    Operation::new(
        Toolset::Org,
        "team.update",
        "team_update",
        "Update a Team",
        "Rename a Team or change its description.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team_update,
    ),
    Operation::new(
        Toolset::Org,
        "team.archive",
        "team_archive",
        "Archive a Team",
        "Archive a Team (reversible with team_restore). Its access to Projects follows Core's rules for archived Teams.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team,
    ),
    Operation::new(
        Toolset::Org,
        "team.restore",
        "team_restore",
        "Restore an archived Team",
        "Bring an archived Team back.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team,
    ),
    Operation::new(
        Toolset::Org,
        "team.member_set",
        "team_member_set",
        "Add or change a Team member",
        "Put an active member in a Team, or change their role. Core refuses adding yourself to a Team that would make you admin of a Project you do not administer (`team_self_promotion`) and removing the last manager.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team_member_set,
    ),
    Operation::new(
        Toolset::Org,
        "team.member_remove",
        "team_member_remove",
        "Remove a Team member",
        "Take a person out of a Team (not out of the Workspace). Core refuses removing the last manager.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team_member,
    ),
    Operation::new(
        Toolset::Org,
        "department.create",
        "department_create",
        "Create a department",
        "Create a department, optionally under a parent.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_department_create,
    ),
    Operation::new(
        Toolset::Org,
        "department.update",
        "department_update",
        "Update a department",
        "Rename or move a department.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_department_update,
    ),
    Operation::new(
        Toolset::Org,
        "department.delete",
        "department_delete",
        "Delete a department",
        "Delete a department; its Teams and sub-departments move up one level, so no Team, member or Project access is lost.",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_department,
    ),
    Operation::new(
        Toolset::Org,
        "department.place_team",
        "department_place_team",
        "Put a Team in a department",
        "Place a Team in a department, or take it out (departmentId null).",
        TEAM_W,
        RiskClass::Write,
        ARG,
        schema_team_place,
    ),
    Operation::new(
        Toolset::Org,
        "invitation.create",
        "invitation_create",
        "Invite someone as a member",
        "Invite an e-mail address to join as MEMBER (optionally into a Team). An open invitation for the address is re-issued instead of duplicated, so a retry is safe. The link is mailed when the server sends e-mail (`emailStatus` SENT / FAILED / NOT_CONFIGURED); this tool never returns the link — when mail is not sent, an owner/admin copies it in the web app. Inviting as OWNER or ADMIN is web-only.",
        MEMBER_W,
        RiskClass::Write,
        ARG,
        schema_invitation_create,
    ),
    Operation::new(
        Toolset::Org,
        "invitation.reissue",
        "invitation_reissue",
        "Send an invitation again",
        "Give an open MEMBER invitation a new link and expiry (the old link stops working) and mail it when possible. Returns no link. OWNER/ADMIN invitations are re-issued in the web app.",
        MEMBER_W,
        RiskClass::Write,
        ARG,
        schema_invitation,
    ),
    Operation::new(
        Toolset::Org,
        "invitation.revoke",
        "invitation_revoke",
        "Revoke an invitation",
        "Cancel an open invitation; its link stops working.",
        MEMBER_W,
        RiskClass::Write,
        ARG,
        schema_invitation,
    ),
];

/// The invitation without its secret.
fn invitation_value(invitation: &penfii_contracts::Invitation) -> Value {
    json!({
        "id": invitation.id,
        "workspaceId": invitation.workspace_id,
        "email": invitation.email,
        "role": invitation.role,
        "teamId": invitation.team_id,
        "expiresAt": invitation.expires_at,
        "reissued": invitation.reissued,
        "emailStatus": invitation.email_status,
        "emailError": invitation.email_error,
        "linkNote": "The invitation link is not returned by MCP. If emailStatus is not SENT, an owner or admin copies the link in the web app (Workspace settings > Members > Invitations).",
    })
}

fn web_only(what: &str) -> Failure {
    Failure::new(
        "web_only",
        format!("{what} is done in the web app, not through MCP (see workspace_web_only)"),
    )
}

pub(super) async fn run(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Handled {
    Some(run_org(ctx, principal, op, args).await)
}

async fn run_org(
    ctx: &McpContext,
    principal: &Principal,
    op: &Operation,
    args: &Value,
) -> Result<Done, Failure> {
    let user = principal.user_id;
    let pool = &ctx.pool;
    let ws = || uuid_arg(args, "workspaceId");
    match op.id {
        "member.list" => {
            let members = admin::list_members(pool, user, ws()?).await?;
            Ok(done(page(items_of(&members)?, args)?))
        }
        "member.directory" => {
            let people = admin::list_people(pool, user, ws()?).await?;
            Ok(done(page(items_of(&people)?, args)?))
        }
        "member.offboarding_impact" => {
            let (workspace, target) = (ws()?, uuid_arg(args, "userId")?);
            let projects =
                project_access::offboarding_impact(pool, user, workspace, target).await?;
            let channels =
                conversation::recovery::offboarding_impact(pool, user, workspace, target).await?;
            let mut value = json!({"projects": to_value(&projects)?, "channels": to_value(&channels)?,
                "note": "Forced removal is done in the web app (workspace_web_only)."});
            cap_arrays(&mut value, 100);
            Ok(done(value))
        }
        "team.projects" => {
            let items =
                organization::team_projects(pool, user, ws()?, uuid_arg(args, "teamId")?).await?;
            Ok(done(json!({"items": to_value(&items)?})))
        }
        "team.members" => {
            let items =
                admin::list_team_members(pool, user, ws()?, uuid_arg(args, "teamId")?).await?;
            Ok(done(json!({"items": to_value(&items)?})))
        }
        "department.list" => {
            let items = organization::list_departments(pool, user, ws()?).await?;
            Ok(done(json!({"items": to_value(&items)?})))
        }
        "invitation.list" => {
            let items = admin::list_invitations(pool, user, ws()?).await?;
            Ok(done(page(items_of(&items)?, args)?))
        }
        "workspace.create" => {
            // Not tied to a Workspace yet, so no Workspace policy or restriction applies to the call.
            // A connection limited to chosen Workspaces may not make new ones.
            if principal.workspace_filter.is_some() {
                return Err(Failure::new(
                    "forbidden",
                    "this connection is limited to chosen Workspaces and cannot create one",
                ));
            }
            let workspace = organization::create_workspace(
                pool,
                user,
                string_arg(args, "name").unwrap_or(""),
                string_arg(args, "timeZone").unwrap_or(""),
            )
            .await?;
            Ok(Done::about_own(to_value(&workspace)?, "workspace"))
        }
        "workspace.update" => {
            let workspace = organization::update_workspace(
                pool,
                user,
                ws()?,
                string_arg(args, "name"),
                string_arg(args, "timeZone"),
            )
            .await?;
            Ok(Done::about_own(to_value(&workspace)?, "workspace"))
        }
        "team.create" => {
            let members: Vec<TeamMemberInput> =
                serde_json::from_value(args.get("members").cloned().unwrap_or(Value::Null))
                    .map_err(|_| Failure::invalid("members must be a list of {userId, role}"))?;
            let team = organization::create_team(
                pool,
                user,
                ws()?,
                string_arg(args, "name").unwrap_or(""),
                string_arg(args, "description"),
                &members,
            )
            .await?;
            Ok(Done::about_own(to_value(&team)?, "team"))
        }
        "team.update" => {
            let description = nullable(args, "description", |v| v.as_str().map(str::to_string))?;
            let team = organization::update_team(
                pool,
                user,
                ws()?,
                uuid_arg(args, "teamId")?,
                string_arg(args, "name"),
                description.as_ref().map(|text| text.as_deref()),
            )
            .await?;
            Ok(Done::about_own(to_value(&team)?, "team"))
        }
        "team.archive" | "team.restore" => {
            let team = organization::set_team_archived(
                pool,
                user,
                ws()?,
                uuid_arg(args, "teamId")?,
                op.id == "team.archive",
            )
            .await?;
            Ok(Done::about_own(to_value(&team)?, "team"))
        }
        "team.member_set" => {
            let team_id = uuid_arg(args, "teamId")?;
            let added = admin::add_team_member(
                pool,
                user,
                ws()?,
                team_id,
                uuid_arg(args, "userId")?,
                string_arg(args, "role").unwrap_or(""),
            )
            .await?;
            Ok(Done::about(
                json!({"added": added, "teamId": team_id}),
                "team",
                team_id,
            ))
        }
        "team.member_remove" => {
            let team_id = uuid_arg(args, "teamId")?;
            let notices =
                admin::remove_team_member(pool, user, ws()?, team_id, uuid_arg(args, "userId")?)
                    .await?;
            announce(ctx, notices);
            Ok(Done::about(
                json!({"removed": true, "teamId": team_id}),
                "team",
                team_id,
            ))
        }
        "department.create" => {
            let parent = nullable(args, "parentId", crate::tools::parse_uuid)?.flatten();
            let department = organization::create_department(
                pool,
                user,
                ws()?,
                string_arg(args, "name").unwrap_or(""),
                parent,
            )
            .await?;
            Ok(Done::about_own(to_value(&department)?, "department"))
        }
        "department.update" => {
            let department_id = uuid_arg(args, "departmentId")?;
            // Core replaces the parent with what it is given: an absent `parentId` keeps the current
            // one (null moves the department to the top level).
            let parent = match nullable(args, "parentId", crate::tools::parse_uuid)? {
                Some(parent) => parent,
                None => sqlx::query_scalar::<_, Option<uuid::Uuid>>(
                    "SELECT parent_id FROM penfii.departments WHERE id = $1 AND workspace_id = $2",
                )
                .bind(department_id)
                .bind(ws()?)
                .fetch_optional(pool)
                .await?
                .flatten(),
            };
            let department = organization::update_department(
                pool,
                user,
                ws()?,
                department_id,
                string_arg(args, "name").unwrap_or(""),
                parent,
            )
            .await?;
            Ok(Done::about_own(to_value(&department)?, "department"))
        }
        "department.delete" => {
            let id = uuid_arg(args, "departmentId")?;
            organization::delete_department(pool, user, ws()?, id).await?;
            Ok(Done::about(json!({"deleted": true}), "department", id))
        }
        "department.place_team" => {
            let department = nullable(args, "departmentId", crate::tools::parse_uuid)?
                .ok_or_else(|| Failure::invalid("departmentId is required (null removes)"))?;
            let team =
                organization::place_team(pool, user, ws()?, uuid_arg(args, "teamId")?, department)
                    .await?;
            Ok(Done::about_own(to_value(&team)?, "team"))
        }
        "invitation.create" => {
            // Q1/Q2: only MEMBER invitations; the role is not an argument, so an OWNER/ADMIN
            // invitation cannot be asked for.
            if args.get("role").is_some() {
                return Err(web_only("Inviting with a role other than MEMBER"));
            }
            let invitation = organization::create_invitation(
                pool,
                &ctx.services.limits,
                ctx.services.mailer.as_deref(),
                user,
                ws()?,
                string_arg(args, "email").unwrap_or(""),
                "MEMBER",
                args.get("teamId").and_then(crate::tools::parse_uuid),
            )
            .await?;
            Ok(Done::about_own(invitation_value(&invitation), "invitation"))
        }
        "invitation.reissue" => {
            let (workspace, id) = (ws()?, uuid_arg(args, "invitationId")?);
            // An OWNER/ADMIN invitation is re-issued in the web app. The role is looked up only for
            // someone who is allowed to see invitations at all, so existence is not revealed.
            let mut conn = pool.acquire().await?;
            let role = access::require_member(&mut conn, workspace, user).await?;
            drop(conn);
            if role == "OWNER" || role == "ADMIN" {
                let invited: Option<String> = sqlx::query_scalar(
                    "SELECT role FROM penfii.workspace_invitations WHERE id = $1 AND workspace_id = $2",
                )
                .bind(id)
                .bind(workspace)
                .fetch_optional(pool)
                .await?;
                if invited.is_some_and(|role| role != "MEMBER") {
                    return Err(web_only("Re-issuing an OWNER or ADMIN invitation"));
                }
            }
            let invitation = organization::reissue_invitation(
                pool,
                &ctx.services.limits,
                ctx.services.mailer.as_deref(),
                user,
                workspace,
                id,
            )
            .await?;
            Ok(Done::about_own(invitation_value(&invitation), "invitation"))
        }
        "invitation.revoke" => {
            let id = uuid_arg(args, "invitationId")?;
            admin::revoke_invitation(pool, user, ws()?, id).await?;
            Ok(Done::about(json!({"revoked": true}), "invitation", id))
        }
        other => Err(Failure::new(
            "internal_error",
            format!("operation {other} has no handler"),
        )),
    }
}
