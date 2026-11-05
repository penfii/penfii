//! Who is in a Project and how: direct grants to named people, and the Teams the Project is linked to.
//!
//! A Team link is an organizational relation and gives no one access. A link may carry a participation role;
//! the Team's current members then reach the Project with that role for as long as they are in the Team
//! (`penfii.in_share_scope`, `project_role::project_role`). Nothing here copies Team members into direct grants.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::admin::audit;
use crate::error::CoreError;
use crate::project_role::{project_role, require_project_admin};

fn project_access_role(role: &str) -> Result<&str, CoreError> {
    match role {
        "ADMIN" | "EDITOR" | "VIEWER" => Ok(role),
        _ => Err(CoreError::validation(
            "role must be ADMIN, EDITOR or VIEWER",
        )),
    }
}

/// Locks the Project row and checks the caller administers it; returns its Workspace.
async fn admin_of(
    conn: &mut PgConnection,
    project_id: Uuid,
    actor: Uuid,
) -> Result<Uuid, CoreError> {
    require_project_admin(conn, project_id, actor).await?;
    let (workspace, _) = project_role(conn, project_id, actor).await?;
    sqlx::query("SELECT 1 FROM penfii.projects WHERE id = $1 FOR UPDATE")
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(workspace)
}

/// A Project keeps at least one person who is directly its admin, so it can always be managed.
async fn require_direct_admin_left(
    conn: &mut PgConnection,
    project_id: Uuid,
) -> Result<(), CoreError> {
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.project_user_grants
         WHERE project_id = $1 AND role IN ('ADMIN', 'OWNER')",
    )
    .bind(project_id)
    .fetch_one(&mut *conn)
    .await?;
    if left == 0 {
        return Err(CoreError::conflict(
            "last_project_admin",
            "a Project keeps at least one admin of its own",
        ));
    }
    Ok(())
}

