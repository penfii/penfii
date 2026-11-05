//! Who a person is in a Project, and what that lets them do.
//!
//! This is the authorization core for everything Project-shaped (Tickets, Planning, Calendar, Files,
//! Personal pins, Conversations, the Project's own settings): one place that answers "what role does
//! this person hold here", and the small `can_*` / `require_*` rules built on it.
//!
//! A Project's inside is for its participants: a Project grant (direct or by Group) or the participation
//! of a Team the person is in now (the Project gives that Team a role; linking alone gives none). The
//! highest role of all the sources counts. Every other Workspace member can find the Project but has no
//! role in it. Someone who is not an active Workspace member, or cannot see the Project, gets "not found".
//! `penfii.in_share_scope` (migration `0001_initial.sql`) mirrors the "has any source" half of this in SQL.
//!
//! Roles are `ADMIN` > `EDITOR` > `VIEWER`. A direct grant may still be stored as the legacy `OWNER`, which
//! means `ADMIN`: [`normalize`] and [`rank_sql`] are the only places that say so.

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::error::CoreError;

/// The Project role a stored grant role stands for: the legacy `OWNER` is `ADMIN`.
pub(crate) fn normalize(role: &str) -> &str {
    if role == "OWNER" { "ADMIN" } else { role }
}

/// Role name for a rank (3 admin, 2 editor, 1 viewer); `None` for no role.
fn role_of_rank(rank: i32) -> Option<&'static str> {
    match rank {
        3 => Some("ADMIN"),
        2 => Some("EDITOR"),
        1 => Some("VIEWER"),
        _ => None,
    }
}

/// SQL expression: the rank (3, 2, 1) of the role stored in `column`. Same meaning as [`normalize`] and
/// [`role_of_rank`]; any role that is not `ADMIN`, `OWNER` or `EDITOR` counts as a viewer.
fn rank_sql(column: &str) -> String {
    format!(
        "CASE WHEN {column} IN ('ADMIN','OWNER') THEN 3 WHEN {column}='EDITOR' THEN 2 ELSE 1 END"
    )
}

