//! Workspace administration: members, roles, account state, invitations, team membership and the
//! audit log.
//!
//! Who may do what (Workspace roles `OWNER` > `ADMIN` > `MEMBER`):
//! - Owners and admins list the management member table, invitations and the audit log, and manage
//!   team membership. Every workspace member may read a Team roster and the people directory.
//! - Only an owner changes a role, grants `OWNER` / `ADMIN` by invitation, removes an owner or admin,
//!   or disables an account. An admin removes and invites plain members.
//! - Nobody changes their own role, removes themself or disables their own account here, so a
//!   Workspace always keeps the owner who is acting. The last owner is also checked explicitly.
//!
//! "Remove" ends the Workspace membership (`state = 'INACTIVE'`); the person's history stays and a new
//! invitation brings them back. "Disable account" is the instance-wide switch (`users.disabled_at`):
//! it ends every session and grant at once and blocks sign-in. Access is decided from PostgreSQL on
//! every request, so a removed or disabled person loses access without waiting for a cache.

use chrono::{DateTime, Utc};
use penfii_contracts::{
    AuditPage, AuditRecord, InvitationSummary, MemberTeam, TeamMember, WorkspaceMember,
    WorkspacePerson,
};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::paging;

const AUDIT_PAGE_DEFAULT: i64 = 50;
const AUDIT_PAGE_MAX: i64 = 200;