/// Gives a Workspace member a direct role in the Project, or changes the role they have.
pub async fn set_grant(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    user_id: Uuid,
    role: &str,
) -> Result<(), CoreError> {
    let role = project_access_role(role)?;
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.workspace_members
                       WHERE workspace_id = $1 AND user_id = $2 AND state = 'ACTIVE')",
    )
    .bind(workspace)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if !active {
        return Err(CoreError::validation(
            "the person must be an active member of the Workspace",
        ));
    }
    // Designations (waive dependencies, manage pins) belong to a direct editor only.
    sqlx::query(
        "INSERT INTO penfii.project_user_grants (project_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role,
            can_waive_dependencies = project_user_grants.can_waive_dependencies AND EXCLUDED.role = 'EDITOR',
            can_manage_pins = project_user_grants.can_manage_pins AND EXCLUDED.role = 'EDITOR'",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut *tx)
    .await?;
    require_direct_admin_left(&mut tx, project_id).await?;
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.grant_set",
        "PROJECT",
        project_id,
        serde_json::json!({"userId": user_id, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Takes a direct role away. What the person reaches through a Team stays as it is.
pub async fn remove_grant(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    let removed = sqlx::query(
        "DELETE FROM penfii.project_user_grants WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if removed == 0 {
        return Err(CoreError::NotFound);
    }
    require_direct_admin_left(&mut tx, project_id).await?;
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.grant_removed",
        "PROJECT",
        project_id,
        serde_json::json!({"userId": user_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn team_of(conn: &mut PgConnection, workspace: Uuid, team_id: Uuid) -> Result<(), CoreError> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.teams WHERE id = $1 AND workspace_id = $2 AND archived_at IS NULL)",
    )
    .bind(team_id)
    .bind(workspace)
    .fetch_one(&mut *conn)
    .await?;
    if found {
        Ok(())
    } else {
        Err(CoreError::NotFound)
    }
}

/// Links a Team to the Project. Linking again changes nothing; a link never gives access by itself.
pub async fn link_team(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    team_of(&mut tx, workspace, team_id).await?;
    let added = sqlx::query(
        "INSERT INTO penfii.project_teams (project_id, team_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .bind(team_id)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;
    if added {
        audit(
            &mut tx,
            workspace,
            Some(actor),
            "project.team_linked",
            "PROJECT",
            project_id,
            serde_json::json!({"teamId": team_id}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Removes the link, and with it the Team's participation.
pub async fn unlink_team(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    let removed =
        sqlx::query("DELETE FROM penfii.project_teams WHERE project_id = $1 AND team_id = $2")
            .bind(project_id)
            .bind(team_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if removed == 0 {
        return Err(CoreError::NotFound);
    }
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.team_unlinked",
        "PROJECT",
        project_id,
        serde_json::json!({"teamId": team_id}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Lets the members of a linked Team into the Project with `role`, or (`None`) ends the participation and
/// keeps the link. A Team that is not linked yet is refused.
pub async fn set_team_participation(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
    role: Option<&str>,
) -> Result<(), CoreError> {
    let role = role.map(project_access_role).transpose()?;
    let mut tx = pool.begin().await?;
    let workspace = admin_of(&mut tx, project_id, actor).await?;
    // An archived Team keeps its participation and may be lowered, changed or unlinked, but is not
    // given a role it did not have.
    let state: Option<(bool, Option<String>)> = sqlx::query_as(
        "SELECT t.archived_at IS NOT NULL, pt.access_role
         FROM penfii.project_teams pt JOIN penfii.teams t ON t.id = pt.team_id
         WHERE pt.project_id = $1 AND pt.team_id = $2 FOR UPDATE OF pt",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((true, None)) = state
        && role.is_some()
    {
        return Err(CoreError::conflict(
            "team_archived",
            "an archived Team cannot be given a new role",
        ));
    }
    let updated = sqlx::query(
        "UPDATE penfii.project_teams SET access_role = $3 WHERE project_id = $1 AND team_id = $2",
    )
    .bind(project_id)
    .bind(team_id)
    .bind(role)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(CoreError::validation(
            "link the Team to the Project before giving it a role",
        ));
    }
    audit(
        &mut tx,
        workspace,
        Some(actor),
        "project.team_participation_set",
        "PROJECT",
        project_id,
        serde_json::json!({"teamId": team_id, "role": role}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The active members of a Team linked to the Project, for a Project admin choosing people to give a direct
/// role. Only the names are shown, and only for a Team that is linked to this Project.
pub async fn team_roster(
    pool: &PgPool,
    actor: Uuid,
    project_id: Uuid,
    team_id: Uuid,
) -> Result<Vec<penfii_contracts::ProjectMember>, CoreError> {
    let mut conn = pool.acquire().await?;
    require_project_admin(&mut conn, project_id, actor).await?;
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT u.id, u.display_name
         FROM penfii.project_teams pt
         JOIN penfii.teams t ON t.id = pt.team_id
         JOIN penfii.team_members tm ON tm.team_id = pt.team_id
         JOIN penfii.projects p ON p.id = pt.project_id
         JOIN penfii.workspace_members wm
           ON wm.workspace_id = p.workspace_id AND wm.user_id = tm.user_id AND wm.state = 'ACTIVE'
         JOIN penfii.users u ON u.id = tm.user_id
         WHERE pt.project_id = $1 AND pt.team_id = $2 AND t.archived_at IS NULL
         ORDER BY u.display_name, u.id",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_all(&mut *conn)
    .await?;
    let linked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.project_teams WHERE project_id = $1 AND team_id = $2)",
    )
    .bind(project_id)
    .bind(team_id)
    .fetch_one(&mut *conn)
    .await?;
    if !linked {
        return Err(CoreError::NotFound);
    }
    Ok(rows
        .into_iter()
        .map(|(user_id, display_name)| penfii_contracts::ProjectMember {
            user_id,
            display_name,
        })
        .collect())
}
