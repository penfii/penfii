//! Departments are a display tree. They do not grant workspace, team, or project access.

use penfii_contracts::{Department, Team};
use sqlx::PgPool;
use uuid::Uuid;

use crate::access;
use crate::admin;
use crate::error::CoreError;
use crate::identity::require_label;

use super::{TeamRow, team_from};

const MAX_DEPTH: i32 = 8;

#[derive(sqlx::FromRow)]
struct DepartmentRow {
    id: Uuid,
    workspace_id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    position: i32,
}

fn department_name(value: &str) -> Result<String, CoreError> {
    let name = require_label(value, "name")?;
    if name.chars().count() > 80 {
        return Err(CoreError::validation("name is invalid"));
    }
    Ok(name)
}

fn from_row(row: DepartmentRow) -> Department {
    Department {
        id: row.id,
        workspace_id: row.workspace_id,
        parent_id: row.parent_id,
        name: row.name,
        position: row.position,
    }
}

pub async fn list_departments(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
) -> Result<Vec<Department>, CoreError> {
    let mut conn = pool.acquire().await?;
    access::require_member(&mut conn, workspace_id, actor_id).await?;
    let rows = sqlx::query_as::<_, DepartmentRow>(
        "SELECT id, workspace_id, parent_id, name, position
         FROM penfii.departments
         WHERE workspace_id = $1
         ORDER BY position, name, id",
    )
    .bind(workspace_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(from_row).collect())
}

