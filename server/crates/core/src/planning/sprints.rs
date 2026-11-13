//! Sprints: their commands, close preview, results and Ticket assignment.

use chrono::NaiveDate;
use penfii_contracts::{
    Sprint, SprintClosePreview, SprintResult, SprintResults, SprintTarget, Ticket, TicketRef,
};
use serde_json::json;
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::derived::visible;
use super::order::lock_project;
use super::sprint_archive::{self, SprintArchive};
use super::{long_text, require_admin, short_name, version_conflict};
use crate::error::CoreError;
use crate::platform::{self, ChangeNotice};
use crate::project_role::{channel_allowed, require_project_role, require_ticket_editor};
use crate::work::{
    attach_derived, attach_planned_end, load_live_ticket, project_change, record_change,
};

pub(crate) const SPRINT_COLUMNS: &str = "id,project_id,name,goal,state,starts_on,ends_on,version,started_at,ended_at,time_zone,estimate_unit";

const ESTIMATE_UNITS: &[&str] = &["NONE", "POINTS", "HOURS"];

async fn known_time_zone(conn: &mut PgConnection, name: &str) -> Result<bool, CoreError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
            .bind(name)
            .fetch_one(&mut *conn)
            .await?,
    )
}

pub(crate) fn sprint_from(row: &sqlx::postgres::PgRow) -> Result<Sprint, CoreError> {
    Ok(Sprint {
        id: row.try_get("id")?,
        project_id: row.try_get("project_id")?,
        name: row.try_get("name")?,
        goal: row.try_get("goal")?,
        state: row.try_get("state")?,
        starts_on: row.try_get("starts_on")?,
        ends_on: row.try_get("ends_on")?,
        version: row.try_get("version")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
        time_zone: row.try_get("time_zone")?,
        estimate_unit: row.try_get("estimate_unit")?,
    })
}

pub struct SprintInput {
    pub name: String,
    pub goal: Option<String>,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
    /// Defaults to the Workspace zone.
    pub time_zone: Option<String>,
    /// Defaults to the Project's unit.
    pub estimate_unit: Option<String>,
}

pub async fn create_sprint(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    input: SprintInput,
    idem: &str,
) -> Result<Sprint, CoreError> {
    let name = short_name(&input.name, "Sprint")?;
    let goal = long_text(input.goal, "goal")?;
    if input.ends_on < input.starts_on {
        return Err(CoreError::validation(
            "a Sprint cannot end before it starts",
        ));
    }
    let mut tx = pool.begin().await?;
    let workspace = require_admin(&mut tx, project_id, actor_id).await?;
    let (default_zone, default_unit): (String, String) = sqlx::query_as(
        "SELECT w.time_zone, p.estimate_unit FROM penfii.projects p
         JOIN penfii.workspaces w ON w.id=p.workspace_id WHERE p.id=$1",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await?;
    let time_zone = input
        .time_zone
        .map_or(default_zone, |zone| zone.trim().to_owned());
    let estimate_unit = input.estimate_unit.unwrap_or(default_unit);
    if !ESTIMATE_UNITS.contains(&estimate_unit.as_str()) {
        return Err(CoreError::validation(
            "estimateUnit must be NONE, POINTS or HOURS",
        ));
    }
    if !known_time_zone(&mut tx, &time_zone).await? {
        return Err(CoreError::validation("unknown time zone"));
    }
    let body = json!({"projectId":project_id,"name":name,"goal":goal,"startsOn":input.starts_on,"endsOn":input.ends_on,"timeZone":time_zone,"estimateUnit":estimate_unit});
    if let Some(done) =
        platform::replay::<Sprint>(&mut tx, workspace, actor_id, idem, "sprint.create", &body)
            .await?
    {
        tx.commit().await?;
        return Ok(done);
    }
    let row = sqlx::query(&format!(
        "INSERT INTO penfii.sprints(id,workspace_id,project_id,name,goal,starts_on,ends_on,time_zone,estimate_unit)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {SPRINT_COLUMNS}"
    ))
    .bind(Uuid::new_v4())
    .bind(workspace)
    .bind(project_id)
    .bind(&name)
    .bind(&goal)
    .bind(input.starts_on)
    .bind(input.ends_on)
    .bind(&time_zone)
    .bind(&estimate_unit)
    .fetch_one(&mut *tx)
    .await?;
    let sprint = sprint_from(&row)?;
    platform::finish(&mut tx, workspace, actor_id, idem, 201, &sprint).await?;
    tx.commit().await?;
    Ok(sprint)
}

#[derive(Default)]
pub struct SprintUpdate {
    pub name: Option<String>,
    pub goal: Option<Option<String>>,
    pub starts_on: Option<NaiveDate>,
    pub ends_on: Option<NaiveDate>,
    /// Fixed once the Sprint starts.
    pub time_zone: Option<String>,
    /// Fixed once the Sprint starts.
    pub estimate_unit: Option<String>,
}

async fn locked_sprint(
    tx: &mut PgConnection,
    actor_id: Uuid,
    sprint_id: Uuid,
) -> Result<(sqlx::postgres::PgRow, Uuid), CoreError> {
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLUMNS},workspace_id FROM penfii.sprints WHERE id=$1 FOR UPDATE"
    ))
    .bind(sprint_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let project_id: Uuid = row.try_get("project_id")?;
    require_admin(tx, project_id, actor_id).await?;
    Ok((row, project_id))
}

