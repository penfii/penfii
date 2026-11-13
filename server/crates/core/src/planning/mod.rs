//! Planning on top of Tickets: the shared Ticket order, dependencies, saved Views,
//! Sprints and Milestones.
//!
//! Tickets stay the source. Everything here is a view of Tickets or a link between them.
//! Each command runs in one transaction, checks the current permission, and takes the Project
//! row lock when it writes order or graph data.

mod dependencies;
mod derived;
mod milestones;
mod order;
mod sprint_archive;
mod sprints;
mod views;

pub use dependencies::{add_dependency, remove_dependency, set_waiver, ticket_dependencies};
pub(crate) use derived::{attach_planning, open_blockers, visible};
use milestones::{MILESTONE_COLUMNS, milestone_from};
pub use milestones::{
    MilestoneInput, MilestoneUpdate, create_milestone, set_milestones, update_milestone,
};
pub use order::Placement;
pub(crate) use order::{append_order, lock_project, reposition};
pub use sprint_archive::{
    ArchiveCounts, SprintArchive, archive_schedule, cancel_archive, reschedule_archive, run_due,
    run_due_notices, spawn as spawn_archive_scheduler, sprint_project_id,
};
pub use sprints::{
    CloseChoice, SprintCommand, SprintInput, SprintUpdate, assign_sprint, create_sprint,
    sprint_close_preview, sprint_command, sprint_results, update_sprint,
};
pub(crate) use sprints::{SPRINT_COLUMNS, sprint_from};
pub use views::{
    EstimateFilter, ViewDefinition, ViewDisplay, ViewFilter, ViewSort, ViewTarget, create_view,
    delete_view, list_views, update_view,
};

use penfii_contracts::ProjectPlanning;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::CoreError;
use crate::project_role::require_project_role;

fn is_unique(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}

fn version_conflict(current: i64) -> CoreError {
    CoreError::conflict_extra(
        "version_conflict",
        "version conflict",
        serde_json::from_value(json!({"currentVersion":current})).unwrap_or_default(),
    )
}

fn short_name(name: &str, what: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(CoreError::validation(format!(
            "{what} name must be 1..80 characters"
        )));
    }
    Ok(name.to_owned())
}

fn long_text(text: Option<String>, what: &str) -> Result<Option<String>, CoreError> {
    let text = text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    if text.as_ref().is_some_and(|t| t.chars().count() > 500) {
        return Err(CoreError::validation(format!(
            "{what} is limited to 500 characters"
        )));
    }
    Ok(text)
}

pub async fn project_planning(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectPlanning, CoreError> {
    let mut tx = pool.begin().await?;
    let (_, role) = require_project_role(&mut tx, project_id, actor_id, false).await?;
    let sprints = sqlx::query(&format!(
        "SELECT {SPRINT_COLUMNS} FROM penfii.sprints WHERE project_id=$1 ORDER BY starts_on, created_at, id"
    ))
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    let milestones = sqlx::query(&format!(
        "SELECT {MILESTONE_COLUMNS} FROM penfii.milestones WHERE project_id=$1 ORDER BY target_on NULLS LAST, created_at, id"
    ))
    .bind(project_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ProjectPlanning {
        can_manage: crate::project_role::can_manage(&role),
        sprints: sprints.iter().map(sprint_from).collect::<Result<_, _>>()?,
        milestones: milestones
            .iter()
            .map(milestone_from)
            .collect::<Result<_, _>>()?,
    })
}

async fn require_admin(
    tx: &mut PgConnection,
    project_id: Uuid,
    actor_id: Uuid,
) -> Result<Uuid, CoreError> {
    let (workspace, role) = require_project_role(tx, project_id, actor_id, true).await?;
    if role != "ADMIN" {
        return Err(CoreError::forbidden("project admin required"));
    }
    Ok(workspace)
}