pub async fn create_department(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    name: &str,
    parent_id: Option<Uuid>,
) -> Result<Department, CoreError> {
    let name = department_name(name)?;
    let mut tx = pool.begin().await?;
    admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    if let Some(parent) = parent_id {
        ensure_parent(&mut tx, workspace_id, parent, None).await?;
    }
    let position: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), -1) + 1
         FROM penfii.departments
         WHERE workspace_id = $1 AND parent_id IS NOT DISTINCT FROM $2",
    )
    .bind(workspace_id)
    .bind(parent_id)
    .fetch_one(&mut *tx)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO penfii.departments (id, workspace_id, parent_id, name, position)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(parent_id)
    .bind(&name)
    .bind(position)
    .execute(&mut *tx)
    .await?;
    crate::events::emit_simple(
        &mut tx,
        workspace_id,
        "WORKSPACE",
        workspace_id,
        "department.created",
        Some(actor_id),
        serde_json::json!({ "departmentId": id }),
    )
    .await?;
    admin::audit(
        &mut tx,
        workspace_id,
        Some(actor_id),
        "department.created",
        "DEPARTMENT",
        id,
        serde_json::json!({ "name": name, "parentId": parent_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Department {
        id,
        workspace_id,
        parent_id,
        name,
        position,
    })
}

pub async fn update_department(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    department_id: Uuid,
    name: &str,
    parent_id: Option<Uuid>,
) -> Result<Department, CoreError> {
    let name = department_name(name)?;
    let mut tx = pool.begin().await?;
    admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    let current = load(&mut tx, workspace_id, department_id).await?;
    if parent_id != current.parent_id
        && let Some(parent) = parent_id
    {
        ensure_parent(&mut tx, workspace_id, parent, Some(department_id)).await?;
    }
    let position = if parent_id == current.parent_id {
        current.position
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(MAX(position), -1) + 1
             FROM penfii.departments
             WHERE workspace_id = $1 AND parent_id IS NOT DISTINCT FROM $2 AND id <> $3",
        )
        .bind(workspace_id)
        .bind(parent_id)
        .bind(department_id)
        .fetch_one(&mut *tx)
        .await?
    };
    sqlx::query(
        "UPDATE penfii.departments SET name = $3, parent_id = $4, position = $5 WHERE id = $1 AND workspace_id = $2",
    )
    .bind(department_id)
    .bind(workspace_id)
    .bind(&name)
    .bind(parent_id)
    .bind(position)
    .execute(&mut *tx)
    .await?;
    crate::events::emit_simple(
        &mut tx,
        workspace_id,
        "WORKSPACE",
        workspace_id,
        "department.updated",
        Some(actor_id),
        serde_json::json!({ "departmentId": department_id }),
    )
    .await?;
    if name != current.name || parent_id != current.parent_id {
        admin::audit(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "department.updated",
            "DEPARTMENT",
            department_id,
            serde_json::json!({
                "name": {"before": current.name, "after": name},
                "parentId": {"before": current.parent_id, "after": parent_id},
            }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Department {
        id: department_id,
        workspace_id,
        parent_id,
        name,
        position,
    })
}

pub async fn delete_department(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    department_id: Uuid,
) -> Result<(), CoreError> {
    let mut tx = pool.begin().await?;
    admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    let current = load(&mut tx, workspace_id, department_id).await?;
    // What sits in the department moves up one level, so deleting it loses no Team, member or Project access.
    // Archived Teams are lifted too: the department they sat in no longer exists.
    let lifted_departments = sqlx::query(
        "UPDATE penfii.departments SET parent_id = $3 WHERE parent_id = $1 AND workspace_id = $2",
    )
    .bind(department_id)
    .bind(workspace_id)
    .bind(current.parent_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let lifted_teams = sqlx::query(
        "UPDATE penfii.teams SET department_id = $3 WHERE department_id = $1 AND workspace_id = $2",
    )
    .bind(department_id)
    .bind(workspace_id)
    .bind(current.parent_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM penfii.departments WHERE id = $1 AND workspace_id = $2")
        .bind(department_id)
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;
    crate::events::emit_simple(
        &mut tx,
        workspace_id,
        "WORKSPACE",
        workspace_id,
        "department.deleted",
        Some(actor_id),
        serde_json::json!({
            "departmentId": department_id,
            "parentId": current.parent_id,
            "liftedDepartments": lifted_departments,
            "liftedTeams": lifted_teams,
        }),
    )
    .await?;
    admin::audit(
        &mut tx,
        workspace_id,
        Some(actor_id),
        "department.deleted",
        "DEPARTMENT",
        department_id,
        serde_json::json!({
            "name": current.name,
            "parentId": current.parent_id,
            "liftedDepartments": lifted_departments,
            "liftedTeams": lifted_teams,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Puts a team under a department, or clears that placement. Does not change who can open a project.
pub async fn place_team(
    pool: &PgPool,
    actor_id: Uuid,
    workspace_id: Uuid,
    team_id: Uuid,
    department_id: Option<Uuid>,
) -> Result<Team, CoreError> {
    let mut tx = pool.begin().await?;
    admin::require_manager(&mut tx, workspace_id, actor_id).await?;
    if let Some(department) = department_id {
        let _ = load(&mut tx, workspace_id, department).await?;
    }
    // An archived Team keeps its department; placement is not changed while it is archived.
    crate::admin::team_open_for_writes(&mut tx, workspace_id, team_id).await?;
    let before: Option<Uuid> = sqlx::query_scalar(
        "SELECT department_id FROM penfii.teams WHERE id = $1 AND workspace_id = $2",
    )
    .bind(team_id)
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE penfii.teams SET department_id = $3 WHERE id = $1 AND workspace_id = $2")
        .bind(team_id)
        .bind(workspace_id)
        .bind(department_id)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, TeamRow>(
        "SELECT id, workspace_id, name, archived_at, description, department_id FROM penfii.teams WHERE id = $1",
    )
    .bind(team_id)
    .fetch_one(&mut *tx)
    .await?;
    if before != department_id {
        admin::audit(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "team.placed",
            "TEAM",
            team_id,
            serde_json::json!({ "departmentId": {"before": before, "after": department_id} }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(team_from(row))
}

async fn load(
    conn: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    department_id: Uuid,
) -> Result<DepartmentRow, CoreError> {
    sqlx::query_as::<_, DepartmentRow>(
        "SELECT id, workspace_id, parent_id, name, position
         FROM penfii.departments
         WHERE id = $1 AND workspace_id = $2",
    )
    .bind(department_id)
    .bind(workspace_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)
}

/// The parent must be in this workspace, must not sit inside the moved department, and must leave the tree within `MAX_DEPTH`.
async fn ensure_parent(
    conn: &mut sqlx::PgConnection,
    workspace_id: Uuid,
    parent_id: Uuid,
    moving: Option<Uuid>,
) -> Result<(), CoreError> {
    let parent = load(conn, workspace_id, parent_id).await?;
    if let Some(moving_id) = moving {
        let inside: bool = sqlx::query_scalar(
            "WITH RECURSIVE down AS (
                 SELECT id FROM penfii.departments WHERE id = $1
                 UNION ALL
                 SELECT child.id FROM penfii.departments child JOIN down ON child.parent_id = down.id
             )
             SELECT EXISTS(SELECT 1 FROM down WHERE id = $2)",
        )
        .bind(moving_id)
        .bind(parent.id)
        .fetch_one(&mut *conn)
        .await?;
        if inside {
            return Err(CoreError::rule(
                "department_cycle",
                "a department cannot sit inside itself",
            ));
        }
    }
    let ancestors: i32 = sqlx::query_scalar(
        "WITH RECURSIVE up AS (
             SELECT id, parent_id, 1 AS depth FROM penfii.departments WHERE id = $1
             UNION ALL
             SELECT parent.id, parent.parent_id, up.depth + 1
             FROM penfii.departments parent JOIN up ON parent.id = up.parent_id
         )
         SELECT COALESCE(MAX(depth), 0) FROM up",
    )
    .bind(parent_id)
    .fetch_one(&mut *conn)
    .await?;
    let below: i32 = if let Some(moving_id) = moving {
        sqlx::query_scalar(
            "WITH RECURSIVE down AS (
                 SELECT id, 1 AS depth FROM penfii.departments WHERE id = $1
                 UNION ALL
                 SELECT child.id, down.depth + 1
                 FROM penfii.departments child JOIN down ON child.parent_id = down.id
             )
             SELECT COALESCE(MAX(depth), 1) FROM down",
        )
        .bind(moving_id)
        .fetch_one(&mut *conn)
        .await?
    } else {
        1
    };
    if ancestors + below > MAX_DEPTH {
        return Err(CoreError::rule(
            "department_too_deep",
            "the department tree is too deep",
        ));
    }
    Ok(())
}