/// Appends one row to the audit log (`audit_entries`) inside the caller's transaction.
pub(crate) async fn audit(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: Option<Uuid>,
    action: &str,
    target_kind: &str,
    target_id: Uuid,
    data: Value,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.audit_entries
            (id, workspace_id, actor_user_id, action_code, outcome_code, target_kind, target_id,
             command_id, data)
         VALUES ($1, $2, $3, $4, 'OK', $5, $6, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(workspace_id)
    .bind(actor)
    .bind(action)
    .bind(target_kind)
    .bind(target_id)
    .bind(Uuid::new_v4())
    .bind(data)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The caller's role when it is `OWNER` or `ADMIN`; a non-member gets "not found", a plain member 403.
pub(crate) async fn require_manager(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    actor: Uuid,
) -> Result<String, CoreError> {
    let role = access::require_member(conn, workspace_id, actor).await?;
    if role != "OWNER" && role != "ADMIN" {
        return Err(CoreError::forbidden("workspace owner or admin required"));
    }
    Ok(role)
}

fn require_owner(role: &str) -> Result<(), CoreError> {
    if role != "OWNER" {
        return Err(CoreError::forbidden("workspace owner required"));
    }
    Ok(())
}

struct MemberRow {
    role: String,
}

/// Locks the Workspace's active owners (so two concurrent changes queue) and returns who they are.
/// "Active" here is a member whose account is not disabled either: an owner who cannot sign in cannot
/// act as one, so they do not count towards the Workspace keeping an owner. (The same meaning as for
/// a Team's manager, a channel's Manager and a Project's admin.)
async fn lock_active_owners(
    conn: &mut PgConnection,
    workspace_id: Uuid,
) -> Result<Vec<Uuid>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT wm.user_id FROM penfii.workspace_members wm
         JOIN penfii.users u ON u.id = wm.user_id AND u.disabled_at IS NULL
         WHERE wm.workspace_id = $1 AND wm.state = 'ACTIVE' AND wm.role = 'OWNER'
         ORDER BY wm.user_id FOR UPDATE OF wm",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?)
}

/// How many active owners there are besides `target`.
fn other_owners(owners: &[Uuid], target: Uuid) -> usize {
    owners.iter().filter(|owner| **owner != target).count()
}

async fn locked_active_member(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<MemberRow, CoreError> {
    let row = sqlx::query(
        "SELECT role FROM penfii.workspace_members
         WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE' FOR UPDATE",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    Ok(MemberRow {
        role: row.try_get("role")?,
    })
}

fn not_self(actor: Uuid, target: Uuid) -> Result<(), CoreError> {
    if actor == target {
        return Err(CoreError::conflict(
            "cannot_modify_self",
            "you cannot change your own membership; ask another owner",
        ));
    }
    Ok(())
}

pub async fn list_members(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<WorkspaceMember>, CoreError> {
    let mut conn = pool.acquire().await?;
    require_manager(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query(
        "SELECT m.user_id, u.display_name, u.email, m.role, m.state, m.joined_at, m.deactivated_at,
                u.disabled_at IS NOT NULL AS account_disabled,
                COALESCE((SELECT jsonb_agg(jsonb_build_object('id', t.id, 'name', t.name) ORDER BY t.name, t.id)
                          FROM penfii.team_members tm
                          JOIN penfii.teams t ON t.id = tm.team_id
                          WHERE tm.user_id = m.user_id AND t.workspace_id = m.workspace_id
                            AND t.archived_at IS NULL), '[]'::jsonb) AS teams
         FROM penfii.workspace_members m
         JOIN penfii.users u ON u.id = m.user_id
         WHERE m.workspace_id = $1
         ORDER BY (m.state = 'ACTIVE') DESC,
                  CASE m.role WHEN 'OWNER' THEN 0 WHEN 'ADMIN' THEN 1 ELSE 2 END,
                  lower(u.display_name), m.user_id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            let teams: Vec<MemberTeam> =
                serde_json::from_value(row.try_get::<Value, _>("teams")?).unwrap_or_default();
            Ok(WorkspaceMember {
                user_id: row.try_get("user_id")?,
                display_name: row.try_get("display_name")?,
                email: row.try_get("email")?,
                role: row.try_get("role")?,
                state: row.try_get("state")?,
                joined_at: row.try_get("joined_at")?,
                deactivated_at: row.try_get("deactivated_at")?,
                account_disabled: row.try_get("account_disabled")?,
                teams,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// Every active member may see the people of the workspace, including who is on no team.
pub async fn list_people(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<WorkspacePerson>, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query(
        "SELECT m.user_id, u.display_name, u.email, m.role, m.state,
                NULLIF(u.job_title, '') AS job_title, u.time_zone,
                CASE WHEN u.status_clear_at IS NOT NULL AND u.status_clear_at <= now() THEN NULL
                     ELSE NULLIF(u.status_emoji, '') END AS status_emoji,
                CASE WHEN u.status_clear_at IS NOT NULL AND u.status_clear_at <= now() THEN NULL
                     ELSE NULLIF(u.status_text, '') END AS status_text,
                CASE WHEN u.status_clear_at IS NOT NULL AND u.status_clear_at <= now() THEN NULL
                     ELSE u.status_clear_at END AS status_clear_at,
                (u.avatar_object_key IS NOT NULL) AS has_avatar,
                COALESCE((SELECT jsonb_agg(jsonb_build_object('id', t.id, 'name', t.name) ORDER BY t.name, t.id)
                          FROM penfii.team_members tm
                          JOIN penfii.teams t ON t.id = tm.team_id
                          WHERE tm.user_id = m.user_id AND t.workspace_id = m.workspace_id
                            AND t.archived_at IS NULL), '[]'::jsonb) AS teams
         FROM penfii.workspace_members m
         JOIN penfii.users u ON u.id = m.user_id
         WHERE m.workspace_id = $1
         ORDER BY (m.state = 'ACTIVE') DESC, lower(u.display_name), m.user_id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            let teams: Vec<MemberTeam> =
                serde_json::from_value(row.try_get::<Value, _>("teams")?).unwrap_or_default();
            Ok(WorkspacePerson {
                user_id: row.try_get("user_id")?,
                display_name: row.try_get("display_name")?,
                email: row.try_get("email")?,
                role: row.try_get("role")?,
                state: row.try_get("state")?,
                job_title: row.try_get("job_title")?,
                time_zone: row.try_get("time_zone")?,
                status_emoji: row.try_get("status_emoji")?,
                status_text: row.try_get("status_text")?,
                status_clear_at: row.try_get("status_clear_at")?,
                has_avatar: row.try_get("has_avatar")?,
                teams,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// Owner only. Returns whether the role changed.
pub async fn change_member_role(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    target: Uuid,
    role: &str,
) -> Result<bool, CoreError> {
    let role = crate::membership::validate_role(role)?;
    let mut tx = pool.begin().await?;
    let actor_role = require_manager(&mut tx, workspace_id, actor).await?;
    require_owner(&actor_role)?;
    not_self(actor, target)?;
    let changed = apply_role_change(&mut tx, workspace_id, Some(actor), target, role, None).await?;
    tx.commit().await?;
    Ok(changed.is_some())
}

/// The role change itself, for the web and for the operator: the Workspace keeps at least one active
/// owner (the owners are locked, so two changes at once queue), only an active member has a role to
/// change, and the same role again changes nothing. Returns the role the member had before, or
/// `None` when nothing changed. Who may do it is for the caller to decide. The audit entry has the
/// actor, or none and `via: admin-cli` for the operator.
async fn apply_role_change(
    tx: &mut PgConnection,
    workspace_id: Uuid,
    actor: Option<Uuid>,
    target: Uuid,
    role: &str,
    via: Option<&str>,
) -> Result<Option<String>, CoreError> {
    let owners = lock_active_owners(&mut *tx, workspace_id).await?;
    let member = locked_active_member(&mut *tx, workspace_id, target).await?;
    if member.role == role {
        return Ok(None);
    }
    if member.role == "OWNER" && other_owners(&owners, target) == 0 {
        return Err(CoreError::conflict(
            "last_owner",
            "a workspace needs at least one owner",
        ));
    }
    sqlx::query(
        "UPDATE penfii.workspace_members SET role = $3
         WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(target)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    let mut data = json!({"userId": target, "before": member.role, "after": role});
    if let Some(via) = via {
        data["via"] = json!(via);
    }
    audit(
        &mut *tx,
        workspace_id,
        actor,
        "member.role_changed",
        "USER",
        target,
        data,
    )
    .await?;
    Ok(Some(member.role))
}

/// What the operator's `workspace-set-role` did.
#[derive(Debug)]
pub struct OperatorRoleChange {
    pub user_id: Uuid,
    pub before: String,
    pub after: String,
    /// False when the member already had the role.
    pub changed: bool,
}

/// `admin-cli workspace-set-role`: the role change of the web, for when no owner can make it. The
/// same rules (an active member, the last owner stays, one audit entry) without an acting person, so
/// the owner's "not yourself" rule does not apply. The account must be an active member.
pub async fn operator_set_member_role(
    pool: &PgPool,
    workspace_id: Uuid,
    email: &str,
    role: &str,
) -> Result<OperatorRoleChange, CoreError> {
    let role = crate::membership::validate_role(role)?;
    let (_, email_key) = crate::identity::normalize_email(email)?;
    let mut tx = pool.begin().await?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM penfii.workspaces WHERE id = $1")
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Err(CoreError::NotFound);
    }
    let user: Option<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT u.id, (SELECT m.state FROM penfii.workspace_members m
                       WHERE m.workspace_id = $2 AND m.user_id = u.id)
         FROM penfii.users u WHERE u.email_key = $1",
    )
    .bind(&email_key)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((user_id, state)) = user else {
        return Err(CoreError::rule(
            "account_not_found",
            "no account has this e-mail",
        ));
    };
    if state.as_deref() != Some("ACTIVE") {
        return Err(CoreError::rule(
            "not_a_member",
            "the account is not an active member of this workspace; add it with workspace-add-member",
        ));
    }
    let before = apply_role_change(
        &mut tx,
        workspace_id,
        None,
        user_id,
        role,
        Some("admin-cli"),
    )
    .await?;
    tx.commit().await?;
    Ok(OperatorRoleChange {
        user_id,
        changed: before.is_some(),
        before: before.unwrap_or_else(|| role.to_string()),
        after: role.to_string(),
    })
}

/// One Workspace as the operator's `workspace-list` shows it.
#[derive(Debug)]
pub struct OperatorWorkspace {
    pub id: Uuid,
    pub name: String,
    pub time_zone: String,
    pub created_at: DateTime<Utc>,
    /// Members whose Workspace membership is ACTIVE (an account that is disabled is still counted).
    pub active_members: i64,
    /// Of them, the ones whose account is disabled: they cannot sign in.
    pub disabled_members: i64,
    /// E-mails of the active owners.
    pub owners: Vec<String>,
    /// The owners among them whose account is disabled.
    pub disabled_owners: Vec<String>,
}

/// `admin-cli workspace-list`: every Workspace of the instance, oldest first, to find the id the
/// other workspace commands take.
pub async fn operator_list_workspaces(pool: &PgPool) -> Result<Vec<OperatorWorkspace>, CoreError> {
    let rows = sqlx::query(
        "SELECT w.id, w.name, w.time_zone, w.created_at,
                (SELECT count(*) FROM penfii.workspace_members m
                 WHERE m.workspace_id = w.id AND m.state = 'ACTIVE') AS active_members,
                (SELECT count(*) FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id
                 WHERE m.workspace_id = w.id AND m.state = 'ACTIVE'
                   AND u.disabled_at IS NOT NULL) AS disabled_members,
                COALESCE((SELECT array_agg(u.email ORDER BY u.email)
                          FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id
                          WHERE m.workspace_id = w.id AND m.state = 'ACTIVE' AND m.role = 'OWNER'),
                         ARRAY[]::text[]) AS owners,
                COALESCE((SELECT array_agg(u.email ORDER BY u.email)
                          FROM penfii.workspace_members m JOIN penfii.users u ON u.id = m.user_id
                          WHERE m.workspace_id = w.id AND m.state = 'ACTIVE' AND m.role = 'OWNER'
                            AND u.disabled_at IS NOT NULL),
                         ARRAY[]::text[]) AS disabled_owners
         FROM penfii.workspaces w ORDER BY w.created_at, w.id",
    )
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(OperatorWorkspace {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                time_zone: row.try_get("time_zone")?,
                created_at: row.try_get("created_at")?,
                active_members: row.try_get("active_members")?,
                disabled_members: row.try_get("disabled_members")?,
                owners: row.try_get("owners")?,
                disabled_owners: row.try_get("disabled_owners")?,
            })
        })
        .collect()
}

/// Ends the Workspace membership. An owner may remove anyone but themself; an admin only plain members.
pub async fn remove_member(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    target: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let actor_role = require_manager(&mut tx, workspace_id, actor).await?;
    not_self(actor, target)?;
    let owners = lock_active_owners(&mut tx, workspace_id).await?;
    let member = locked_active_member(&mut tx, workspace_id, target).await?;
    if actor_role != "OWNER" && member.role != "MEMBER" {
        return Err(CoreError::forbidden(
            "only a workspace owner removes an owner or admin",
        ));
    }
    if member.role == "OWNER" && other_owners(&owners, target) == 0 {
        return Err(CoreError::conflict(
            "last_owner",
            "a workspace needs at least one owner",
        ));
    }
    // Asked while the person still counts as a manager.
    let channels_without_manager: Vec<Uuid> =
        crate::conversation::recovery::offboarding_impact_in(&mut tx, workspace_id, target)
            .await?
            .into_iter()
            .map(|c| c.channel_id)
            .collect();
    sqlx::query(
        "UPDATE penfii.workspace_members SET state = 'INACTIVE', deactivated_at = now()
         WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(target)
    .execute(&mut *tx)
    .await?;
    // Every channel membership goes with them, private and public, in the same transaction; what is
    // kept per person (read positions, mutes, ...) stays. The private channels left without an active
    // manager are named in the audit entry; the removal is not refused because of them.
    let cleared =
        crate::conversation::recovery::clear_memberships(&mut tx, workspace_id, target).await?;
    for membership in &cleared {
        crate::events::emit_simple(
            &mut tx,
            workspace_id,
            "CHANNEL",
            membership.channel_id,
            "channel.member_removed",
            Some(actor),
            json!({"userId": target, "reason": "member_removed"}),
        )
        .await?;
    }
    let cleared_private = cleared.iter().filter(|m| m.private).count();
    // Their pending sign-in invitations for this Workspace stay; an unused invitation is managed in its own list.
    audit(
        &mut tx,
        workspace_id,
        Some(actor),
        "member.removed",
        "USER",
        target,
        json!({
            "userId": target,
            "role": member.role,
            "clearedChannels": {"private": cleared_private, "public": cleared.len() - cleared_private},
            "channelsWithoutManager": channels_without_manager,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Owner only. Disabling ends every session, OAuth grant and token of the account at once and blocks
/// sign-in; enabling lets the person sign in again (their Workspace membership is separate).
/// The account must belong to this Workspace and to no other active Workspace, because the switch
/// is instance-wide.
pub async fn set_account_disabled(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    target: Uuid,
    disabled: bool,
) -> Result<bool, CoreError> {
    let mut tx = pool.begin().await?;
    let actor_role = require_manager(&mut tx, workspace_id, actor).await?;
    require_owner(&actor_role)?;
    not_self(actor, target)?;
    let in_workspace: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.workspace_members
                        WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(workspace_id)
    .bind(target)
    .fetch_one(&mut *tx)
    .await?;
    if !in_workspace {
        return Err(CoreError::NotFound);
    }
    let other_workspace: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.workspace_members
                        WHERE user_id = $1 AND workspace_id <> $2 AND state = 'ACTIVE')",
    )
    .bind(target)
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    if other_workspace {
        return Err(CoreError::conflict(
            "account_in_other_workspace",
            "the account is also active in another workspace; remove the membership instead",
        ));
    }
    let current: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT disabled_at FROM penfii.users WHERE id = $1 FOR UPDATE")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
    if current.is_some() == disabled {
        return Ok(false);
    }
    if disabled {
        sqlx::query("UPDATE penfii.users SET disabled_at = now() WHERE id = $1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
        end_all_access(&mut tx, target, "account_disabled").await?;
    } else {
        sqlx::query("UPDATE penfii.users SET disabled_at = NULL WHERE id = $1")
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    audit(
        &mut tx,
        workspace_id,
        Some(actor),
        if disabled {
            "user.disabled"
        } else {
            "user.enabled"
        },
        "USER",
        target,
        json!({"userId": target}),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// `admin-cli user-disable` / `user-enable`: the operator's switch for an account, used when no owner
/// can act (for example the only owner is locked out). Same effect as the owner's switch, logged
/// without an actor in every Workspace the account belongs to.
pub async fn operator_set_account_disabled(
    pool: &PgPool,
    email: &str,
    disabled: bool,
) -> Result<OperatorAccountChange, CoreError> {
    let (_, email_key) = crate::identity::normalize_email(email)?;
    let mut tx = pool.begin().await?;
    let user_id: Uuid =
        sqlx::query_scalar("SELECT id FROM penfii.users WHERE email_key = $1 FOR UPDATE")
            .bind(&email_key)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    // What disabling leaves without an admin, asked while the person still counts. It only informs.
    let mut impacts = Vec::new();
    if disabled {
        let already: bool =
            sqlx::query_scalar("SELECT disabled_at IS NOT NULL FROM penfii.users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        let workspaces: Vec<(Uuid, String)> = if already {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT w.id, w.name FROM penfii.workspace_members m
                 JOIN penfii.workspaces w ON w.id = m.workspace_id
                 WHERE m.user_id = $1 AND m.state = 'ACTIVE' ORDER BY w.created_at, w.id",
            )
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?
        };
        for (workspace_id, workspace_name) in workspaces {
            let projects =
                crate::project_access::offboarding_impact_in(&mut tx, workspace_id, user_id)
                    .await?;
            let channels = crate::conversation::recovery::offboarding_impact_in(
                &mut tx,
                workspace_id,
                user_id,
            )
            .await?;
            if !projects.is_empty() || !channels.is_empty() {
                impacts.push(WorkspaceImpact {
                    workspace_id,
                    workspace_name,
                    projects,
                    channels,
                });
            }
        }
    }
    if disabled {
        sqlx::query(
            "UPDATE penfii.users SET disabled_at = COALESCE(disabled_at, now()) WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        end_all_access(&mut tx, user_id, "account_disabled").await?;
    } else {
        sqlx::query("UPDATE penfii.users SET disabled_at = NULL WHERE id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }
    let workspaces: Vec<Uuid> =
        sqlx::query_scalar("SELECT workspace_id FROM penfii.workspace_members WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&mut *tx)
            .await?;
    for workspace_id in workspaces {
        audit(
            &mut tx,
            workspace_id,
            None,
            if disabled {
                "user.disabled"
            } else {
                "user.enabled"
            },
            "USER",
            user_id,
            json!({"userId": user_id, "via": "admin-cli"}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(OperatorAccountChange { user_id, impacts })
}

/// What the operator's switch did, and what disabling left without an admin (nothing is refused).
#[derive(Debug)]
pub struct OperatorAccountChange {
    pub user_id: Uuid,
    pub impacts: Vec<WorkspaceImpact>,
}

/// The Projects and private channels of one Workspace left without their last active direct admin
/// or manager, or without any admin.
#[derive(Debug)]
pub struct WorkspaceImpact {
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub projects: Vec<crate::project_access::ProjectImpact>,
    /// The private channels left without an active manager.
    pub channels: Vec<crate::conversation::recovery::ChannelImpact>,
}

/// Revokes every session, OAuth grant and token of the user.
pub(crate) async fn end_all_access(
    conn: &mut PgConnection,
    user_id: Uuid,
    reason: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE penfii.oauth_tokens SET revoked_at = now()
         WHERE revoked_at IS NULL
           AND grant_id IN (SELECT id FROM penfii.oauth_grants WHERE user_id = $1)",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE penfii.oauth_grants SET revoked_at = now(), revoked_reason = $2
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn list_invitations(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<InvitationSummary>, CoreError> {
    let mut conn = pool.acquire().await?;
    require_manager(&mut conn, workspace_id, actor).await?;
    let rows = sqlx::query(
        "SELECT i.id, i.email_key, i.role, i.expires_at, i.created_at, i.created_by,
                u.display_name AS created_by_name, i.accepted_at, i.revoked_at,
                i.team_id AS invited_team_id, i.email_status, i.email_attempted_at, i.email_error,
                COALESCE(t.name, i.team_name) AS invited_team_name,
                CASE WHEN i.team_id IS NULL AND i.team_name IS NULL THEN NULL
                     WHEN t.id IS NULL THEN 'GONE'
                     WHEN t.archived_at IS NOT NULL THEN 'ARCHIVED'
                     ELSE 'ACTIVE' END AS invited_team_status,
                CASE WHEN i.accepted_at IS NOT NULL THEN 'ACCEPTED'
                     -- Closed at or after its expiry (a new invitation took its place): it ran out.
                     WHEN i.revoked_at IS NOT NULL AND i.revoked_at >= i.expires_at THEN 'EXPIRED'
                     WHEN i.revoked_at IS NOT NULL THEN 'REVOKED'
                     WHEN i.expires_at <= now() THEN 'EXPIRED'
                     ELSE 'PENDING' END AS status
         FROM penfii.workspace_invitations i
         LEFT JOIN penfii.users u ON u.id = i.created_by
         LEFT JOIN penfii.teams t ON t.id = i.team_id
         WHERE i.workspace_id = $1
         ORDER BY i.created_at DESC, i.id DESC
         LIMIT 500",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(InvitationSummary {
                id: row.try_get("id")?,
                email: row.try_get("email_key")?,
                role: row.try_get("role")?,
                status: row.try_get("status")?,
                expires_at: row.try_get("expires_at")?,
                created_at: row.try_get("created_at")?,
                created_by: row.try_get("created_by")?,
                created_by_name: row.try_get("created_by_name")?,
                accepted_at: row.try_get("accepted_at")?,
                revoked_at: row.try_get("revoked_at")?,
                team_id: row.try_get("invited_team_id")?,
                team_name: row.try_get("invited_team_name")?,
                team_status: row.try_get("invited_team_status")?,
                email_status: row.try_get("email_status")?,
                email_attempted_at: row.try_get("email_attempted_at")?,
                email_error: row.try_get("email_error")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// Revokes a pending invitation: its link stops working at once. An owner revokes any; an admin only
/// invitations for plain members. An accepted, expired or already revoked invitation is a conflict.
pub async fn revoke_invitation(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    invitation_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let actor_role = require_manager(&mut tx, workspace_id, actor).await?;
    let row = sqlx::query(
        "SELECT email_key, role, accepted_at, revoked_at, expires_at <= now() AS expired
         FROM penfii.workspace_invitations
         WHERE id = $1 AND workspace_id = $2 FOR UPDATE",
    )
    .bind(invitation_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let role: String = row.try_get("role")?;
    if actor_role != "OWNER" && role != "MEMBER" {
        return Err(CoreError::forbidden(
            "only a workspace owner revokes an invitation for an owner or admin",
        ));
    }
    let accepted: Option<DateTime<Utc>> = row.try_get("accepted_at")?;
    let revoked: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
    let expired: bool = row.try_get("expired")?;
    if accepted.is_some() || revoked.is_some() || expired {
        return Err(CoreError::conflict(
            "invitation_not_pending",
            "only a pending invitation can be revoked",
        ));
    }
    sqlx::query("UPDATE penfii.workspace_invitations SET revoked_at = now() WHERE id = $1")
        .bind(invitation_id)
        .execute(&mut *tx)
        .await?;
    let email: String = row.try_get("email_key")?;
    audit(
        &mut tx,
        workspace_id,
        Some(actor),
        "invitation.revoked",
        "INVITATION",
        invitation_id,
        json!({"email": email, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The Team exists in the Workspace (archived or not); archived Teams stay readable. Returns whether it is archived.
pub(crate) async fn team_in_workspace(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    team_id: Uuid,
) -> Result<bool, CoreError> {
    let archived: Option<bool> = sqlx::query_scalar(
        "SELECT archived_at IS NOT NULL FROM penfii.teams WHERE id = $1 AND workspace_id = $2",
    )
    .bind(team_id)
    .bind(workspace_id)
    .fetch_optional(&mut *conn)
    .await?;
    archived.ok_or(CoreError::NotFound)
}

/// Writes on a Team (members, roles, information) stop while it is archived.
pub(crate) async fn team_open_for_writes(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    team_id: Uuid,
) -> Result<(), CoreError> {
    if team_in_workspace(conn, workspace_id, team_id).await? {
        return Err(CoreError::conflict(
            "team_archived",
            "the Team is archived; restore it to change it",
        ));
    }
    Ok(())
}

/// Who may change a Team: a Workspace owner or admin, or a `MANAGER` of that Team. A Workspace member who
/// is neither gets 403, a non-member "not found". Reading the roster is separate and open to every member.
pub(crate) async fn require_team_manager(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    team_id: Uuid,
    actor: Uuid,
) -> Result<(), CoreError> {
    let role = access::require_member(conn, workspace_id, actor).await?;
    team_open_for_writes(conn, workspace_id, team_id).await?;
    if role == "OWNER" || role == "ADMIN" {
        return Ok(());
    }
    let manages: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.team_members
                        WHERE team_id = $1 AND user_id = $2 AND role = 'MANAGER')",
    )
    .bind(team_id)
    .bind(actor)
    .fetch_one(&mut *conn)
    .await?;
    if manages {
        Ok(())
    } else {
        Err(CoreError::forbidden("team manager required"))
    }
}

pub(crate) fn team_role(role: &str) -> Result<&str, CoreError> {
    match role {
        "MANAGER" | "MEMBER" => Ok(role),
        _ => Err(CoreError::validation("role must be MANAGER or MEMBER")),
    }
}

pub async fn list_team_members(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
) -> Result<Vec<TeamMember>, CoreError> {
    let mut conn = pool.acquire().await?;
    // The roster is visible to every member. Changing it still requires a manager (see add/remove).
    access::require_member(&mut conn, workspace_id, actor).await?;
    let _ = team_in_workspace(&mut conn, workspace_id, team_id).await?;
    let rows = sqlx::query(
        "SELECT tm.user_id, u.display_name, u.email, tm.role
         FROM penfii.team_members tm
         JOIN penfii.users u ON u.id = tm.user_id
         JOIN penfii.workspace_members m
           ON m.workspace_id = $2 AND m.user_id = tm.user_id AND m.state = 'ACTIVE'
         WHERE tm.team_id = $1
         ORDER BY lower(u.display_name), tm.user_id",
    )
    .bind(team_id)
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(TeamMember {
                user_id: row.try_get("user_id")?,
                display_name: row.try_get("display_name")?,
                email: row.try_get("email")?,
                role: row.try_get("role")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
}

/// A Team keeps at least one active `MANAGER`: the last one cannot be demoted or removed until another exists.
/// Locks the Team row so two managers cannot take each other out at the same time.
async fn require_other_manager(
    conn: &mut PgConnection,
    team_id: Uuid,
    leaving: Uuid,
) -> Result<(), CoreError> {
    let others: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.team_members tm
                        JOIN penfii.teams t ON t.id = tm.team_id
                        JOIN penfii.workspace_members wm
                          ON wm.workspace_id = t.workspace_id AND wm.user_id = tm.user_id AND wm.state = 'ACTIVE'
                        JOIN penfii.users u ON u.id = tm.user_id AND u.disabled_at IS NULL
                        WHERE tm.team_id = $1 AND tm.role = 'MANAGER' AND tm.user_id <> $2)",
    )
    .bind(team_id)
    .bind(leaving)
    .fetch_one(&mut *conn)
    .await?;
    if others {
        Ok(())
    } else {
        Err(CoreError::conflict(
            "last_team_manager",
            "a Team keeps at least one manager",
        ))
    }
}

async fn lock_team(conn: &mut PgConnection, team_id: Uuid) -> Result<(), CoreError> {
    sqlx::query("SELECT 1 FROM penfii.teams WHERE id = $1 FOR UPDATE")
        .bind(team_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(())
}

/// Puts an active Workspace member in a Team with the given role, or changes the role of someone already
/// in it. Team membership changes who reaches the Projects that give the Team a participation role.
pub async fn add_team_member(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<bool, CoreError> {
    let role = team_role(role)?;
    let mut tx = pool.begin().await?;
    require_team_manager(&mut tx, workspace_id, team_id, actor).await?;
    locked_active_member(&mut tx, workspace_id, user_id).await?;
    lock_team(&mut tx, team_id).await?;
    let before: Option<String> = sqlx::query_scalar(
        "SELECT role FROM penfii.team_members WHERE team_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if before.as_deref() == Some(role) {
        tx.commit().await?;
        return Ok(false);
    }
    if before.as_deref() == Some("MANAGER") {
        require_other_manager(&mut tx, team_id, user_id).await?;
    }
    sqlx::query(
        "INSERT INTO penfii.team_members (team_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (team_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        workspace_id,
        Some(actor),
        if before.is_none() {
            "team.member_added"
        } else {
            "team.member_role_changed"
        },
        "TEAM",
        team_id,
        json!({"userId": user_id, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(before.is_none())
}

pub async fn remove_team_member(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<crate::platform::ChangeNotice>, CoreError> {
    let mut tx = pool.begin().await?;
    require_team_manager(&mut tx, workspace_id, team_id, actor).await?;
    lock_team(&mut tx, team_id).await?;
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM penfii.team_members WHERE team_id = $1 AND user_id = $2",
    )
    .bind(team_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if role.as_deref() == Some("MANAGER") {
        require_other_manager(&mut tx, team_id, user_id).await?;
    }
    let removed =
        sqlx::query("DELETE FROM penfii.team_members WHERE team_id = $1 AND user_id = $2")
            .bind(team_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
    let mut notices = Vec::new();
    if removed {
        // The Projects this Team gave the person a way into (also those where they hold no row yet: an
        // addition racing this change must wait for it), and the channels narrowed to the Team, may have
        // lost them. The Projects are locked one by one in a fixed order.
        let projects: Vec<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM (
                 SELECT ch.project_id FROM penfii.channels ch
                 JOIN penfii.channel_members cm ON cm.channel_id = ch.id AND cm.user_id = $2
                 JOIN penfii.conversations c ON c.id = ch.id
                 WHERE c.workspace_id = $1 AND ch.project_id IS NOT NULL
                 UNION
                 SELECT project_id FROM penfii.project_teams WHERE team_id = $3
             ) p ORDER BY project_id",
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(team_id)
        .fetch_all(&mut *tx)
        .await?;
        for project_id in projects {
            crate::conversation::recovery::lock_project_channels(&mut tx, project_id).await?;
        }
        let lost = crate::conversation::recovery::clear_lost_participation(
            &mut tx,
            workspace_id,
            None,
            Some(user_id),
            actor,
        )
        .await?;
        audit(
            &mut tx,
            workspace_id,
            Some(actor),
            "team.member_removed",
            "TEAM",
            team_id,
            lost.audit_with(json!({"userId": user_id})),
        )
        .await?;
        notices = lost.notices();
    }
    tx.commit().await?;
    Ok(notices)
}

/// The audit log of one Workspace, newest first. Owners, admins and the Workspace's security
/// auditors read it; `action` filters by exact code or by a prefix ending in a dot (`member.`).
pub async fn list_audit(
    pool: &PgPool,
    actor: Uuid,
    workspace_id: Uuid,
    action: Option<&str>,
    limit: Option<i64>,
    cursor: Option<&str>,
) -> Result<AuditPage, CoreError> {
    let mut conn = pool.acquire().await?;
    let role = access::require_member(&mut conn, workspace_id, actor).await?;
    if role != "OWNER" && role != "ADMIN" {
        let auditor: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM penfii.workspace_security_auditors
                            WHERE workspace_id = $1 AND user_id = $2)",
        )
        .bind(workspace_id)
        .bind(actor)
        .fetch_one(&mut *conn)
        .await?;
        if !auditor {
            return Err(CoreError::forbidden(
                "workspace owner, admin or security auditor required",
            ));
        }
    }
    let limit = paging::clamp_limit(limit, AUDIT_PAGE_DEFAULT, AUDIT_PAGE_MAX);
    let after = cursor.map(paging::decode_cursor).transpose()?;
    let (exact, prefix) = match action {
        Some(code) if code.ends_with('.') => (None, Some(format!("{}%", escape_like(code)))),
        Some(code) if !code.is_empty() => (Some(code.to_string()), None),
        _ => (None, None),
    };
    let rows = sqlx::query(
        "SELECT a.id, a.occurred_at, a.actor_user_id, u.display_name AS actor_name, a.action_code,
                a.outcome_code, a.target_kind, a.target_id, a.data,
                CASE upper(a.target_kind)
                    WHEN 'USER' THEN (SELECT x.display_name FROM penfii.users x WHERE x.id = a.target_id)
                    WHEN 'TEAM' THEN (SELECT x.name FROM penfii.teams x WHERE x.id = a.target_id)
                    WHEN 'PROJECT' THEN (SELECT x.name FROM penfii.projects x WHERE x.id = a.target_id)
                    WHEN 'TICKET' THEN (SELECT x.public_key FROM penfii.tickets x WHERE x.id = a.target_id)
                    WHEN 'INVITATION' THEN a.data->>'email'
                END AS target_label,
                CASE WHEN upper(a.target_kind) <> 'USER'
                          AND a.data->>'userId' ~ '^[0-9a-fA-F-]{36}$'
                     THEN (SELECT x.display_name FROM penfii.users x WHERE x.id = (a.data->>'userId')::uuid)
                END AS subject_label
         FROM penfii.audit_entries a
         LEFT JOIN penfii.users u ON u.id = a.actor_user_id
         WHERE a.workspace_id = $1
           AND ($2::text IS NULL OR a.action_code = $2)
           AND ($3::text IS NULL OR a.action_code LIKE $3)
           AND ($4::timestamptz IS NULL OR (a.occurred_at, a.id) < ($4, $5))
         ORDER BY a.occurred_at DESC, a.id DESC
         LIMIT $6",
    )
    .bind(workspace_id)
    .bind(exact)
    .bind(prefix)
    .bind(after.map(|(at, _)| at))
    .bind(after.map(|(_, id)| id).unwrap_or_else(Uuid::nil))
    .bind(limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let mut items = rows
        .iter()
        .map(|row| {
            Ok(AuditRecord {
                id: row.try_get("id")?,
                occurred_at: row.try_get("occurred_at")?,
                actor_user_id: row.try_get("actor_user_id")?,
                actor_name: row.try_get("actor_name")?,
                action: row.try_get("action_code")?,
                outcome: row.try_get("outcome_code")?,
                target_kind: row.try_get("target_kind")?,
                target_id: row.try_get("target_id")?,
                target_label: row.try_get("target_label")?,
                subject_label: row.try_get("subject_label")?,
                data: row.try_get("data")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    let next_cursor = if items.len() as i64 > limit {
        items.truncate(limit as usize);
        items
            .last()
            .map(|last| paging::encode_cursor(last.occurred_at, last.id))
    } else {
        None
    };
    Ok(AuditPage { items, next_cursor })
}

fn escape_like(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Whether the e-mail already belongs to an active member of the Workspace.
pub(crate) async fn is_active_member_email(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    email_key: &str,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM penfii.workspace_members m
                        JOIN penfii.users u ON u.id = m.user_id
                        WHERE m.workspace_id = $1 AND m.state = 'ACTIVE' AND u.email_key = $2)",
    )
    .bind(workspace_id)
    .bind(email_key)
    .fetch_one(&mut *conn)
    .await?)
}
