//! Workspaces, membership, teams, projects, and invitations.

mod departments;
pub use departments::{
    create_department, delete_department, list_departments, place_team, update_department,
};

use chrono::{DateTime, Duration, Utc};
use penfii_contracts::{
    AcceptInvitationResponse, Channel, DirectMessage, Invitation, PrefixCheck, PrefixStatus,
    Project, Space, Team, TeamMemberInput, User, Workspace,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::access;
use crate::admin;
use crate::conversation;
use crate::error::CoreError;
use crate::identity::{self, require_label};
use crate::limits::Limits;
use crate::ticket_key;

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: Uuid,
    name: String,
    time_zone: String,
    role: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct TeamRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    archived_at: Option<DateTime<Utc>>,
    description: Option<String>,
    department_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    ticket_prefix: String,
    estimate_unit: String,
    archived_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ChannelRow {
    id: Uuid,
    workspace_id: Uuid,
    name: String,
    description: Option<String>,
    topic: Option<String>,
    visibility: String,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
    allow_member_invites: bool,
    archived_at: Option<DateTime<Utc>>,
    role: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DmRow {
    id: Uuid,
    workspace_id: Uuid,
    created_at: DateTime<Utc>,
    last_message_seq: i64,
    last_change_seq: i64,
}

pub async fn list_workspaces(pool: &PgPool, user_id: Uuid) -> Result<Vec<Workspace>, CoreError> {
    let rows = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT w.id, w.name, w.time_zone, m.role, w.created_at
         FROM penfii.workspaces w
         JOIN penfii.workspace_members m ON m.workspace_id = w.id
         WHERE m.user_id = $1 AND m.state = 'ACTIVE'
         ORDER BY w.created_at, w.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(workspace_from).collect())
}

/// Changes the name or time zone a Workspace owner or admin set. An empty body is refused.
/// The time zone is one PostgreSQL knows, the same check a Sprint uses.
pub async fn update_workspace(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: Option<&str>,
    time_zone: Option<&str>,
) -> Result<Workspace, CoreError> {
    if name.is_none() && time_zone.is_none() {
        return Err(CoreError::validation("nothing to update"));
    }
    let name = name.map(|value| require_label(value, "name")).transpose()?;
    let time_zone = time_zone
        .map(|value| require_label(value, "timeZone"))
        .transpose()?;
    let mut tx = pool.begin().await?;
    let role = crate::admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    if let Some(zone) = &time_zone {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
                .bind(zone)
                .fetch_one(&mut *tx)
                .await?;
        if !known {
            return Err(CoreError::validation("unknown time zone"));
        }
    }
    sqlx::query(
        "UPDATE penfii.workspaces SET name = COALESCE($2, name), time_zone = COALESCE($3, time_zone) WHERE id = $1",
    )
    .bind(workspace_id)
    .bind(name.as_deref())
    .bind(time_zone.as_deref())
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT w.id, w.name, w.time_zone, m.role, w.created_at
         FROM penfii.workspaces w
         JOIN penfii.workspace_members m ON m.workspace_id = w.id
         WHERE w.id = $1 AND m.user_id = $2 AND m.state = 'ACTIVE'",
    )
    .bind(workspace_id)
    .bind(actor_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut workspace = workspace_from(row);
    workspace.role = role;
    Ok(workspace)
}

/// The technical guard on a Team description (not a product limit; the UI shows no counter).
const TEAM_DESCRIPTION_GUARD: usize = 2000;

/// Plain text: trimmed, empty is none.
fn clean_description(raw: Option<&str>) -> Result<Option<String>, CoreError> {
    let text = raw.map(str::trim).filter(|text| !text.is_empty());
    if let Some(text) = text
        && text.chars().count() > TEAM_DESCRIPTION_GUARD
    {
        return Err(CoreError::validation(
            "description is too long (the technical limit is 2000 characters)",
        ));
    }
    Ok(text.map(str::to_string))
}

/// Changes a Team's name and/or description (`description`: `None` leaves it, `Some(None)` clears it).
/// A Workspace owner or admin, or a manager of that Team, may do it; an archived Team is not edited.
pub async fn update_team(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
    name: Option<&str>,
    description: Option<Option<&str>>,
) -> Result<Team, CoreError> {
    let name = name.map(|name| require_label(name, "name")).transpose()?;
    let description = description
        .map(|text| clean_description(text))
        .transpose()?;
    if name.is_none() && description.is_none() {
        return Err(CoreError::validation("name or description is required"));
    }
    let mut tx = pool.begin().await?;
    crate::admin::require_team_manager(&mut tx, workspace_id, team_id, actor_id).await?;
    let before = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT name, description FROM penfii.teams WHERE id = $1 FOR UPDATE",
    )
    .bind(team_id)
    .fetch_one(&mut *tx)
    .await?;
    let new_name = name.clone().unwrap_or_else(|| before.0.clone());
    let new_description = match &description {
        Some(text) => text.clone(),
        None => before.1.clone(),
    };
    let row = sqlx::query_as::<_, TeamRow>(
        "UPDATE penfii.teams SET name = $3, description = $4
         WHERE id = $1 AND workspace_id = $2
         RETURNING id, workspace_id, name, archived_at, description, department_id",
    )
    .bind(team_id)
    .bind(workspace_id)
    .bind(&new_name)
    .bind(&new_description)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    if new_name != before.0 || new_description != before.1 {
        admin::audit(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "team.updated",
            "TEAM",
            team_id,
            json!({
                "name": {"before": before.0, "after": new_name},
                "description": {"before": before.1, "after": new_description},
            }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(team_from(row))
}

/// Archives (`archive`) or restores a Team. Only `archived_at` changes: memberships, roles, Project
/// participation and channels are untouched, so restoring loses nothing. Owner or admin only.
pub async fn set_team_archived(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
    archive: bool,
) -> Result<Team, CoreError> {
    let mut tx = pool.begin().await?;
    crate::admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    let current: Option<Option<DateTime<Utc>>> = sqlx::query_scalar(
        "SELECT archived_at FROM penfii.teams WHERE id = $1 AND workspace_id = $2 FOR UPDATE",
    )
    .bind(team_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let archived_at = current.ok_or(CoreError::NotFound)?;
    if archive == archived_at.is_some() {
        return Err(CoreError::conflict(
            if archive {
                "team_already_archived"
            } else {
                "team_not_archived"
            },
            if archive {
                "the Team is already archived"
            } else {
                "the Team is not archived"
            },
        ));
    }
    let row = sqlx::query_as::<_, TeamRow>(
        "UPDATE penfii.teams SET archived_at = CASE WHEN $3 THEN now() ELSE NULL END
         WHERE id = $1 AND workspace_id = $2
         RETURNING id, workspace_id, name, archived_at, description, department_id",
    )
    .bind(team_id)
    .bind(workspace_id)
    .bind(archive)
    .fetch_one(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        workspace_id,
        Some(actor_id),
        if archive {
            "team.archived"
        } else {
            "team.restored"
        },
        "TEAM",
        team_id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(team_from(row))
}

/// The Projects linked to a Team, for any Workspace member: the Team's participation role in each, and the
/// asking person's own access (role and where it comes from). Nothing about other members is returned, and a
/// Project the person does not take part in shows only its name and the Team's role.
pub async fn team_projects(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
) -> Result<Vec<penfii_contracts::TeamProject>, CoreError> {
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, actor_id).await?;
    crate::admin::team_in_workspace(&mut tx, workspace_id, team_id).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<DateTime<Utc>>, Option<String>)>(
        "SELECT p.id, p.name, p.ticket_prefix, p.archived_at, pt.access_role
         FROM penfii.project_teams pt
         JOIN penfii.projects p ON p.id = pt.project_id
         WHERE pt.team_id = $1 AND p.workspace_id = $2
         ORDER BY p.name, p.id",
    )
    .bind(team_id)
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for (project_id, name, ticket_prefix, archived_at, team_role) in rows {
        let (_, my_role, sources) =
            crate::project_role::project_role_sources(&mut tx, project_id, actor_id).await?;
        items.push(penfii_contracts::TeamProject {
            project_id,
            name,
            ticket_prefix,
            archived_at,
            team_role,
            can_open: my_role.is_some(),
            my_role,
            my_sources: sources.into_iter().map(str::to_string).collect(),
        });
    }
    tx.commit().await?;
    Ok(items)
}

async fn workspace_exists(conn: &mut sqlx::PgConnection) -> Result<bool, CoreError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM penfii.workspaces)")
            .fetch_one(conn)
            .await?,
    )
}