pub(crate) async fn project_role(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(Uuid, Option<String>), CoreError> {
    let (workspace_id, role, _) = project_role_sources(conn, project_id, user_id).await?;
    Ok((workspace_id, role))
}

/// [`project_role`] plus where the role comes from (`DIRECT`, `GROUP`, `TEAM`), for the asking person only.
pub(crate) async fn project_role_sources(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(Uuid, Option<String>, Vec<&'static str>), CoreError> {
    // Each source yields its best rank, or NULL when the person has no role from it.
    let row = sqlx::query(&format!(
        "SELECT p.workspace_id,
          (SELECT {direct} FROM penfii.project_user_grants g
            WHERE g.project_id=p.id AND g.user_id=$2) AS direct_rank,
          (SELECT max({group})
             FROM penfii.project_group_grants g
             JOIN penfii.groups gr ON gr.id=g.group_id AND gr.deleted_at IS NULL
             LEFT JOIN penfii.group_members gm ON gm.group_id=gr.id
             LEFT JOIN penfii.team_members tm ON tm.team_id=gr.system_team_id
             WHERE g.project_id=p.id AND (gm.user_id=$2 OR tm.user_id=$2)) AS group_rank,
          (SELECT max({team})
             FROM penfii.project_teams pt JOIN penfii.team_members tm ON tm.team_id=pt.team_id
             WHERE pt.project_id=p.id AND pt.access_role IS NOT NULL AND tm.user_id=$2) AS team_rank,
          wm.state='ACTIVE' AS active
         FROM penfii.projects p
         LEFT JOIN penfii.workspace_members wm ON wm.workspace_id=p.workspace_id AND wm.user_id=$2
         WHERE p.id=$1",
        direct = rank_sql("g.role"),
        group = rank_sql("g.role"),
        team = rank_sql("pt.access_role"),
    ))
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    let workspace_id: Uuid = row.try_get("workspace_id")?;
    let active: Option<bool> = row.try_get("active")?;
    if active != Some(true) {
        return Err(CoreError::NotFound);
    }
    let ranks = [
        (row.try_get::<Option<i32>, _>("direct_rank")?, "DIRECT"),
        (row.try_get::<Option<i32>, _>("group_rank")?, "GROUP"),
        (row.try_get::<Option<i32>, _>("team_rank")?, "TEAM"),
    ];
    let best = ranks
        .iter()
        .filter_map(|(rank, _)| *rank)
        .max()
        .unwrap_or(0);
    let sources = ranks
        .iter()
        .filter(|(rank, _)| rank.is_some())
        .map(|(_, source)| *source)
        .collect();
    Ok((workspace_id, role_of_rank(best).map(str::to_owned), sources))
}

/// The gate a Ticket's origin channel puts on it: the person may read that channel. It is the same
/// rule as every other channel read ([`crate::access::CHANNEL_READABLE`] plus an active Workspace
/// membership); a Ticket with no origin channel has no gate.
pub(crate) async fn channel_allowed(
    conn: &mut PgConnection,
    channel_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<bool, CoreError> {
    let Some(channel_id) = channel_id else {
        return Ok(true);
    };
    let allowed: Option<bool> = sqlx::query_scalar(&format!(
        "SELECT COALESCE({}, false)
            AND EXISTS(SELECT 1 FROM penfii.workspace_members wm
                       WHERE wm.workspace_id = c.workspace_id AND wm.user_id = $2 AND wm.state = 'ACTIVE')
         FROM penfii.channels ch
         JOIN penfii.conversations c ON c.id = ch.id
         LEFT JOIN penfii.channel_members cm ON cm.channel_id = ch.id AND cm.user_id = $2
         WHERE ch.id = $1 AND c.kind = 'CHANNEL'",
        crate::access::CHANNEL_READABLE
    ))
    .bind(channel_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(allowed.unwrap_or(false))
}

/// What a Project role may do. `Collaborate` is the everyday work (create and edit Tickets, comment,
/// attach, label, move); `Manage` is the Project's own settings, workflow, trash and designations.
/// `Manage` includes `Collaborate`, which includes reading.
pub(crate) fn can_collaborate(role: &str) -> bool {
    matches!(role, "EDITOR" | "ADMIN")
}

pub(crate) fn can_manage(role: &str) -> bool {
    role == "ADMIN"
}

/// A power the Project admin may hand to one editor on their own grant.
#[derive(Clone, Copy)]
pub(crate) enum Designation {
    WaiveDependencies,
    ManagePins,
}

/// Whether the person has the power: a Project admin always, an editor only when designated. A
/// designation never reaches a Ticket the person cannot open; callers check that first.
pub(crate) async fn can_manage_or_designated(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
    role: &str,
    designation: Designation,
) -> Result<bool, CoreError> {
    if can_manage(role) {
        return Ok(true);
    }
    if role != "EDITOR" {
        return Ok(false);
    }
    let column = match designation {
        Designation::WaiveDependencies => "can_waive_dependencies",
        Designation::ManagePins => "can_manage_pins",
    };
    Ok(sqlx::query_scalar(&format!(
        "SELECT coalesce((SELECT {column} FROM penfii.project_user_grants WHERE project_id=$1 AND user_id=$2), false)"
    ))
    .bind(project_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

pub(crate) async fn require_project_role(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
    edit: bool,
) -> Result<(Uuid, String), CoreError> {
    let (workspace, role) = project_role(conn, project_id, user_id).await?;
    let role = role.ok_or(CoreError::NotFound)?;
    if edit && !can_collaborate(&role) {
        return Err(CoreError::forbidden("project editor required"));
    }
    Ok((workspace, role))
}

/// Workflow and State settings belong to the Project admin; everyone else reads them.
pub(crate) async fn require_project_admin(
    conn: &mut PgConnection,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), CoreError> {
    let (_, role) = require_project_role(conn, project_id, user_id, false).await?;
    if !can_manage(&role) {
        return Err(CoreError::forbidden("project admin required"));
    }
    Ok(())
}

pub(crate) async fn require_ticket_editor(
    conn: &mut PgConnection,
    project_id: Uuid,
    channel_id: Option<Uuid>,
    user_id: Uuid,
) -> Result<(), CoreError> {
    let (_, role) = require_project_role(conn, project_id, user_id, false).await?;
    if !channel_allowed(conn, channel_id, user_id).await? {
        return Err(CoreError::NotFound);
    }
    if !can_collaborate(&role) {
        return Err(CoreError::forbidden("project editor required"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_owner_grant_is_an_admin() {
        assert_eq!(normalize("OWNER"), "ADMIN");
        for role in ["ADMIN", "EDITOR", "VIEWER"] {
            assert_eq!(normalize(role), role);
        }
    }

    #[test]
    fn ranks_map_back_to_roles() {
        assert_eq!(role_of_rank(3), Some("ADMIN"));
        assert_eq!(role_of_rank(2), Some("EDITOR"));
        assert_eq!(role_of_rank(1), Some("VIEWER"));
        assert_eq!(role_of_rank(0), None);
    }

    #[test]
    fn what_each_role_may_do() {
        assert!(can_manage("ADMIN") && can_collaborate("ADMIN"));
        assert!(!can_manage("EDITOR") && can_collaborate("EDITOR"));
        assert!(!can_manage("VIEWER") && !can_collaborate("VIEWER"));
    }
}