pub async fn update_sprint(
    pool: &PgPool,
    actor_id: Uuid,
    sprint_id: Uuid,
    expected_version: i64,
    update: SprintUpdate,
) -> Result<Sprint, CoreError> {
    let mut tx = pool.begin().await?;
    let (row, _) = locked_sprint(&mut tx, actor_id, sprint_id).await?;
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    let state: String = row.try_get("state")?;
    if !matches!(state.as_str(), "PLANNED" | "ACTIVE") {
        return Err(CoreError::conflict(
            "invalid_sprint_state",
            "a finished Sprint cannot be edited",
        ));
    }
    let name = match update.name {
        Some(name) => short_name(&name, "Sprint")?,
        None => row.try_get("name")?,
    };
    let goal = match update.goal {
        Some(goal) => long_text(goal, "goal")?,
        None => row.try_get("goal")?,
    };
    let starts_on: NaiveDate = update.starts_on.unwrap_or(row.try_get("starts_on")?);
    let ends_on: NaiveDate = update.ends_on.unwrap_or(row.try_get("ends_on")?);
    if ends_on < starts_on {
        return Err(CoreError::validation(
            "a Sprint cannot end before it starts",
        ));
    }
    let current_zone: String = row.try_get("time_zone")?;
    let current_unit: String = row.try_get("estimate_unit")?;
    let time_zone = update
        .time_zone
        .map_or(current_zone.clone(), |zone| zone.trim().to_owned());
    let estimate_unit = update.estimate_unit.unwrap_or(current_unit.clone());
    if (time_zone != current_zone || estimate_unit != current_unit) && state != "PLANNED" {
        return Err(CoreError::conflict(
            "invalid_sprint_state",
            "the time zone and the estimate unit are fixed once the Sprint starts",
        ));
    }
    if !ESTIMATE_UNITS.contains(&estimate_unit.as_str()) {
        return Err(CoreError::validation(
            "estimateUnit must be NONE, POINTS or HOURS",
        ));
    }
    if time_zone != current_zone && !known_time_zone(&mut tx, &time_zone).await? {
        return Err(CoreError::validation("unknown time zone"));
    }
    let updated = sqlx::query(&format!(
        "UPDATE penfii.sprints SET name=$2,goal=$3,starts_on=$4,ends_on=$5,time_zone=$6,estimate_unit=$7,version=version+1 WHERE id=$1 RETURNING {SPRINT_COLUMNS}"
    ))
    .bind(sprint_id)
    .bind(&name)
    .bind(&goal)
    .bind(starts_on)
    .bind(ends_on)
    .bind(&time_zone)
    .bind(&estimate_unit)
    .fetch_one(&mut *tx)
    .await?;
    let sprint = sprint_from(&updated)?;
    tx.commit().await?;
    Ok(sprint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprintCommand {
    Start,
    Complete,
    Cancel,
}

impl SprintCommand {
    fn kind(self) -> &'static str {
        match self {
            Self::Start => "sprint.start",
            Self::Complete => "sprint.complete",
            Self::Cancel => "sprint.cancel",
        }
    }
}

/// Writes, once, what the Task was when its assignment ended: its State category and estimate. The
/// Report of a closed Sprint reads these and never changes with later edits.
async fn snapshot_assignment_end(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<(), CoreError> {
    sqlx::query(
        "UPDATE penfii.sprint_assignments a
         SET ended_status_category=s.category, ended_estimate_value=t.estimate_value, ended_estimate_unit=t.estimate_unit
         FROM penfii.tickets t JOIN penfii.workflow_statuses s ON s.id=t.status_id
         WHERE a.id=$1 AND t.id=a.ticket_id",
    )
    .bind(assignment_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The outcome recorded for a Task that leaves a Sprint, from its State category at that moment.
fn outcome_of(category: &str) -> &'static str {
    match category {
        "DONE" => "DONE",
        "CANCELED" => "CANCELED",
        _ => "NOT_DONE",
    }
}

/// What closing a Sprint decides beyond completing it: where its unfinished Tasks go, and when its
/// finished work is archived.
#[derive(Debug, Clone, Copy, Default)]
pub struct CloseChoice {
    pub carry_to: Option<Uuid>,
    pub archive: Option<SprintArchive>,
}

/// Starts, completes or cancels a Sprint.
///
/// Start records the baseline once. Complete is the one-decision close: every current assignment
/// ends with its outcome, and the unfinished Tasks the admin can see go either to the backlog or
/// (`carry_to`) into one PLANNED/ACTIVE Sprint of the Project, in the same transaction. Ticket
/// States never change. Cancel only releases assignments. Neither can be undone.
pub async fn sprint_command(
    pool: &PgPool,
    actor_id: Uuid,
    sprint_id: Uuid,
    expected_version: i64,
    command: SprintCommand,
    close: CloseChoice,
    idem: &str,
) -> Result<(Sprint, Vec<ChangeNotice>), CoreError> {
    let CloseChoice { carry_to, archive } = close;
    if carry_to.is_some() && command != SprintCommand::Complete {
        return Err(CoreError::validation(
            "only closing a Sprint can carry its unfinished work to another Sprint",
        ));
    }
    if archive.is_some() && command != SprintCommand::Complete {
        return Err(CoreError::validation(
            "only closing a Sprint can archive its finished work",
        ));
    }
    if let Some(SprintArchive::At(at)) = archive {
        sprint_archive::require_future(at)?;
    }
    let mut tx = pool.begin().await?;
    let (row, project_id) = locked_sprint(&mut tx, actor_id, sprint_id).await?;
    let workspace: Uuid = row.try_get("workspace_id")?;
    let body = json!({"sprintId":sprint_id,"expectedVersion":expected_version,"carryTo":carry_to,"archive":sprint_archive::fingerprint(archive)});
    if let Some(done) =
        platform::replay::<Sprint>(&mut tx, workspace, actor_id, idem, command.kind(), &body)
            .await?
    {
        tx.commit().await?;
        return Ok((done, Vec::new()));
    }
    let current: i64 = row.try_get("version")?;
    if current != expected_version {
        return Err(version_conflict(current));
    }
    let state: String = row.try_get("state")?;
    let next = match (command, state.as_str()) {
        (SprintCommand::Start, "PLANNED") => "ACTIVE",
        (SprintCommand::Complete, "ACTIVE") => "COMPLETED",
        (SprintCommand::Cancel, "PLANNED" | "ACTIVE") => "CANCELED",
        _ => {
            return Err(CoreError::conflict(
                "invalid_sprint_state",
                format!("this Sprint is {state}"),
            ));
        }
    };
    // Every Sprint command serializes on the Project, so carry-over sees one consistent picture.
    lock_project(&mut tx, project_id).await?;
    let target = carry_target(&mut tx, project_id, sprint_id, carry_to).await?;
    let updated = sqlx::query(&format!(
        "UPDATE penfii.sprints SET state=$2,version=version+1,
                started_at=CASE WHEN $2='ACTIVE' THEN now() ELSE started_at END,
                ended_at=CASE WHEN $2 IN ('COMPLETED','CANCELED') THEN now() ELSE ended_at END
         WHERE id=$1 RETURNING {SPRINT_COLUMNS}"
    ))
    .bind(sprint_id)
    .bind(next)
    .fetch_one(&mut *tx)
    .await?;
    let sprint_name: String = row.try_get("name")?;
    let mut notices = if command == SprintCommand::Start {
        record_baseline(&mut tx, actor_id, sprint_id, &updated).await?;
        Vec::new()
    } else {
        release_assignments(
            &mut tx,
            ReleasedFrom {
                actor_id,
                project_id,
                sprint_id,
                sprint_name: &sprint_name,
                complete: command == SprintCommand::Complete,
            },
            target.as_ref(),
        )
        .await?
    };
    // The finished work is archived with the completion, or scheduled. Unfinished work was already
    // carried above; nothing here moves it.
    match archive {
        Some(SprintArchive::Immediate) => {
            let counts =
                sprint_archive::archive_finished(&mut tx, actor_id, sprint_id, false).await?;
            if counts.archived > 0 {
                // Many Tickets left the board at once: one notice for the Project, not one per Ticket.
                notices.push(crate::ChangeNotice::Project { project_id });
            }
        }
        Some(SprintArchive::At(at)) => {
            let time_zone: String = row.try_get("time_zone")?;
            sprint_archive::insert_schedule(
                &mut tx, workspace, project_id, sprint_id, actor_id, at, &time_zone,
            )
            .await?;
        }
        None => {}
    }
    let sprint = sprint_from(&updated)?;
    let sprint_event = match command {
        SprintCommand::Start => Some("sprint.started"),
        SprintCommand::Complete => Some("sprint.closed"),
        SprintCommand::Cancel => None,
    };
    if let Some(kind) = sprint_event {
        crate::events::emit(
            &mut tx,
            crate::events::NewEvent {
                workspace_id: workspace,
                target_kind: "SPRINT",
                target_id: sprint_id,
                target_version: None,
                kind,
                actor_user_id: Some(actor_id),
                command_id: Uuid::new_v4(),
                facts: json!({"projectId": project_id, "name": sprint_name}),
                security_context: json!({"projectId": project_id.to_string()}),
            },
        )
        .await?;
    }
    platform::finish(&mut tx, workspace, actor_id, idem, 200, &sprint).await?;
    tx.commit().await?;
    Ok((sprint, notices))
}

/// The Sprint that unfinished work is carried into on close, checked against the Project.
async fn carry_target(
    tx: &mut PgConnection,
    project_id: Uuid,
    sprint_id: Uuid,
    carry_to: Option<Uuid>,
) -> Result<Option<(Uuid, String)>, CoreError> {
    let Some(target_id) = carry_to else {
        return Ok(None);
    };
    if target_id == sprint_id {
        return Err(CoreError::validation(
            "unfinished work cannot be carried into the Sprint being closed",
        ));
    }
    let target = sqlx::query(
        "SELECT id,name FROM penfii.sprints WHERE id=$1 AND project_id=$2 AND state IN ('PLANNED','ACTIVE')",
    )
    .bind(target_id)
    .bind(project_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        CoreError::validation("the next Sprint must be planned or active in this project")
    })?;
    Ok(Some((
        target.try_get::<Uuid, _>("id")?,
        target.try_get::<String, _>("name")?,
    )))
}

/// Records the Sprint baseline, inside the start command's transaction.
async fn record_baseline(
    tx: &mut PgConnection,
    actor_id: Uuid,
    sprint_id: Uuid,
    updated: &sqlx::postgres::PgRow,
) -> Result<(), CoreError> {
    // The baseline is written once, at start, and never corrected. Version 2 keeps, per Task,
    // the revision, State and category and estimate it had at this moment; the Sprint's
    // zone and unit are fixed with it. One statement reads one consistent picture.
    sqlx::query(
        "INSERT INTO penfii.sprint_baseline_tickets(sprint_id,ticket_id,ticket_revision_no,status_id,status_category,estimate_value,estimate_unit,hierarchy_kind)
         SELECT $1,a.ticket_id,t.revision_no,t.status_id,s.category,t.estimate_value,t.estimate_unit,t.hierarchy_kind
         FROM penfii.sprint_assignments a
         JOIN penfii.tickets t ON t.id=a.ticket_id
         JOIN penfii.workflow_statuses s ON s.id=t.status_id
         WHERE a.sprint_id=$1 AND a.ended_at IS NULL",
    )
    .bind(sprint_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO penfii.sprint_baselines(sprint_id,recorded_by,task_count,snapshot_version,estimate_unit,time_zone,policy_version)
         SELECT $1,$2,count(*),2,$3,$4,1 FROM penfii.sprint_baseline_tickets WHERE sprint_id=$1",
    )
    .bind(sprint_id)
    .bind(actor_id)
    .bind(updated.try_get::<String, _>("estimate_unit")?)
    .bind(updated.try_get::<String, _>("time_zone")?)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The Sprint being closed or canceled, as the assignment release needs it.
struct ReleasedFrom<'a> {
    actor_id: Uuid,
    project_id: Uuid,
    sprint_id: Uuid,
    sprint_name: &'a str,
    /// Complete (true) or cancel (false).
    complete: bool,
}

/// Ends every open assignment of a Sprint that is being completed or canceled, inside that
/// command's transaction. Unfinished Tasks the actor can see go to `target` when there is one; a
/// Task the actor cannot see is released, never moved on their behalf, and flagged for its editors
/// (the `channel_allowed` check below). Returns one change notice per Ticket.
async fn release_assignments(
    tx: &mut PgConnection,
    sprint: ReleasedFrom<'_>,
    target: Option<&(Uuid, String)>,
) -> Result<Vec<ChangeNotice>, CoreError> {
    let ReleasedFrom {
        actor_id,
        project_id,
        sprint_id,
        sprint_name,
        complete,
    } = sprint;
    let mut notices = Vec::new();
    let reason = if complete {
        "SPRINT_COMPLETED"
    } else {
        "SPRINT_CANCELED"
    };
    let open = sqlx::query(
        "SELECT a.id, a.ticket_id, s.category
         FROM penfii.sprint_assignments a
         JOIN penfii.tickets t ON t.id=a.ticket_id
         JOIN penfii.workflow_statuses s ON s.id=t.status_id
         WHERE a.sprint_id=$1 AND a.ended_at IS NULL
         ORDER BY a.assigned_at, a.id
         FOR UPDATE OF a",
    )
    .bind(sprint_id)
    .fetch_all(&mut *tx)
    .await?;
    for entry in open {
        let assignment_id: Uuid = entry.try_get("id")?;
        let ticket_id: Uuid = entry.try_get("ticket_id")?;
        let category: String = entry.try_get("category")?;
        let outcome = outcome_of(&category);
        let ticket = load_live_ticket(&mut *tx, ticket_id, true).await.ok();
        let unfinished = outcome == "NOT_DONE";
        let seen = match &ticket {
            Some(t) => channel_allowed(&mut *tx, t.channel_id, actor_id).await?,
            None => false,
        };
        // A Task the admin cannot see is released, never moved on their behalf, and flagged for its
        // editors. A Task that was trashed meanwhile is released quietly.
        let carry = if unfinished && seen { target } else { None };
        sqlx::query(
            "UPDATE penfii.sprint_assignments
             SET ended_at=now(), end_reason=$2, outcome=$3, carried_to_sprint_id=$4, needs_next_assignment=$5
             WHERE id=$1",
        )
        .bind(assignment_id)
        .bind(reason)
        .bind(outcome)
        .bind(carry.map(|(id, _)| *id))
        .bind(unfinished && ticket.is_some() && !seen)
        .execute(&mut *tx)
        .await?;
        snapshot_assignment_end(&mut *tx, assignment_id).await?;
        if let Some((target_id, _)) = carry {
            sqlx::query(
                "INSERT INTO penfii.sprint_assignments(id,sprint_id,ticket_id,assigned_by,carried_from_sprint_id)
                 VALUES($1,$2,$3,$4,$5)",
            )
            .bind(Uuid::new_v4())
            .bind(target_id)
            .bind(ticket_id)
            .bind(actor_id)
            .bind(sprint_id)
            .execute(&mut *tx)
            .await?;
        }
        let revision_no = ticket.as_ref().map_or(0, |t| t.revision_no);
        let seq = match &ticket {
            Some(t) => {
                let from = json!({"id":sprint_id,"name":sprint_name});
                let to = carry.map(|(id, name)| json!({"id":id,"name":name}));
                record_change(
                    &mut *tx,
                    t,
                    actor_id,
                    Uuid::new_v4(),
                    "ticket.sprint_changed",
                    json!({"ticketId":ticket_id,"from":from,"to":to,"outcome":outcome}),
                )
                .await?
                .0
            }
            None => {
                project_change(
                    &mut *tx,
                    project_id,
                    ticket_id,
                    revision_no,
                    "ticket.sprint_changed",
                )
                .await?
            }
        };
        notices.push(ChangeNotice::Ticket {
            project_id,
            change_seq: seq,
            ticket_id,
            ticket_revision_no: revision_no,
            change_kind: "ticket.sprint_changed".into(),
        });
    }
    Ok(notices)
}

fn target_from(row: &sqlx::postgres::PgRow) -> Result<SprintTarget, CoreError> {
    Ok(SprintTarget {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        state: row.try_get("state")?,
        starts_on: row.try_get("starts_on")?,
    })
}

fn ticket_ref(row: &sqlx::postgres::PgRow) -> Result<TicketRef, CoreError> {
    Ok(TicketRef {
        id: row.try_get("ticket_id")?,
        key: row.try_get("public_key")?,
        title: row.try_get("title")?,
        hierarchy_kind: row.try_get("hierarchy_kind")?,
    })
}

/// What closing the Sprint would do, for the admin to look at before confirming. It lists only
/// Tasks the admin can see; the others are released and flagged for their editors without a trace
/// here.
pub async fn sprint_close_preview(
    pool: &PgPool,
    actor_id: Uuid,
    sprint_id: Uuid,
) -> Result<SprintClosePreview, CoreError> {
    let mut tx = pool.begin().await?;
    let (row, project_id) = locked_sprint(&mut tx, actor_id, sprint_id).await?;
    let state: String = row.try_get("state")?;
    if state != "ACTIVE" {
        return Err(CoreError::conflict(
            "invalid_sprint_state",
            format!("this Sprint is {state}"),
        ));
    }
    let version: i64 = row.try_get("version")?;
    let starts_on: NaiveDate = row.try_get("starts_on")?;
    let rows = sqlx::query(
        "SELECT t.id AS ticket_id, t.public_key, t.title, t.hierarchy_kind, t.channel_id, s.category
         FROM penfii.sprint_assignments a
         JOIN penfii.tickets t ON t.id=a.ticket_id
         JOIN penfii.workflow_statuses s ON s.id=t.status_id
         WHERE a.sprint_id=$1 AND a.ended_at IS NULL
         ORDER BY a.assigned_at, a.id",
    )
    .bind(sprint_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut access = HashMap::new();
    let (mut done, mut canceled, mut unfinished) = (Vec::new(), Vec::new(), Vec::new());
    for entry in rows {
        let channel: Option<Uuid> = entry.try_get("channel_id")?;
        if !visible(&mut tx, &mut access, channel, actor_id).await? {
            continue;
        }
        let category: String = entry.try_get("category")?;
        let item = ticket_ref(&entry)?;
        match outcome_of(&category) {
            "DONE" => done.push(item),
            "CANCELED" => canceled.push(item),
            _ => unfinished.push(item),
        }
    }
    let targets = sqlx::query(
        "SELECT id,name,state,starts_on FROM penfii.sprints
         WHERE project_id=$1 AND id<>$2 AND state IN ('PLANNED','ACTIVE')
         ORDER BY starts_on, created_at, id",
    )
    .bind(project_id)
    .bind(sprint_id)
    .fetch_all(&mut *tx)
    .await?;
    let targets = targets
        .iter()
        .map(target_from)
        .collect::<Result<Vec<_>, _>>()?;
    // The next Sprint is the first one that starts on or after this one; none means the backlog.
    let default_target_id = targets
        .iter()
        .find(|t| t.starts_on >= starts_on)
        .map(|t| t.id);
    tx.commit().await?;
    Ok(SprintClosePreview {
        sprint_id,
        sprint_version: version,
        done,
        canceled,
        unfinished,
        targets,
        default_target_id,
    })
}

/// The recorded participation of a Sprint: each Task with how it left, where unfinished work went,
/// and whether it was in the baseline. It is built from the assignment history, so a closed
/// Sprint keeps its list after the current assignments are gone.
pub async fn sprint_results(
    pool: &PgPool,
    actor_id: Uuid,
    sprint_id: Uuid,
) -> Result<SprintResults, CoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLUMNS} FROM penfii.sprints WHERE id=$1"
    ))
    .bind(sprint_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let sprint = sprint_from(&row)?;
    require_project_role(&mut tx, sprint.project_id, actor_id, false).await?;
    let baseline = sqlx::query(
        "SELECT recorded_at, task_count FROM penfii.sprint_baselines WHERE sprint_id=$1",
    )
    .bind(sprint_id)
    .fetch_optional(&mut *tx)
    .await?;
    let rows = sqlx::query(
        "SELECT DISTINCT ON (a.ticket_id)
                a.ticket_id, a.assigned_at, a.ended_at, a.end_reason, a.outcome,
                a.needs_next_assignment, t.public_key, t.title, t.hierarchy_kind, t.channel_id,
                (b.ticket_id IS NOT NULL) AS in_baseline,
                ct.id AS to_id, ct.name AS to_name, ct.state AS to_state, ct.starts_on AS to_starts_on,
                cf.id AS from_id, cf.name AS from_name, cf.state AS from_state, cf.starts_on AS from_starts_on
         FROM penfii.sprint_assignments a
         JOIN penfii.tickets t ON t.id=a.ticket_id
         LEFT JOIN penfii.sprint_baseline_tickets b ON b.sprint_id=a.sprint_id AND b.ticket_id=a.ticket_id
         LEFT JOIN penfii.sprints ct ON ct.id=a.carried_to_sprint_id
         LEFT JOIN penfii.sprints cf ON cf.id=a.carried_from_sprint_id
         WHERE a.sprint_id=$1 AND t.purged_at IS NULL
         ORDER BY a.ticket_id, a.assigned_at DESC, a.id",
    )
    .bind(sprint_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut access = HashMap::new();
    let mut results = Vec::new();
    for entry in rows {
        let channel: Option<Uuid> = entry.try_get("channel_id")?;
        if !visible(&mut tx, &mut access, channel, actor_id).await? {
            continue;
        }
        let ended_at: Option<chrono::DateTime<chrono::Utc>> = entry.try_get("ended_at")?;
        let end_reason: Option<String> = entry.try_get("end_reason")?;
        let outcome: Option<String> = entry.try_get("outcome")?;
        let outcome = match (outcome, end_reason.as_deref()) {
            (Some(outcome), _) => Some(outcome),
            (None, Some("REMOVED" | "REASSIGNED")) => Some("REMOVED".to_owned()),
            _ => None,
        };
        // A manual removal keeps its own outcome even when the Task was finished at that time.
        let outcome = if matches!(end_reason.as_deref(), Some("REMOVED" | "REASSIGNED")) {
            Some("REMOVED".to_owned())
        } else {
            outcome
        };
        let carried_to = match entry.try_get::<Option<Uuid>, _>("to_id")? {
            Some(id) => Some(SprintTarget {
                id,
                name: entry.try_get("to_name")?,
                state: entry.try_get("to_state")?,
                starts_on: entry.try_get("to_starts_on")?,
            }),
            None => None,
        };
        let carried_from = match entry.try_get::<Option<Uuid>, _>("from_id")? {
            Some(id) => Some(SprintTarget {
                id,
                name: entry.try_get("from_name")?,
                state: entry.try_get("from_state")?,
                starts_on: entry.try_get("from_starts_on")?,
            }),
            None => None,
        };
        results.push(SprintResult {
            ticket: ticket_ref(&entry)?,
            outcome,
            carried_to,
            carried_from,
            needs_next_assignment: entry.try_get("needs_next_assignment")?,
            in_baseline: entry.try_get("in_baseline")?,
            assigned_at: entry.try_get("assigned_at")?,
            ended_at,
        });
    }
    results.sort_by(|a, b| {
        a.assigned_at
            .cmp(&b.assigned_at)
            .then_with(|| a.ticket.key.cmp(&b.ticket.key))
    });
    tx.commit().await?;
    Ok(SprintResults {
        sprint,
        baseline_count: baseline
            .as_ref()
            .map(|b| b.try_get::<i32, _>("task_count"))
            .transpose()?
            .map(i64::from),
        baseline_recorded_at: baseline.map(|b| b.try_get("recorded_at")).transpose()?,
        results,
    })
}

/// Puts a Task into a PLANNED or ACTIVE Sprint, moves it, or takes it out (`None`).
/// A Subtask follows its parent Task, and an Epic is not a unit of Sprint work.
pub async fn assign_sprint(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    sprint_id: Option<Uuid>,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let ticket = load_live_ticket(&mut tx, ticket_id, true).await?;
    require_ticket_editor(&mut tx, ticket.project_id, ticket.channel_id, actor_id).await?;
    let body = json!({"ticketId":ticket_id,"sprintId":sprint_id});
    if let Some(done) = platform::replay::<Ticket>(
        &mut tx,
        ticket.workspace_id,
        actor_id,
        idem,
        "ticket.assign_sprint",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((done, None));
    }
    match ticket.hierarchy_kind.as_str() {
        "TASK" => {}
        "EPIC" => {
            return Err(CoreError::validation(
                "an Epic is not planned into a Sprint; plan its Tasks",
            ));
        }
        _ => {
            return Err(CoreError::validation(
                "a Subtask follows the Sprint of its parent Task",
            ));
        }
    }
    lock_project(&mut tx, ticket.project_id).await?;
    let current = sqlx::query(
        "SELECT a.id, s.id AS sprint_id, s.name FROM penfii.sprint_assignments a
         JOIN penfii.sprints s ON s.id=a.sprint_id WHERE a.ticket_id=$1 AND a.ended_at IS NULL",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *tx)
    .await?;
    let current_id: Option<Uuid> = current
        .as_ref()
        .map(|r| r.try_get("sprint_id"))
        .transpose()?;
    let mut notice = None;
    if current_id != sprint_id {
        let target = match sprint_id {
            Some(id) => {
                let row = sqlx::query(
                    "SELECT id,name FROM penfii.sprints WHERE id=$1 AND project_id=$2 AND state IN ('PLANNED','ACTIVE')",
                )
                .bind(id)
                .bind(ticket.project_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    CoreError::validation("the Sprint must be planned or active in this project")
                })?;
                Some((id, row.try_get::<String, _>("name")?))
            }
            None => None,
        };
        if let Some(row) = &current {
            let reason = if target.is_some() {
                "REASSIGNED"
            } else {
                "REMOVED"
            };
            sqlx::query(
                "UPDATE penfii.sprint_assignments SET ended_at=now(), end_reason=$2 WHERE id=$1",
            )
            .bind(row.try_get::<Uuid, _>("id")?)
            .bind(reason)
            .execute(&mut *tx)
            .await?;
            snapshot_assignment_end(&mut tx, row.try_get::<Uuid, _>("id")?).await?;
        }
        if let Some((id, _)) = &target {
            sqlx::query(
                "INSERT INTO penfii.sprint_assignments(id,sprint_id,ticket_id,assigned_by) VALUES($1,$2,$3,$4)",
            )
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(ticket_id)
            .bind(actor_id)
            .execute(&mut *tx)
            .await?;
        }
        let from = current
            .as_ref()
            .map(|r| -> Result<_, sqlx::Error> { Ok(json!({"id":r.try_get::<Uuid,_>("sprint_id")?,"name":r.try_get::<String,_>("name")?})) }).transpose()?;
        let to = target
            .as_ref()
            .map(|(id, name)| json!({"id":id,"name":name}));
        let (seq, _) = record_change(
            &mut tx,
            &ticket,
            actor_id,
            Uuid::new_v4(),
            "ticket.sprint_changed",
            json!({"ticketId":ticket_id,"from":from,"to":to}),
        )
        .await?;
        notice = Some(ChangeNotice::Ticket {
            project_id: ticket.project_id,
            change_seq: seq,
            ticket_id,
            ticket_revision_no: ticket.revision_no,
            change_kind: "ticket.sprint_changed".into(),
        });
    }
    let mut value = load_live_ticket(&mut tx, ticket_id, false).await?;
    attach_derived(&mut tx, &mut value, actor_id).await?;
    attach_planned_end(&mut tx, &mut value).await?;
    platform::finish(&mut tx, ticket.workspace_id, actor_id, idem, 200, &value).await?;
    tx.commit().await?;
    Ok((value, notice))
}
