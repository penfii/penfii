//! Archiving a completed Sprint's finished work, now or at a time chosen when the Sprint is completed.
//!
//! Only Tasks whose assignment in the Sprint ended `DONE` or `CANCELED` are candidates, and each is
//! judged again at the moment of archiving, under the Task's row lock: one that was reopened, is already
//! archived or trashed, or still has an open child is left alone. Channel membership plays no part: archiving
//! is a Project-level lifecycle power and changes nobody's access to the Ticket. Unfinished work never moves here: the Sprint's own carry-over
//! (`release_assignments`) already sent it to the backlog or the next Sprint.
//!
//! A scheduled archive is one row per Sprint. A polling loop with the same shape as the reminders loop
//! (`SKIP LOCKED`, one transaction per row) runs due rows, so a server that was down at the due time
//! archives afterwards, and two instances never run the same row.

use chrono::{DateTime, Utc};
use penfii_contracts::SprintArchiveSchedule;
use serde_json::json;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use super::{require_admin, version_conflict};
use crate::error::CoreError;
use crate::project_role;
use crate::ticket_lifecycle::{self, ArchiveOutcome};

/// When the finished work of a Sprint being completed is archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprintArchive {
    /// With the completion itself, in the same transaction.
    Immediate,
    /// At this instant, which must be in the future.
    At(DateTime<Utc>),
}

/// How many Tickets an archive run changed and how many candidates it left alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArchiveCounts {
    pub archived: i32,
    pub skipped: i32,
}

pub(crate) fn require_future(at: DateTime<Utc>) -> Result<(), CoreError> {
    if at <= Utc::now() {
        return Err(CoreError::validation(
            "the archive time must be in the future",
        ));
    }
    Ok(())
}

