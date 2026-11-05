//! Project workflow states.

use penfii_contracts::WorkflowState;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::changes::record_status_category;
use crate::error::CoreError;
use crate::project_role::{require_project_admin, require_project_role};

#[derive(Debug, Clone)]
pub struct StateInput {
    pub name: String,
    pub category: String,
    pub position: i32,
    pub is_initial: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StateUpdate {
    pub name: Option<String>,
    pub category: Option<String>,
    pub position: Option<i32>,
    pub is_initial: Option<bool>,
    /// `Some(None)` clears the limit.
    pub wip_limit: Option<Option<i32>>,
}

#[derive(sqlx::FromRow)]
pub(super) struct StateRow {
    id: Uuid,
    project_id: Uuid,
    name: String,
    category: String,
    position: i32,
    is_initial: bool,
    wip_limit: Option<i32>,
}

pub(super) fn state(row: StateRow) -> WorkflowState {
    WorkflowState {
        id: row.id,
        project_id: row.project_id,
        name: row.name,
        category: row.category,
        position: row.position,
        is_initial: row.is_initial,
        wip_limit: row.wip_limit,
    }
}

pub async fn workflow(
    pool: &PgPool,
    project_id: Uuid,
    actor_id: Uuid,
) -> Result<(i64, Vec<WorkflowState>), CoreError> {
    let mut tx = pool.begin().await?;
    let _ = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM penfii.workflows WHERE project_id=$1")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    let rows = sqlx::query_as::<_, StateRow>("SELECT id,project_id,name,category,position,is_initial,wip_limit FROM penfii.workflow_statuses WHERE project_id=$1 AND retired_at IS NULL ORDER BY position,id").bind(project_id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok((version, rows.into_iter().map(state).collect()))
}

pub async fn create_state(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    expected_version: i64,
    input: StateInput,
) -> Result<(i64, WorkflowState), CoreError> {
    let name = input.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(CoreError::validation("state name must be 1..80 characters"));
    }
    if !matches!(
        input.category.as_str(),
        "NOT_STARTED" | "IN_PROGRESS" | "DONE" | "CANCELED"
    ) {
        return Err(CoreError::validation("invalid state category"));
    }
    if input.is_initial && input.category != "NOT_STARTED" {
        return Err(CoreError::validation("initial state must be NOT_STARTED"));
    }
    let mut tx = pool.begin().await?;
    require_project_admin(&mut tx, project_id, actor_id).await?;
    let current: i64 =
        sqlx::query_scalar("SELECT version FROM penfii.workflows WHERE project_id=$1 FOR UPDATE")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    if current != expected_version {
        return Err(CoreError::conflict_extra(
            "workflow_version_conflict",
            "workflow version conflict",
            serde_json::from_value(json!({"currentVersion":current})).unwrap_or_default(),
        ));
    }
    if input.is_initial {
        sqlx::query("UPDATE penfii.workflow_statuses SET is_initial=false WHERE project_id=$1 AND is_initial").bind(project_id).execute(&mut *tx).await?;
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO penfii.workflow_statuses(id,project_id,name,category,position,is_initial) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(project_id).bind(name).bind(&input.category).bind(input.position).bind(input.is_initial).execute(&mut *tx).await?;
    record_status_category(&mut tx, id, &input.category).await?;
    let version: i64 = sqlx::query_scalar(
        "UPDATE penfii.workflows SET version=version+1 WHERE project_id=$1 RETURNING version",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        version,
        WorkflowState {
            id,
            project_id,
            name: name.to_owned(),
            category: input.category,
            position: input.position,
            is_initial: input.is_initial,
            wip_limit: None,
        },
    ))
}