/// Whether `POST /v1/workspaces` can succeed now: only while the instance has no Workspace.
pub async fn workspace_creation_open(pool: &PgPool) -> Result<bool, CoreError> {
    let mut conn = pool.acquire().await?;
    Ok(!workspace_exists(&mut conn).await?)
}

pub async fn create_workspace(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    time_zone: &str,
) -> Result<Workspace, CoreError> {
    let name = require_label(name, "name")?;
    let time_zone = require_label(time_zone, "timeZone")?;
    let mut tx = pool.begin().await?;
    // Bootstrap only: the first Workspace of an instance. The lock makes two first creations queue,
    // so the second one sees the first and is refused.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('penfii.workspace_bootstrap'))")
        .execute(&mut *tx)
        .await?;
    if workspace_exists(&mut tx).await? {
        return Err(CoreError::rule(
            "workspace_creation_closed",
            "workspaces are created by an invitation-based owner, not by sign-in",
        ));
    }
    let id = identity::insert_owned_workspace(&mut tx, user_id, &name, &time_zone).await?;
    let created_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM penfii.workspaces WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(Workspace {
        id,
        name,
        time_zone,
        role: "OWNER".to_string(),
        created_at,
    })
}

pub async fn create_invitation(
    pool: &PgPool,
    limits: &Limits,
    actor_id: Uuid,
    workspace_id: Uuid,
    email: &str,
    role: &str,
    team_id: Option<Uuid>,
) -> Result<Invitation, CoreError> {
    let ttl = limits.invite_ttl_seconds.ok_or_else(|| {
        CoreError::conflict("invite_ttl_unconfigured", "invite ttl is not configured")
    })?;
    if !matches!(role, "OWNER" | "ADMIN" | "MEMBER") {
        return Err(CoreError::validation(
            "role must be OWNER, ADMIN, or MEMBER",
        ));
    }
    let (_, email_key) = identity::normalize_email(email)?;
    let mut tx = pool.begin().await?;
    let actor_role = access::require_member(&mut tx, workspace_id, actor_id).await?;
    if actor_role != "OWNER" && actor_role != "ADMIN" {
        return Err(CoreError::forbidden("workspace owner or admin required"));
    }
    // Only an owner hands out the owner or admin role; an admin invites plain members.
    if actor_role != "OWNER" && role != "MEMBER" {
        return Err(CoreError::forbidden(
            "only a workspace owner invites an owner or admin",
        ));
    }
    if admin::is_active_member_email(&mut tx, workspace_id, &email_key).await? {
        return Err(CoreError::conflict(
            "already_member",
            "this e-mail already belongs to a member of the workspace",
        ));
    }
    // The Team step is for the new member only, always as MEMBER. An archived Team is not offered.
    let team_name: Option<String> = match team_id {
        None => None,
        Some(team_id) => {
            crate::admin::team_open_for_writes(&mut tx, workspace_id, team_id).await?;
            Some(
                sqlx::query_scalar("SELECT name FROM penfii.teams WHERE id = $1")
                    .bind(team_id)
                    .fetch_one(&mut *tx)
                    .await?,
            )
        }
    };
    let (token, token_hash) = identity::new_secret();
    let id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::seconds(ttl);
    sqlx::query(
        "INSERT INTO penfii.workspace_invitations
            (id, workspace_id, email_key, role, token_hash, expires_at, created_by, team_id, team_name)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(&email_key)
    .bind(role)
    .bind(&token_hash)
    .bind(expires_at)
    .bind(actor_id)
    .bind(team_id)
    .bind(&team_name)
    .execute(&mut *tx)
    .await?;
    admin::audit(
        &mut tx,
        workspace_id,
        Some(actor_id),
        "invitation.created",
        "INVITATION",
        id,
        json!({"email": email_key, "role": role, "teamId": team_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(Invitation {
        id,
        workspace_id,
        email: email_key,
        role: role.to_string(),
        expires_at,
        invite_token: limits.dev_return_invite_token.then_some(token),
        team_id,
    })
}

/// What the holder of an invitation link may know before signing in: the invited e-mail, the Workspace
/// and role, and whether that e-mail already has an account (then the person signs in to accept).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvitationPreview {
    pub email: String,
    pub workspace_name: String,
    pub role: String,
    pub account_exists: bool,
}

pub async fn preview_invitation(
    pool: &PgPool,
    token: &str,
) -> Result<InvitationPreview, CoreError> {
    use sha2::{Digest, Sha256};
    let token_hash = Sha256::digest(token.as_bytes()).to_vec();
    let row: Option<(String, String, String, bool)> = sqlx::query_as(
        "SELECT i.email_key, w.name, i.role,
                EXISTS (SELECT 1 FROM penfii.users u WHERE u.email_key = i.email_key)
         FROM penfii.workspace_invitations i
         JOIN penfii.workspaces w ON w.id = i.workspace_id
         WHERE i.token_hash = $1 AND i.revoked_at IS NULL AND i.accepted_at IS NULL
           AND i.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await?;
    let (email, workspace_name, role, account_exists) = row.ok_or(CoreError::NotFound)?;
    Ok(InvitationPreview {
        email,
        workspace_name,
        role,
        account_exists,
    })
}

pub async fn accept_invitation(
    pool: &PgPool,
    token: &str,
    display_name: &str,
    password: &str,
    session: Option<&identity::AuthSession>,
) -> Result<AcceptInvitationResponse, CoreError> {
    let display_name = require_label(display_name, "displayName")?;
    let token_hash = {
        use sha2::{Digest, Sha256};
        Sha256::digest(token.as_bytes()).to_vec()
    };
    let mut tx = pool.begin().await?;
    let invite = sqlx::query_as::<_, InviteRow>(
        "SELECT id, workspace_id, email_key, role, team_id, team_name
         FROM penfii.workspace_invitations
         WHERE token_hash = $1
           AND revoked_at IS NULL
           AND accepted_at IS NULL
           AND expires_at > now()
         FOR UPDATE",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(invite) = invite else {
        return Err(CoreError::NotFound);
    };

    let existing = sqlx::query_as::<_, ExistingUser>(
        "SELECT u.id, u.email, u.display_name, u.locale, u.time_zone, u.email_verified_at,
                u.disabled_at, p.password_hash, u.job_title, u.status_emoji, u.status_text,
                u.status_clear_at, (u.avatar_object_key IS NOT NULL) AS has_avatar
         FROM penfii.users u
         LEFT JOIN penfii.password_credentials p ON p.user_id = u.id
         WHERE u.email_key = $1
         FOR UPDATE OF u",
    )
    .bind(&invite.email_key)
    .fetch_optional(&mut *tx)
    .await?;

    let user = if let Some(existing) = existing {
        // An existing account accepts as itself: its own session (recently authenticated, because
        // this adds a Workspace), or the password inline. The token alone is never enough.
        // A disabled account cannot accept; the answer is the same as for a wrong password.
        if existing.disabled_at.is_some() {
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
        let by_session = match session {
            Some(session) if session.user.id == existing.id => {
                identity::require_recent_authentication(pool, session).await?;
                true
            }
            _ => false,
        };
        let matches = by_session
            || match existing.password_hash.as_deref() {
                Some(hash) => identity::verify_password(password, hash).await,
                None => false,
            };
        if !matches {
            return Err(CoreError::conflict(
                "account_exists",
                "account already exists",
            ));
        }
        {
            let (status_emoji, status_text, status_clear_at) = identity::shown_status(
                existing.status_emoji,
                existing.status_text,
                existing.status_clear_at,
            );
            User {
                id: existing.id,
                email: existing.email,
                display_name: existing.display_name,
                locale: existing.locale,
                time_zone: existing.time_zone,
                email_verified_at: existing.email_verified_at,
                job_title: existing.job_title.filter(|value| !value.is_empty()),
                status_emoji,
                status_text,
                status_clear_at,
                has_avatar: existing.has_avatar,
            }
        }
    } else {
        identity::validate_new_password(password, &invite.email_key)?;
        let user_id = identity::insert_user(
            &mut tx,
            &invite.email_key,
            &invite.email_key,
            &display_name,
            password,
            true,
        )
        .await?;
        identity::plain_user(
            user_id,
            invite.email_key.clone(),
            display_name,
            None,
            None,
            Some(Utc::now()),
        )
    };

    // A person who was removed earlier comes back with the invited role; an active member keeps theirs.
    let joined = sqlx::query(
        "INSERT INTO penfii.workspace_members (workspace_id, user_id, role, state)
         VALUES ($1, $2, $3, 'ACTIVE')
         ON CONFLICT (workspace_id, user_id) DO UPDATE
            SET state = 'ACTIVE', role = EXCLUDED.role, deactivated_at = NULL, joined_at = now()
            WHERE penfii.workspace_members.state = 'INACTIVE'",
    )
    .bind(invite.workspace_id)
    .bind(user.id)
    .bind(&invite.role)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if joined {
        admin::audit(
            &mut tx,
            invite.workspace_id,
            Some(user.id),
            "member.joined",
            "USER",
            user.id,
            json!({"userId": user.id, "role": invite.role, "invitationId": invite.id}),
        )
        .await?;
    }
    sqlx::query(
        "UPDATE penfii.workspace_invitations
         SET accepted_at = now(), accepted_user_id = $2
         WHERE id = $1",
    )
    .bind(invite.id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    let workspace_id = invite.workspace_id;
    let team = place_invited_team(
        &mut tx,
        workspace_id,
        user.id,
        invite.id,
        invite.team_id,
        invite.team_name.as_deref(),
    )
    .await?;
    tx.commit().await?;
    Ok(AcceptInvitationResponse {
        user,
        workspace_id,
        team,
    })
}

/// The Team step of an accepted invitation, in the transaction of the Workspace membership: the person
/// joins the Team as `MEMBER` if it exists, is active and belongs to the Workspace; otherwise the Team
/// is skipped and the outcome says why. Joining the Workspace never fails because of it.
pub(crate) async fn place_invited_team(
    tx: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
    invitation_id: Uuid,
    team_id: Option<Uuid>,
    team_name: Option<&str>,
) -> Result<Option<penfii_contracts::InvitationTeamOutcome>, CoreError> {
    if team_id.is_none() && team_name.is_none() {
        return Ok(None);
    }
    // Share-locked so an archive running at the same time is ordered before or after this step.
    let team: Option<(Uuid, String, bool)> = match team_id {
        Some(team_id) => {
            sqlx::query_as(
                "SELECT id, name, archived_at IS NOT NULL FROM penfii.teams
                 WHERE id = $1 AND workspace_id = $2 FOR SHARE",
            )
            .bind(team_id)
            .bind(workspace_id)
            .fetch_optional(&mut *tx)
            .await?
        }
        None => None,
    };
    let (status, name) = match &team {
        Some((_, name, false)) => ("JOINED", name.clone()),
        Some((_, name, true)) => ("ARCHIVED", name.clone()),
        None => ("GONE", team_name.unwrap_or_default().to_string()),
    };
    if let Some((team_id, _, false)) = team {
        sqlx::query(
            "INSERT INTO penfii.team_members (team_id, user_id, role) VALUES ($1, $2, 'MEMBER')
             ON CONFLICT (team_id, user_id) DO NOTHING",
        )
        .bind(team_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        admin::audit(
            &mut *tx,
            workspace_id,
            Some(user_id),
            "team.member_added",
            "TEAM",
            team_id,
            json!({"userId": user_id, "role": "MEMBER", "invitationId": invitation_id}),
        )
        .await?;
    } else {
        admin::audit(
            &mut *tx,
            workspace_id,
            Some(user_id),
            "invitation.team_skipped",
            "INVITATION",
            invitation_id,
            json!({"teamId": team_id, "teamName": name, "reason": status}),
        )
        .await?;
    }
    Ok(Some(penfii_contracts::InvitationTeamOutcome {
        team_id: team_id.filter(|_| status != "GONE"),
        team_name: name,
        status: status.to_string(),
    }))
}

/// Creates a Team with its first members, each with an explicit role, at least one of them a `MANAGER`. Creating a Team is for a Workspace
/// owner or admin and does not make the creator a member: the creator belongs only if listed.
pub async fn create_team(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
    description: Option<&str>,
    members: &[TeamMemberInput],
) -> Result<Team, CoreError> {
    let mut tx = pool.begin().await?;
    crate::admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    let name = require_label(name, "name")?;
    let description = clean_description(description)?;
    if members.is_empty() {
        return Err(CoreError::validation(
            "a Team needs at least one member, each with a role",
        ));
    }
    let mut distinct: Vec<Uuid> = members.iter().map(|member| member.user_id).collect();
    distinct.sort();
    distinct.dedup();
    if distinct.len() != members.len() {
        return Err(CoreError::validation("members must be distinct"));
    }
    for member in members {
        crate::admin::team_role(&member.role)?;
    }
    if !members.iter().any(|member| member.role == "MANAGER") {
        return Err(CoreError::validation("a Team needs at least one MANAGER"));
    }
    for member in members {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.workspace_members
                           WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE')",
        )
        .bind(workspace_id)
        .bind(member.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !active {
            return Err(CoreError::validation(
                "every member must be an active member of the Workspace",
            ));
        }
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.teams (id, workspace_id, name, description) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(&name)
    .bind(&description)
    .execute(&mut *tx)
    .await?;
    for member in members {
        sqlx::query("INSERT INTO penfii.team_members (team_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(member.user_id)
            .bind(&member.role)
            .execute(&mut *tx)
            .await?;
    }
    crate::events::emit_simple(
        &mut tx,
        workspace_id,
        "TEAM",
        id,
        "team.created",
        Some(actor_id),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Team {
        id,
        workspace_id,
        name,
        archived_at: None,
        description,
        department_id: None,
    })
}

pub async fn create_project(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
    team_ids: &[Uuid],
    ticket_prefix: Option<&str>,
) -> Result<Project, CoreError> {
    let name = require_label(name, "name")?;
    let chosen_prefix = ticket_prefix
        .map(ticket_key::canonical_prefix)
        .transpose()?;
    let mut distinct = team_ids.to_vec();
    distinct.sort();
    distinct.dedup();
    if distinct.len() != team_ids.len() {
        return Err(CoreError::validation("teamIds must be distinct"));
    }
    let mut tx = pool.begin().await?;
    crate::admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    for team_id in &distinct {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.teams WHERE id = $1 AND workspace_id = $2)",
        )
        .bind(team_id)
        .bind(workspace_id)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            return Err(CoreError::NotFound);
        }
    }
    let id = Uuid::new_v4();
    let prefix = match chosen_prefix {
        Some(prefix) => prefix,
        None => ticket_key::available_suggestion(&mut tx, workspace_id, &name)
            .await?
            .ok_or_else(|| {
                CoreError::validation("ticketPrefix is required for this project name")
            })?,
    };
    // The unique index on (workspace, lower(prefix)) decides races; the error is mapped below.
    let inserted = sqlx::query("INSERT INTO penfii.projects (id, workspace_id, name, ticket_prefix) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(workspace_id)
        .bind(&name)
        .bind(&prefix)
        .execute(&mut *tx)
        .await;
    if let Err(error) = inserted {
        let taken = error
            .as_database_error()
            .is_some_and(|db| db.code().as_deref() == Some("23505"));
        return Err(if taken {
            CoreError::conflict(
                "ticket_prefix_taken",
                "this ticket prefix is already used by another project",
            )
        } else {
            error.into()
        });
    }
    sqlx::query("INSERT INTO penfii.workflows (project_id) VALUES ($1)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for (status_name, category, position, initial) in [
        ("Todo", "NOT_STARTED", 0_i32, true),
        ("In Progress", "IN_PROGRESS", 1, false),
        ("Done", "DONE", 2, false),
        ("Canceled", "CANCELED", 3, false),
    ] {
        let status_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO penfii.workflow_statuses (id, project_id, name, category, position, is_initial)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(status_id)
        .bind(id)
        .bind(status_name)
        .bind(category)
        .bind(position)
        .bind(initial)
        .execute(&mut *tx)
        .await?;
        crate::work::record_status_category(&mut tx, status_id, category).await?;
    }
    for (type_name, description, color) in [
        ("FEATURE", "New capability or improvement", "#2f6fd0"),
        ("BUGFIX", "Defect to fix", "#c8452c"),
        ("ISSUE", "Question, risk, or other work item", "#7a6a1f"),
    ] {
        sqlx::query(
            "INSERT INTO penfii.ticket_types (id, project_id, name, description, color)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(type_name)
        .bind(description)
        .bind(color)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO penfii.project_user_grants (project_id, user_id, role)
         VALUES ($1, $2, 'ADMIN')",
    )
    .bind(id)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;
    for team_id in &distinct {
        // Linked only: a link gives the Team's members no access until a Project admin gives the Team a role.
        sqlx::query("INSERT INTO penfii.project_teams (project_id, team_id) VALUES ($1, $2)")
            .bind(id)
            .bind(team_id)
            .execute(&mut *tx)
            .await?;
    }
    conversation::insert_channel(
        &mut tx,
        actor_id,
        workspace_id,
        &name,
        None,
        "PUBLIC",
        None,
        Some(id),
    )
    .await?;
    let created_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM penfii.projects WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    crate::events::emit_simple(
        &mut tx,
        workspace_id,
        "PROJECT",
        id,
        "project.created",
        Some(actor_id),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Project {
        id,
        workspace_id,
        name,
        ticket_prefix: prefix,
        estimate_unit: "NONE".to_string(),
        archived_at: None,
        created_at,
        team_ids: distinct,
        participating: true,
        can_manage: true,
    })
}

/// Changes the Project's estimate unit. `NONE` to a unit is always allowed. Any other change
/// (`POINTS` to `HOURS`, a unit to `NONE`) only while no Sprint is ACTIVE, because a running
/// Sprint's baseline and Report are counted in the unit it started with. Estimates already
/// written keep their own unit and are never converted. A PLANNED Sprint follows the new unit.
pub async fn update_project_estimate_unit(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    unit: &str,
) -> Result<Project, CoreError> {
    if !matches!(unit, "NONE" | "POINTS" | "HOURS") {
        return Err(CoreError::validation(
            "estimateUnit must be NONE, POINTS or HOURS",
        ));
    }
    let mut tx = pool.begin().await?;
    crate::project_role::require_project_admin(&mut tx, project_id, actor_id).await?;
    let current: String =
        sqlx::query_scalar("SELECT estimate_unit FROM penfii.projects WHERE id=$1 FOR UPDATE")
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
    if current != unit {
        if current != "NONE" {
            let active: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM penfii.sprints WHERE project_id=$1 AND state='ACTIVE')",
            )
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await?;
            if active {
                return Err(CoreError::conflict(
                    "estimate_unit_locked",
                    "the estimate unit cannot change while a Sprint is active",
                ));
            }
        }
        sqlx::query("UPDATE penfii.projects SET estimate_unit=$2 WHERE id=$1")
            .bind(project_id)
            .bind(unit)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE penfii.sprints SET estimate_unit=$2, version=version+1
             WHERE project_id=$1 AND state='PLANNED'",
        )
        .bind(project_id)
        .bind(unit)
        .execute(&mut *tx)
        .await?;
    }
    let row = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, name, ticket_prefix, estimate_unit, archived_at, created_at
         FROM penfii.projects WHERE id=$1",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    let teams: Vec<Uuid> = sqlx::query_scalar(
        "SELECT team_id FROM penfii.project_teams WHERE project_id=$1 ORDER BY team_id",
    )
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(project_from(row, teams, true, true))
}

/// Suggests a Prefix for a Project name and says whether a typed Prefix can be used.
pub async fn prefix_check(
    pool: &PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
    name: Option<&str>,
    prefix: Option<&str>,
) -> Result<PrefixCheck, CoreError> {
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, user_id).await?;
    let suggestion = match name {
        Some(name) => ticket_key::available_suggestion(&mut tx, workspace_id, name).await?,
        None => None,
    };
    let status = match prefix.map(str::trim).filter(|value| !value.is_empty()) {
        None => None,
        Some(value) => Some(match ticket_key::canonical_prefix(value) {
            Err(_) => PrefixStatus {
                value: value.to_ascii_uppercase(),
                valid: false,
                available: false,
            },
            Ok(canonical) => {
                let taken = ticket_key::prefix_taken(&mut tx, workspace_id, &canonical).await?;
                PrefixStatus {
                    value: canonical,
                    valid: true,
                    available: !taken,
                }
            }
        }),
    };
    tx.commit().await?;
    Ok(PrefixCheck {
        suggestion,
        prefix: status,
    })
}

pub async fn space(pool: &PgPool, user_id: Uuid, workspace_id: Uuid) -> Result<Space, CoreError> {
    let mut tx = pool.begin().await?;
    access::require_member(&mut tx, workspace_id, user_id).await?;
    let teams = sqlx::query_as::<_, TeamRow>(
        "SELECT id, workspace_id, name, archived_at, description, department_id
         FROM penfii.teams
         WHERE workspace_id = $1
         ORDER BY name, id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let projects = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, workspace_id, name, ticket_prefix, estimate_unit, archived_at, created_at
         FROM penfii.projects
         WHERE workspace_id = $1
         ORDER BY name, id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let participating: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM penfii.projects WHERE workspace_id = $1 AND penfii.in_share_scope(id, NULL, $2)",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    // What the person manages (Project admin, directly or by group); the client shows its actions by it.
    let mut managing: Vec<Uuid> = Vec::new();
    for id in &participating {
        let (_, role) = crate::project_role::project_role(&mut tx, *id, user_id).await?;
        if role.as_deref().is_some_and(crate::project_role::can_manage) {
            managing.push(*id);
        }
    }
    let links = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT pt.project_id, pt.team_id
         FROM penfii.project_teams pt
         JOIN penfii.projects p ON p.id = pt.project_id
         WHERE p.workspace_id = $1
         ORDER BY pt.team_id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *tx)
    .await?;
    let channels = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT ch.id, c.workspace_id, ch.name, ch.description, ch.topic, ch.visibility,
                ch.team_id, ch.project_id, ch.allow_member_invites, ch.archived_at, cm.role
         FROM penfii.channels ch
         JOIN penfii.conversations c ON c.id = ch.id
         LEFT JOIN penfii.channel_members cm
            ON cm.channel_id = ch.id AND cm.user_id = $2
         WHERE c.workspace_id = $1 AND {}
         ORDER BY ch.name, ch.id",
        crate::access::CHANNEL_READABLE
    ))
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let dms = sqlx::query_as::<_, DmRow>(
        "SELECT c.id, c.workspace_id, c.created_at, c.last_message_seq, c.last_change_seq
         FROM penfii.conversations c
         JOIN penfii.dm_members me ON me.conversation_id = c.id AND me.user_id = $2
         WHERE c.workspace_id = $1 AND c.kind = 'DM'
         ORDER BY c.created_at, c.id",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let dm_ids: Vec<Uuid> = dms.iter().map(|dm| dm.id).collect();
    let members = if dm_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, (Uuid, Uuid)>(
            "SELECT conversation_id, user_id
             FROM penfii.dm_members
             WHERE conversation_id = ANY($1)
             ORDER BY user_id",
        )
        .bind(&dm_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    let joined_ids: Vec<Uuid> = channels
        .iter()
        .filter(|channel| {
            channel.role.is_some()
                || (channel.visibility == "PUBLIC" && channel.project_id.is_some())
        })
        .map(|channel| channel.id)
        .chain(dm_ids.iter().copied())
        .collect();
    let sidebar = conversation_sidebar(&mut tx, user_id, &joined_ids, &dm_ids).await?;
    tx.commit().await?;

    let mut team_ids_by_project: Vec<(Uuid, Vec<Uuid>)> = Vec::new();
    for (project_id, team_id) in links {
        if let Some((_, ids)) = team_ids_by_project
            .iter_mut()
            .find(|(id, _)| *id == project_id)
        {
            ids.push(team_id);
        } else {
            team_ids_by_project.push((project_id, vec![team_id]));
        }
    }

    Ok(Space {
        teams: teams.into_iter().map(team_from).collect(),
        projects: projects
            .into_iter()
            .map(|row| {
                let team_ids = team_ids_by_project
                    .iter()
                    .find(|(id, _)| *id == row.id)
                    .map(|(_, ids)| ids.clone())
                    .unwrap_or_default();
                let taking_part = participating.contains(&row.id);
                let can_manage = managing.contains(&row.id);
                project_from(row, team_ids, taking_part, can_manage)
            })
            .collect(),
        channels: channels
            .into_iter()
            .map(|row| {
                let mut channel = channel_from(row);
                channel.unread_count = sidebar.unread.get(&channel.id).copied().unwrap_or(0);
                channel
            })
            .collect(),
        dms: {
            let mut listed: Vec<DirectMessage> = dms
                .into_iter()
                .map(|row| {
                    let user_ids = members
                        .iter()
                        .filter(|(conversation_id, _)| *conversation_id == row.id)
                        .map(|(_, user_id)| *user_id)
                        .collect();
                    let preview = sidebar.preview.get(&row.id);
                    DirectMessage {
                        id: row.id,
                        workspace_id: row.workspace_id,
                        user_ids,
                        created_at: row.created_at,
                        last_message_seq: row.last_message_seq,
                        last_change_seq: row.last_change_seq,
                        peers: sidebar.peers.get(&row.id).cloned().unwrap_or_default(),
                        last_message_preview: preview.map(|(text, _)| text.clone()),
                        last_message_at: preview.map(|(_, at)| *at),
                        unread_count: sidebar.unread.get(&row.id).copied().unwrap_or(0),
                    }
                })
                .collect();
            listed.sort_by(|left, right| {
                right
                    .last_message_at
                    .cmp(&left.last_message_at)
                    .then(right.created_at.cmp(&left.created_at))
            });
            listed
        },
    })
}

struct SidebarFacts {
    unread: std::collections::HashMap<Uuid, i64>,
    preview: std::collections::HashMap<Uuid, (String, DateTime<Utc>)>,
    peers: std::collections::HashMap<Uuid, Vec<penfii_contracts::ConversationPeer>>,
}

async fn conversation_sidebar(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    unread_ids: &[Uuid],
    dm_ids: &[Uuid],
) -> Result<SidebarFacts, CoreError> {
    use penfii_contracts::ConversationPeer;
    use serde_json::Value;
    let mut unread = std::collections::HashMap::new();
    if !unread_ids.is_empty() {
        let rows = sqlx::query_as::<_, (Uuid, i64)>(
            "SELECT m.conversation_id, count(*)::bigint
             FROM penfii.messages m
             LEFT JOIN penfii.conversation_reads r
               ON r.conversation_id = m.conversation_id AND r.user_id = $2
             WHERE m.conversation_id = ANY($1)
               AND m.thread_root_id IS NULL
               AND m.deleted_at IS NULL
               AND m.author_id <> $2
               AND m.stream_seq > COALESCE(r.manual_unread_from_seq, r.read_through_seq, 0)
             GROUP BY m.conversation_id",
        )
        .bind(unread_ids)
        .bind(user_id)
        .fetch_all(&mut **tx)
        .await?;
        unread.extend(rows);
    }
    let mut preview = std::collections::HashMap::new();
    let mut peers = std::collections::HashMap::new();
    if !dm_ids.is_empty() {
        let rows = sqlx::query_as::<_, (Uuid, Value, DateTime<Utc>)>(
            "SELECT DISTINCT ON (conversation_id) conversation_id, body, created_at
             FROM penfii.messages
             WHERE conversation_id = ANY($1)
               AND thread_root_id IS NULL
               AND deleted_at IS NULL
             ORDER BY conversation_id, stream_seq DESC",
        )
        .bind(dm_ids)
        .fetch_all(&mut **tx)
        .await?;
        for (id, body, at) in rows {
            let text = message_preview(&body, 80);
            if !text.is_empty() {
                preview.insert(id, (text, at));
            }
        }
        let rows = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>)>(
            "SELECT m.conversation_id, u.id, u.display_name, u.username
             FROM penfii.dm_members m
             JOIN penfii.users u ON u.id = m.user_id
             WHERE m.conversation_id = ANY($1) AND u.id <> $2
             ORDER BY u.display_name, u.id",
        )
        .bind(dm_ids)
        .bind(user_id)
        .fetch_all(&mut **tx)
        .await?;
        for (conversation_id, id, display_name, username) in rows {
            peers
                .entry(conversation_id)
                .or_insert_with(Vec::new)
                .push(ConversationPeer {
                    id,
                    display_name,
                    username,
                });
        }
    }
    Ok(SidebarFacts {
        unread,
        preview,
        peers,
    })
}

fn message_preview(value: &serde_json::Value, max_chars: usize) -> String {
    let mut parts = Vec::new();
    fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(text)) = map.get("text")
                    && !text.is_empty()
                {
                    out.push(text.clone());
                }
                for (key, child) in map {
                    if key != "text" {
                        walk(child, out);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    walk(child, out);
                }
            }
            _ => {}
        }
    }
    walk(value, &mut parts);
    let text = parts.join(" ");
    let mut chars = text.chars();
    let clipped: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn workspace_from(row: WorkspaceRow) -> Workspace {
    Workspace {
        id: row.id,
        name: row.name,
        time_zone: row.time_zone,
        role: row.role,
        created_at: row.created_at,
    }
}

