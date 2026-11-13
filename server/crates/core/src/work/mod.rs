//! Project workflow, Ticket, revision, activity and basic Board operations.

mod activity;
mod board;
mod changes;
mod comments;
mod restore;
mod states;
mod tickets;

pub use activity::{activity, revision_detail, revisions, ticket_activity};
pub use board::{
    board, create_project_label, project_labels, project_members, project_readable,
    project_ticket_types, ticket_change_visible,
};
pub use changes::{ProjectChange, ProjectChanges, project_changes_since};
pub(crate) use changes::{project_change, record_change, record_status_category};
pub(crate) use comments::comment_change;
pub use comments::{comments, create_comment, delete_comment, edit_comment};
pub use restore::{RestoreInput, restore_ticket};
pub use states::{StateInput, StateUpdate, create_state, delete_state, update_state, workflow};
pub use tickets::{
    TicketInput, TicketUpdate, change_status, children, create_ticket, get_ticket, move_ticket,
    update_ticket,
};
pub(crate) use tickets::{load_live_ticket, load_readable_ticket};

use chrono::{DateTime, NaiveDate, Utc};
use penfii_contracts::{
    ChildSummary, DependencySummary, Ticket, TicketEstimate, TicketLabel, TicketRef,
    TicketSchedule, TicketType,
};
use serde_json::Value;
use sqlx::{PgConnection, Row};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::CoreError;
use crate::plan_fields;
use crate::planning;
use crate::project_role::{channel_allowed, require_project_role};

pub(crate) const TICKET_COLUMNS: &str = "t.id,t.workspace_id,t.project_id,t.channel_id,t.public_key,t.title,t.description,t.hierarchy_kind,t.parent_id,t.status_id,t.assignee_id,(SELECT u.display_name FROM penfii.users u WHERE u.id=t.assignee_id) AS assignee_name,t.priority,t.due_on,t.revision_no,t.created_at,t.updated_at,t.archived_at,t.ticket_type_id AS type_id,ty.name AS type_name,ty.color AS type_color,pt.public_key AS parent_key,pt.title AS parent_title,pt.hierarchy_kind AS parent_kind,t.estimate_value::text AS estimate_value,t.estimate_unit,tsc.planned_start_at AS schedule_start,tsc.duration_minutes AS schedule_duration,tsc.is_milestone AS schedule_milestone,(tsc.ticket_id IS NOT NULL) AS has_schedule";

pub(crate) const TICKET_FROM: &str = "penfii.tickets t LEFT JOIN penfii.ticket_types ty ON ty.id=t.ticket_type_id LEFT JOIN penfii.tickets pt ON pt.id=t.parent_id AND pt.trashed_at IS NULL AND pt.purged_at IS NULL LEFT JOIN penfii.ticket_schedules tsc ON tsc.ticket_id=t.id";

const TITLE_MAX_CHARS: usize = 300;

const PAGE_DEFAULT: i64 = 20;

const PAGE_MAX: i64 = 50;

#[derive(sqlx::FromRow)]
pub(crate) struct TicketRow {
    id: Uuid,
    workspace_id: Uuid,
    project_id: Uuid,
    channel_id: Option<Uuid>,
    public_key: String,
    title: String,
    description: Value,
    hierarchy_kind: String,
    parent_id: Option<Uuid>,
    status_id: Uuid,
    assignee_id: Option<Uuid>,
    assignee_name: Option<String>,
    priority: Option<String>,
    due_on: Option<NaiveDate>,
    revision_no: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    archived_at: Option<DateTime<Utc>>,
    type_id: Option<Uuid>,
    type_name: Option<String>,
    type_color: Option<String>,
    parent_key: Option<String>,
    parent_title: Option<String>,
    parent_kind: Option<String>,
    estimate_value: Option<String>,
    estimate_unit: Option<String>,
    schedule_start: Option<DateTime<Utc>>,
    schedule_duration: Option<i64>,
    schedule_milestone: Option<bool>,
    has_schedule: bool,
}

pub(crate) fn ticket(row: TicketRow) -> Ticket {
    Ticket {
        id: row.id,
        workspace_id: row.workspace_id,
        project_id: row.project_id,
        channel_id: row.channel_id,
        key: row.public_key,
        title: row.title,
        description: row.description,
        hierarchy_kind: row.hierarchy_kind,
        parent_id: row.parent_id,
        status_id: row.status_id,
        assignee_id: row.assignee_id,
        assignee_name: row.assignee_name,
        labels: Vec::new(),
        priority: row.priority,
        due_on: row.due_on,
        ticket_type: row.type_id.zip(row.type_name).map(|(id, name)| TicketType {
            id,
            name,
            color: row.type_color,
        }),
        parent: row
            .parent_id
            .zip(row.parent_key)
            .zip(row.parent_title)
            .zip(row.parent_kind)
            .map(|(((id, key), title), hierarchy_kind)| TicketRef {
                id,
                key,
                title,
                hierarchy_kind,
            }),
        child_summary: ChildSummary::default(),
        dependencies: DependencySummary::default(),
        sprint: None,
        milestones: Vec::new(),
        moved_to_top_at: None,
        estimate: row
            .estimate_value
            .zip(row.estimate_unit)
            .map(|(value, unit)| TicketEstimate {
                value: plan_fields::normalize_decimal_text(&value),
                unit,
            }),
        schedule: row.has_schedule.then_some(TicketSchedule {
            planned_start_at: row.schedule_start,
            duration_minutes: row.schedule_duration,
            is_schedule_milestone: row.schedule_milestone.unwrap_or(false),
            planned_end_at: None,
        }),
        revision_no: row.revision_no,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
    }
}