pub async fn update_state(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    state_id: Uuid,
    expected_version: i64,
    update: StateUpdate,
) -> Result<(i64, WorkflowState), CoreError> {
    if let Some(name) = update.name.as_deref()
        && (name.trim().is_empty() || name.trim().len() > 80)
    {
        return Err(CoreError::validation("state name must be 1..80 characters"));
    }
    if update
        .category
        .as_deref()
        .is_some_and(|v| !matches!(v, "NOT_STARTED" | "IN_PROGRESS" | "DONE" | "CANCELED"))
    {
        return Err(CoreError::validation("invalid state category"));
    }
    if update
        .wip_limit
        .flatten()
        .is_some_and(|limit| !(1..=1000).contains(&limit))
    {
        return Err(CoreError::validation("WIP limit must be 1..1000 or unset"));
    }
    let mut tx = pool.begin().await?;
    require_project_admin(&mut tx, project_id, actor_id).await?;
    let current: i64 =
        sqlx::query_scalar("SELECT version FROM penfii.workflows WHERE project_id=$1 FOR UPDATE")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    if current != expected_version {
        return Err(CoreError::conflict_extra(
            "workflow_version_conflict",
            "workflow version conflict",
            serde_json::from_value(json!({"currentVersion":current})).unwrap_or_default(),
        ));
    }
    let row=sqlx::query("SELECT id,project_id,name,category,position,is_initial,wip_limit FROM penfii.workflow_statuses WHERE project_id=$1 AND id=$2 AND retired_at IS NULL FOR UPDATE").bind(project_id).bind(state_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let old = WorkflowState {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        category: row.get("category"),
        position: row.get("position"),
        is_initial: row.get("is_initial"),
        wip_limit: row.get("wip_limit"),
    };
    let name = update
        .name
        .map(|v| v.trim().to_owned())
        .unwrap_or(old.name.clone());
    let category = update.category.unwrap_or(old.category.clone());
    let position = update.position.unwrap_or(old.position);
    let initial = update.is_initial.unwrap_or(old.is_initial);
    let wip_limit = update.wip_limit.unwrap_or(old.wip_limit);
    if initial && category != "NOT_STARTED" {
        return Err(CoreError::validation("initial state must be NOT_STARTED"));
    }
    if old.is_initial && !initial {
        return Err(CoreError::conflict(
            "workflow_requires_initial",
            "workflow requires one initial state",
        ));
    }
    if category != old.category {
        let used: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM penfii.tickets WHERE project_id=$1 AND status_id=$2)",
        )
        .bind(project_id)
        .bind(state_id)
        .fetch_one(&mut *tx)
        .await?;
        if used {
            return Err(CoreError::conflict(
                "state_in_use",
                "cannot change category of a state used by tickets",
            ));
        }
    }
    if old.category == "NOT_STARTED" && category != "NOT_STARTED" {
        let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM penfii.workflow_statuses WHERE project_id=$1 AND category='NOT_STARTED' AND retired_at IS NULL AND id<>$2").bind(project_id).bind(state_id).fetch_one(&mut *tx).await?;
        if remaining == 0 {
            return Err(CoreError::conflict(
                "workflow_requires_not_started",
                "workflow requires a NOT_STARTED state",
            ));
        }
    }
    if old.category == "DONE" && category != "DONE" {
        let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM penfii.workflow_statuses WHERE project_id=$1 AND category='DONE' AND retired_at IS NULL AND id<>$2").bind(project_id).bind(state_id).fetch_one(&mut *tx).await?;
        if remaining == 0 {
            return Err(CoreError::conflict(
                "workflow_requires_done",
                "workflow requires a DONE state",
            ));
        }
    }
    if initial {
        sqlx::query("UPDATE penfii.workflow_statuses SET is_initial=false WHERE project_id=$1 AND is_initial AND id<>$2").bind(project_id).bind(state_id).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE penfii.workflow_statuses SET name=$3,category=$4,position=$5,is_initial=$6,wip_limit=$7 WHERE project_id=$1 AND id=$2").bind(project_id).bind(state_id).bind(&name).bind(&category).bind(position).bind(initial).bind(wip_limit).execute(&mut *tx).await?;
    if category != old.category {
        record_status_category(&mut tx, state_id, &category).await?;
    }
    let version: i64 = sqlx::query_scalar(
        "UPDATE penfii.workflows SET version=version+1 WHERE project_id=$1 RETURNING version",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        version,
        WorkflowState {
            id: state_id,
            project_id,
            name,
            category,
            position,
            is_initial: initial,
            wip_limit,
        },
    ))
}

/// Retires a State. It keeps the Workflow valid: not the initial State, not the last
/// NOT_STARTED or DONE State, and no Ticket may still be in it (move them first).
pub async fn delete_state(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    state_id: Uuid,
    expected_version: i64,
) -> Result<i64, CoreError> {
    let mut tx = pool.begin().await?;
    require_project_admin(&mut tx, project_id, actor_id).await?;
    let current: i64 =
        sqlx::query_scalar("SELECT version FROM penfii.workflows WHERE project_id=$1 FOR UPDATE")
            .bind(project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(CoreError::NotFound)?;
    if current != expected_version {
        return Err(CoreError::conflict_extra(
            "workflow_version_conflict",
            "workflow version conflict",
            serde_json::from_value(json!({"currentVersion":current})).unwrap_or_default(),
        ));
    }
    let row = sqlx::query("SELECT category,is_initial FROM penfii.workflow_statuses WHERE project_id=$1 AND id=$2 AND retired_at IS NULL FOR UPDATE")
        .bind(project_id)
        .bind(state_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(CoreError::NotFound)?;
    let category: String = row.try_get("category")?;
    let is_initial: bool = row.try_get("is_initial")?;
    if is_initial {
        return Err(CoreError::conflict(
            "workflow_requires_initial",
            "the initial state cannot be deleted; choose another initial state first",
        ));
    }
    if matches!(category.as_str(), "NOT_STARTED" | "DONE") {
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.workflow_statuses WHERE project_id=$1 AND category=$2 AND retired_at IS NULL AND id<>$3")
            .bind(project_id)
            .bind(&category)
            .bind(state_id)
            .fetch_one(&mut *tx)
            .await?;
        if remaining == 0 {
            let code = if category == "DONE" {
                "workflow_requires_done"
            } else {
                "workflow_requires_not_started"
            };
            return Err(CoreError::conflict(
                code,
                "the workflow needs at least one state of this category",
            ));
        }
    }
    let tickets: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM penfii.tickets WHERE project_id=$1 AND status_id=$2 AND purged_at IS NULL",
    )
    .bind(project_id)
    .bind(state_id)
    .fetch_one(&mut *tx)
    .await?;
    if tickets > 0 {
        return Err(CoreError::conflict_extra(
            "state_in_use",
            "move the tickets out of this state before deleting it",
            serde_json::from_value(json!({"tickets":tickets})).unwrap_or_default(),
        ));
    }
    sqlx::query(
        "DELETE FROM penfii.workflow_transitions WHERE from_status_id=$1 OR to_status_id=$1",
    )
    .bind(state_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE penfii.workflow_statuses SET retired_at=now(),wip_limit=NULL WHERE project_id=$1 AND id=$2")
        .bind(project_id)
        .bind(state_id)
        .execute(&mut *tx)
        .await?;
    let version: i64 = sqlx::query_scalar(
        "UPDATE penfii.workflows SET version=version+1 WHERE project_id=$1 RETURNING version",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(version)
}