fn team_from(row: TeamRow) -> Team {
    Team {
        id: row.id,
        workspace_id: row.workspace_id,
        name: row.name,
        archived_at: row.archived_at,
        description: row.description,
        department_id: row.department_id,
    }
}

fn project_from(
    row: ProjectRow,
    team_ids: Vec<Uuid>,
    participating: bool,
    can_manage: bool,
) -> Project {
    Project {
        id: row.id,
        workspace_id: row.workspace_id,
        name: row.name,
        ticket_prefix: row.ticket_prefix,
        estimate_unit: row.estimate_unit,
        archived_at: row.archived_at,
        created_at: row.created_at,
        team_ids,
        participating,
        can_manage,
    }
}

fn channel_from(row: ChannelRow) -> Channel {
    Channel {
        id: row.id,
        workspace_id: row.workspace_id,
        name: row.name,
        description: row.description,
        topic: row.topic,
        visibility: row.visibility,
        team_id: row.team_id,
        project_id: row.project_id,
        allow_member_invites: row.allow_member_invites,
        archived_at: row.archived_at,
        role: row.role,
        unread_count: 0,
    }
}

#[derive(sqlx::FromRow)]
struct InviteRow {
    id: Uuid,
    workspace_id: Uuid,
    email_key: String,
    role: String,
    team_id: Option<Uuid>,
    team_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ExistingUser {
    id: Uuid,
    email: String,
    display_name: String,
    locale: Option<String>,
    time_zone: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
    disabled_at: Option<DateTime<Utc>>,
    password_hash: Option<String>,
    job_title: Option<String>,
    status_emoji: Option<String>,
    status_text: Option<String>,
    status_clear_at: Option<DateTime<Utc>>,
    has_avatar: bool,
}