/// Archives the Sprint's finished Tasks (and their finished Subtasks) inside the caller's transaction.
pub(crate) async fn archive_finished(
    tx: &mut PgConnection,
    actor: Uuid,
    sprint_id: Uuid,
    scheduled: bool,
) -> Result<ArchiveCounts, CoreError> {
    let candidates: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT a.ticket_id
         FROM penfii.sprint_assignments a
         WHERE a.sprint_id = $1 AND a.outcome IN ('DONE', 'CANCELED')
         ORDER BY a.ticket_id",
    )
    .bind(sprint_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut counts = ArchiveCounts::default();
    for ticket_id in candidates {
        // Lock the Task before judging it. A status change, a reopen, a trash all take this row lock, so what
        // is decided below cannot change until this transaction ends. (Whether the person can see the Task's
        // channel does not matter: archiving is a Project-level lifecycle step and changes nobody's access.)
        let ticket = match ticket_lifecycle::locked_ticket(&mut *tx, ticket_id).await {
            Ok(ticket) => ticket,
            Err(CoreError::NotFound) => {
                counts.skipped += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        // Judge again now, under the lock: live, still DONE/CANCELED, no open child.
        let eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM penfii.tickets t
                 JOIN penfii.workflow_statuses s ON s.id = t.status_id
                 WHERE t.id = $1 AND t.hierarchy_kind = 'TASK'
                   AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL
                   AND s.category IN ('DONE', 'CANCELED')
                   AND NOT EXISTS(
                       SELECT 1 FROM penfii.tickets c
                       JOIN penfii.workflow_statuses cs ON cs.id = c.status_id
                       WHERE c.parent_id = t.id AND c.archived_at IS NULL AND c.trashed_at IS NULL
                         AND c.purged_at IS NULL AND cs.category NOT IN ('DONE', 'CANCELED')))",
        )
        .bind(ticket_id)
        .fetch_one(&mut *tx)
        .await?;
        if !eligible {
            counts.skipped += 1;
            continue;
        }
        // The finished Subtasks go with their Task, so no live child is left under an archived parent. They are
        // locked here too, so none of them can be reopened between this read and the archive.
        let children: Vec<Uuid> = sqlx::query_scalar(
            "SELECT c.id FROM penfii.tickets c
             JOIN penfii.workflow_statuses cs ON cs.id = c.status_id
             WHERE c.parent_id = $1 AND c.archived_at IS NULL AND c.trashed_at IS NULL
               AND c.purged_at IS NULL AND cs.category IN ('DONE', 'CANCELED')
             ORDER BY c.id
             FOR UPDATE OF c",
        )
        .bind(ticket_id)
        .fetch_all(&mut *tx)
        .await?;
        for child in children {
            match ticket_lifecycle::archive_in_tx(&mut *tx, actor, child, scheduled).await {
                Ok(ArchiveOutcome::Archived) => counts.archived += 1,
                Ok(ArchiveOutcome::AlreadyArchived) | Err(CoreError::Conflict { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        match ticket_lifecycle::archive_locked(&mut *tx, actor, ticket_id, &ticket, scheduled).await
        {
            Ok(ArchiveOutcome::Archived) => counts.archived += 1,
            Ok(ArchiveOutcome::AlreadyArchived) | Err(CoreError::Conflict { .. }) => {
                counts.skipped += 1;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(counts)
}

/// Records the schedule of a Sprint being completed, inside the completion's transaction.
pub(crate) async fn insert_schedule(
    tx: &mut PgConnection,
    workspace_id: Uuid,
    project_id: Uuid,
    sprint_id: Uuid,
    actor: Uuid,
    due_at: DateTime<Utc>,
    time_zone: &str,
) -> Result<(), CoreError> {
    sqlx::query(
        "INSERT INTO penfii.sprint_archive_schedules(sprint_id, workspace_id, project_id, scheduled_by, due_at, time_zone)
         VALUES($1, $2, $3, $4, $5, $6)",
    )
    .bind(sprint_id)
    .bind(workspace_id)
    .bind(project_id)
    .bind(actor)
    .bind(due_at)
    .bind(time_zone)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

fn schedule_from(row: &sqlx::postgres::PgRow) -> Result<SprintArchiveSchedule, CoreError> {
    Ok(SprintArchiveSchedule {
        sprint_id: row.try_get("sprint_id")?,
        due_at: row.try_get("due_at")?,
        time_zone: row.try_get("time_zone")?,
        state: row.try_get("state")?,
        version: row.try_get("version")?,
        archived_count: row.try_get("archived_count")?,
        skipped_count: row.try_get("skipped_count")?,
        finished_at: row.try_get("finished_at")?,
    })
}

const SCHEDULE_COLUMNS: &str =
    "sprint_id,due_at,time_zone,state,version,archived_count,skipped_count,finished_at";

/// The Sprint's archive schedule, for anyone who can read its Project. `None` when none was made.
pub async fn archive_schedule(
    pool: &PgPool,
    actor: Uuid,
    sprint_id: Uuid,
) -> Result<Option<SprintArchiveSchedule>, CoreError> {
    let mut conn = pool.acquire().await?;
    let project_id: Uuid =
        sqlx::query_scalar("SELECT project_id FROM penfii.sprints WHERE id = $1")
            .bind(sprint_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(CoreError::NotFound)?;
    project_role::require_project_role(&mut conn, project_id, actor, false).await?;
    let row = sqlx::query(&format!(
        "SELECT {SCHEDULE_COLUMNS} FROM penfii.sprint_archive_schedules WHERE sprint_id = $1"
    ))
    .bind(sprint_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.as_ref().map(schedule_from).transpose()
}

async fn locked_schedule(
    tx: &mut PgConnection,
    actor: Uuid,
    sprint_id: Uuid,
    expected_version: i64,
) -> Result<sqlx::postgres::PgRow, CoreError> {
    let row = sqlx::query(
        "SELECT project_id, state, version FROM penfii.sprint_archive_schedules WHERE sprint_id = $1 FOR UPDATE",
    )
    .bind(sprint_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    require_admin(tx, row.try_get("project_id")?, actor).await?;
    if row.try_get::<String, _>("state")? != "SCHEDULED" {
        return Err(CoreError::conflict(
            "archive_schedule_finished",
            "the archive already ran or was canceled",
        ));
    }
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    Ok(row)
}

/// Moves a scheduled archive to another future time. Project admin only; only before it runs.
pub async fn reschedule_archive(
    pool: &PgPool,
    actor: Uuid,
    sprint_id: Uuid,
    expected_version: i64,
    due_at: DateTime<Utc>,
) -> Result<SprintArchiveSchedule, CoreError> {
    require_future(due_at)?;
    let mut tx = pool.begin().await?;
    locked_schedule(&mut tx, actor, sprint_id, expected_version).await?;
    let row = sqlx::query(&format!(
        "UPDATE penfii.sprint_archive_schedules SET due_at = $2, scheduled_by = $3, version = version + 1
         WHERE sprint_id = $1 RETURNING {SCHEDULE_COLUMNS}"
    ))
    .bind(sprint_id)
    .bind(due_at)
    .bind(actor)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    schedule_from(&row)
}

/// Cancels a scheduled archive: nothing is archived and the Tickets stay as they are.
pub async fn cancel_archive(
    pool: &PgPool,
    actor: Uuid,
    sprint_id: Uuid,
    expected_version: i64,
) -> Result<SprintArchiveSchedule, CoreError> {
    let mut tx = pool.begin().await?;
    locked_schedule(&mut tx, actor, sprint_id, expected_version).await?;
    let row = sqlx::query(&format!(
        "UPDATE penfii.sprint_archive_schedules SET state = 'CANCELED', version = version + 1, finished_at = now()
         WHERE sprint_id = $1 RETURNING {SCHEDULE_COLUMNS}"
    ))
    .bind(sprint_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    schedule_from(&row)
}

/// Runs the schedules that are due. Each is handled in its own transaction under a row lock
/// (`SKIP LOCKED`). The person who scheduled it must still be a Project admin; otherwise the
/// schedule is canceled rather than archiving on behalf of someone who no longer may. A schedule whose run
/// fails is rolled back, left `SCHEDULED` for the next pass, and skipped for the rest of this one, so one
/// failing Sprint never keeps the others from running. Returns how many schedules were handled.
pub async fn run_due(pool: &PgPool, limit: i64) -> Result<usize, CoreError> {
    Ok(run_due_notices(pool, limit).await?.0)
}

/// [`run_due`], and the notices (one per Project whose Tickets were archived) that tell open boards to look
/// again. The caller publishes them after this returns; every schedule is committed by then.
pub async fn run_due_notices(
    pool: &PgPool,
    limit: i64,
) -> Result<(usize, Vec<crate::ChangeNotice>), CoreError> {
    let mut notices: Vec<crate::ChangeNotice> = Vec::new();
    let mut handled = 0;
    let mut failed: Vec<Uuid> = Vec::new();
    for _ in 0..limit {
        let mut tx = pool.begin().await?;
        let Some(row) = sqlx::query(
            "SELECT sprint_id, project_id, scheduled_by FROM penfii.sprint_archive_schedules
             WHERE state = 'SCHEDULED' AND due_at <= now() AND sprint_id <> ALL($1)
             ORDER BY due_at, sprint_id
             FOR UPDATE SKIP LOCKED
             LIMIT 1",
        )
        .bind(&failed)
        .fetch_optional(&mut *tx)
        .await?
        else {
            break;
        };
        let sprint_id: Uuid = row.try_get("sprint_id")?;
        let project_id: Uuid = row.try_get("project_id")?;
        let by: Uuid = row.try_get("scheduled_by")?;
        let may = project_role::require_project_admin(&mut tx, project_id, by)
            .await
            .is_ok();
        if may {
            // A panic archiving this Sprint is its failure like an error is: the schedule stays SCHEDULED and is
            // skipped for the rest of the pass, so the other Sprints still run.
            let outcome = crate::lifecycle::guarded(
                "sprint archive scheduler",
                archive_finished(&mut tx, by, sprint_id, true),
            )
            .await;
            let counts = match outcome {
                Ok(counts) => counts,
                Err(error) => {
                    tracing::warn!(?error, %sprint_id, "scheduled archive failed; it is retried on the next pass");
                    failed.push(sprint_id);
                    continue;
                }
            };
            sqlx::query(
                "UPDATE penfii.sprint_archive_schedules
                 SET state = 'DONE', version = version + 1, finished_at = now(), archived_count = $2, skipped_count = $3
                 WHERE sprint_id = $1",
            )
            .bind(sprint_id)
            .bind(counts.archived)
            .bind(counts.skipped)
            .execute(&mut *tx)
            .await?;
            if counts.archived > 0 {
                notices.push(crate::ChangeNotice::Project { project_id });
            }
        } else {
            tracing::warn!(%sprint_id, "scheduled archive canceled: the scheduler is no longer a project admin");
            sqlx::query(
                "UPDATE penfii.sprint_archive_schedules
                 SET state = 'CANCELED', version = version + 1, finished_at = now()
                 WHERE sprint_id = $1",
            )
            .bind(sprint_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        handled += 1;
    }
    Ok((handled, notices))
}

/// Runs the due schedules every 15 seconds and publishes the notices on `changes` (none: nobody is
/// listening, as in the admin CLI and tests).
pub fn spawn(
    pool: PgPool,
    changes: Option<tokio::sync::broadcast::Sender<crate::ChangeNotice>>,
) -> tokio::task::JoinHandle<()> {
    crate::lifecycle::spawn_worker("sprint archive scheduler", async move {
        while !crate::lifecycle::stopping() {
            match run_due_notices(&pool, 20).await {
                Ok((handled, notices)) => {
                    if let Some(changes) = &changes {
                        for notice in notices {
                            let _ = changes.send(notice);
                        }
                    }
                    if handled == 0 {
                        crate::lifecycle::pause(std::time::Duration::from_secs(15)).await;
                    }
                }
                Err(error) => {
                    tracing::warn!(?error, "sprint archive scheduler failed");
                    crate::lifecycle::pause(std::time::Duration::from_secs(15)).await;
                }
            }
        }
    })
}

/// Used by the completion command to describe the choice in its idempotency fingerprint.
pub(crate) fn fingerprint(choice: Option<SprintArchive>) -> serde_json::Value {
    match choice {
        None => json!(null),
        Some(SprintArchive::Immediate) => json!("IMMEDIATE"),
        Some(SprintArchive::At(at)) => json!(at.to_rfc3339()),
    }
}

/// The Project a Sprint belongs to, for telling that Project's open boards about a change that no ticket record carries.
pub async fn sprint_project_id(pool: &PgPool, sprint_id: Uuid) -> Result<Uuid, CoreError> {
    sqlx::query_scalar("SELECT project_id FROM penfii.sprints WHERE id = $1")
        .bind(sprint_id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::NotFound)
}