pub(crate) async fn attach_labels(
    conn: &mut PgConnection,
    ticket: &mut Ticket,
) -> Result<(), CoreError> {
    let rows=sqlx::query("SELECT l.id,l.name,l.color FROM penfii.ticket_labels tl JOIN penfii.labels l ON l.id=tl.label_id WHERE tl.ticket_id=$1 AND l.retired_at IS NULL ORDER BY l.name,l.id").bind(ticket.id).fetch_all(&mut *conn).await?;
    ticket.labels = rows
        .into_iter()
        .map(|row| -> Result<_, sqlx::Error> {
            Ok(TicketLabel {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                color: row.try_get("color")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(())
}

/// The labels of many Tickets in one query (the Board, the Team and Workspace scopes). Every Ticket gets
/// its labels, in name order, whatever the number of Tickets.
pub(crate) async fn attach_labels_batch(
    conn: &mut PgConnection,
    tickets: &mut [Ticket],
) -> Result<(), CoreError> {
    if tickets.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = tickets.iter().map(|t| t.id).collect();
    let rows = sqlx::query(
        "SELECT tl.ticket_id,l.id,l.name,l.color FROM penfii.ticket_labels tl
         JOIN penfii.labels l ON l.id=tl.label_id AND l.retired_at IS NULL
         WHERE tl.ticket_id=ANY($1) ORDER BY l.name,l.id",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut labels: std::collections::HashMap<Uuid, Vec<TicketLabel>> =
        std::collections::HashMap::new();
    for row in rows {
        labels
            .entry(row.try_get("ticket_id")?)
            .or_default()
            .push(TicketLabel {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                color: row.try_get("color")?,
            });
    }
    for ticket in tickets.iter_mut() {
        ticket.labels = labels.remove(&ticket.id).unwrap_or_default();
    }
    Ok(())
}

/// Counts direct children the viewer may see. Children in channels the viewer cannot open
/// are left out so their number is never inferable.
/// Fills the fields derived from other records: child progress, dependencies, Sprint, Milestones.
pub(crate) async fn attach_derived(
    conn: &mut PgConnection,
    ticket: &mut Ticket,
    actor_id: Uuid,
) -> Result<(), CoreError> {
    attach_children(conn, ticket, actor_id).await?;
    planning::attach_planning(conn, std::slice::from_mut(ticket), actor_id).await
}

/// Derives the planned end of a Ticket's schedule from the Project's current calendar. Only the
/// calls that return one Ticket do this; a list leaves it null.
pub(crate) async fn attach_planned_end(
    conn: &mut PgConnection,
    ticket: &mut Ticket,
) -> Result<(), CoreError> {
    if let Some(schedule) = ticket.schedule.as_mut() {
        plan_fields::with_planned_end(conn, ticket.project_id, schedule).await?;
    }
    Ok(())
}

pub(crate) async fn attach_children(
    conn: &mut PgConnection,
    ticket: &mut Ticket,
    actor_id: Uuid,
) -> Result<(), CoreError> {
    if ticket.hierarchy_kind == "SUBTASK" {
        return Ok(());
    }
    let rows = sqlx::query("SELECT c.channel_id,s.category FROM penfii.tickets c JOIN penfii.workflow_statuses s ON s.id=c.status_id WHERE c.parent_id=$1 AND ($2 OR c.archived_at IS NULL) AND c.trashed_at IS NULL AND c.purged_at IS NULL")
        .bind(ticket.id)
        // An archived parent counts the children that were archived with it.
        .bind(ticket.archived_at.is_some())
        .fetch_all(&mut *conn)
        .await?;
    let mut access: HashMap<Uuid, bool> = HashMap::new();
    let mut summary = ChildSummary::default();
    for row in rows {
        let channel: Option<Uuid> = row.try_get("channel_id")?;
        if let Some(channel_id) = channel {
            if let std::collections::hash_map::Entry::Vacant(e) = access.entry(channel_id) {
                let allowed = channel_allowed(conn, Some(channel_id), actor_id).await?;
                e.insert(allowed);
            }
            if !access[&channel_id] {
                continue;
            }
        }
        let category: String = row.try_get("category")?;
        summary.total += 1;
        match category.as_str() {
            "DONE" => summary.done += 1,
            "CANCELED" => summary.canceled += 1,
            _ => {}
        }
    }
    ticket.child_summary = summary;
    Ok(())
}

struct ReadAccess {
    project_id: Uuid,
    role: String,
    channel_id: Option<Uuid>,
    live: bool,
}

/// Read boundary shared by comments, children, Activity and History.
async fn read_access(
    conn: &mut PgConnection,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<ReadAccess, CoreError> {
    let row = sqlx::query(
        "SELECT project_id,channel_id,(archived_at IS NULL AND trashed_at IS NULL) AS live FROM penfii.tickets WHERE id=$1 AND purged_at IS NULL",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    let project_id: Uuid = row.try_get("project_id")?;
    let channel_id: Option<Uuid> = row.try_get("channel_id")?;
    let (_, role) = require_project_role(conn, project_id, actor_id, false).await?;
    if !channel_allowed(conn, channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    Ok(ReadAccess {
        project_id,
        role,
        channel_id,
        live: row.try_get("live")?,
    })
}
