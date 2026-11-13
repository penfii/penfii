//! Ticket reads and the commands that create, update, move and change the status of a Ticket.

use chrono::NaiveDate;
use penfii_contracts::Ticket;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::changes::{project_change, record_change, save_snapshot};
use super::{
    TICKET_COLUMNS, TICKET_FROM, TITLE_MAX_CHARS, TicketRow, attach_derived, attach_labels,
    attach_planned_end, read_access, ticket,
};
use crate::error::CoreError;
use crate::plan_fields::{self, EstimateInput, ScheduleInput};
use crate::planning;
use crate::platform::{self, ChangeNotice};
use crate::project_role::{channel_allowed, require_project_role, require_ticket_editor};

#[derive(Debug, Clone)]
pub struct TicketInput {
    pub title: String,
    pub description: Value,
    pub hierarchy_kind: String,
    pub parent_id: Option<Uuid>,
    pub channel_id: Option<Uuid>,
    pub status_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub priority: Option<String>,
    pub due_on: Option<NaiveDate>,
    pub label_ids: Vec<Uuid>,
    pub type_id: Option<Uuid>,
    pub estimate: Option<EstimateInput>,
    pub schedule: Option<ScheduleInput>,
    /// Finished uploads that become the new Ticket's body attachments, in the same transaction.
    pub file_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct TicketUpdate {
    pub title: Option<String>,
    pub description: Option<Value>,
    pub assignee_id: Option<Option<Uuid>>,
    pub priority: Option<Option<String>>,
    pub due_on: Option<Option<NaiveDate>>,
    pub label_ids: Option<Vec<Uuid>>,
    pub parent_id: Option<Option<Uuid>>,
    pub type_id: Option<Option<Uuid>>,
    /// `Some(None)` clears the estimate.
    pub estimate: Option<Option<EstimateInput>>,
    /// `Some(None)` clears the schedule.
    pub schedule: Option<Option<ScheduleInput>>,
}

async fn validate_labels(
    conn: &mut PgConnection,
    project_id: Uuid,
    label_ids: &[Uuid],
) -> Result<(), CoreError> {
    let mut ids = label_ids.to_vec();
    ids.sort();
    ids.dedup();
    if ids.len() != label_ids.len() {
        return Err(CoreError::validation("labelIds must be distinct"));
    }
    if ids.is_empty() {
        return Ok(());
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM penfii.labels WHERE project_id=$1 AND id=ANY($2) AND retired_at IS NULL").bind(project_id).bind(&ids).fetch_one(&mut *conn).await?;
    if count != ids.len() as i64 {
        return Err(CoreError::validation(
            "all labels must be active labels in this project",
        ));
    }
    Ok(())
}

pub(super) async fn replace_labels(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    label_ids: &[Uuid],
) -> Result<(), CoreError> {
    sqlx::query("DELETE FROM penfii.ticket_labels WHERE ticket_id=$1")
        .bind(ticket_id)
        .execute(&mut *conn)
        .await?;
    for label_id in label_ids {
        sqlx::query("INSERT INTO penfii.ticket_labels(ticket_id,label_id) VALUES($1,$2)")
            .bind(ticket_id)
            .bind(label_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn channel_belongs_to_project(
    conn: &mut PgConnection,
    channel_id: Option<Uuid>,
    project_id: Uuid,
) -> Result<bool, CoreError> {
    let Some(channel_id) = channel_id else {
        return Ok(true);
    };
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.channels WHERE id=$1 AND project_id=$2 AND archived_at IS NULL)").bind(channel_id).bind(project_id).fetch_one(&mut *conn).await?)
}

/// A ticket description or comment body is a JSON document (an object), as `openapi.json` says; a
/// description may also be absent. Anything else would be stored but could never be shown.
pub(super) fn document_is_valid(value: &Value, allow_null: bool) -> bool {
    value.is_object() || (allow_null && value.is_null())
}

fn priority_is_valid(priority: Option<&str>) -> bool {
    priority.is_none_or(|value| matches!(value, "LOW" | "MEDIUM" | "HIGH" | "URGENT"))
}

pub(super) fn title_is_valid(title: &str) -> bool {
    let count = title.chars().count();
    count > 0 && count <= TITLE_MAX_CHARS
}

async fn validate_type(
    conn: &mut PgConnection,
    project_id: Uuid,
    type_id: Option<Uuid>,
) -> Result<(), CoreError> {
    let Some(type_id) = type_id else {
        return Ok(());
    };
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.ticket_types WHERE id=$1 AND project_id=$2 AND retired_at IS NULL)",
    )
    .bind(type_id)
    .bind(project_id)
    .fetch_one(&mut *conn)
    .await?;
    if !found {
        return Err(CoreError::validation(
            "type must be an active ticket type in this project",
        ));
    }
    Ok(())
}

/// Hierarchy rule shared by create and parent change. `self_id` is set when an existing
/// Ticket changes parent so the new parent cannot be itself or one of its descendants.
async fn check_parent(
    conn: &mut PgConnection,
    actor_id: Uuid,
    project_id: Uuid,
    kind: &str,
    channel_id: Option<Uuid>,
    self_id: Option<Uuid>,
    parent_id: Option<Uuid>,
) -> Result<(), CoreError> {
    match (kind, parent_id) {
        ("EPIC", Some(_)) => return Err(CoreError::validation("Epic cannot have a parent")),
        ("SUBTASK", None) => return Err(CoreError::validation("Subtask requires parentId")),
        (_, None) => return Ok(()),
        _ => {}
    }
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    if self_id == Some(parent_id) {
        return Err(CoreError::validation("a ticket cannot be its own parent"));
    }
    let parent = sqlx::query("SELECT project_id,hierarchy_kind,channel_id,parent_id FROM penfii.tickets WHERE id=$1 AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL FOR SHARE")
        .bind(parent_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(CoreError::NotFound)?;
    let parent_project: Uuid = parent.try_get("project_id")?;
    let parent_kind: String = parent.try_get("hierarchy_kind")?;
    let parent_channel: Option<Uuid> = parent.try_get("channel_id")?;
    if !channel_allowed(conn, parent_channel, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    if parent_project != project_id {
        return Err(CoreError::validation("parent must be in the same project"));
    }
    let expected_parent_kind = if kind == "SUBTASK" { "TASK" } else { "EPIC" };
    if parent_kind != expected_parent_kind {
        return Err(CoreError::validation(format!(
            "{kind} parent must be a {expected_parent_kind}"
        )));
    }
    if kind == "SUBTASK" && parent_channel != channel_id {
        return Err(CoreError::validation(
            "Subtask must be in the same channel as its parent Task",
        ));
    }
    if let Some(self_id) = self_id {
        // Walk up from the proposed parent; reaching this ticket would close a cycle.
        let mut cursor: Option<Uuid> = parent.try_get("parent_id")?;
        let mut hops = 0;
        while let Some(ancestor) = cursor {
            if ancestor == self_id || hops > 3 {
                return Err(CoreError::validation("parent would create a cycle"));
            }
            cursor = sqlx::query_scalar("SELECT parent_id FROM penfii.tickets WHERE id=$1")
                .bind(ancestor)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
            hops += 1;
        }
    }
    Ok(())
}

pub async fn get_ticket(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<Ticket, CoreError> {
    let mut tx = pool.begin().await?;
    // An archived Ticket opens (read-only: every command goes through the live loaders); a trashed or purged one does not.
    let row=sqlx::query_as::<_,TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.trashed_at IS NULL AND t.purged_at IS NULL")).bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let mut value = ticket(row);
    let _ = require_project_role(&mut tx, value.project_id, actor_id, false).await?;
    if !channel_allowed(&mut tx, value.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    attach_labels(&mut tx, &mut value).await?;
    attach_derived(&mut tx, &mut value, actor_id).await?;
    attach_planned_end(&mut tx, &mut value).await?;
    tx.commit().await?;
    Ok(value)
}

/// Whether `assignee` may be given a Ticket of this Project and channel: an active member of the
/// Workspace, taking part in the Project (only when `must_take_part`; work is given to people who
/// take part, and taking part comes first), and able to open the channel. Runs inside the caller's
/// transaction, after the caller has authorized the actor.
async fn check_assignee(
    tx: &mut PgConnection,
    workspace: Uuid,
    project_id: Uuid,
    channel_id: Option<Uuid>,
    assignee: Uuid,
    must_take_part: bool,
) -> Result<(), CoreError> {
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.workspace_members WHERE workspace_id=$1 AND user_id=$2 AND state='ACTIVE')").bind(workspace).bind(assignee).fetch_one(&mut *tx).await?;
    if !active {
        return Err(CoreError::validation(
            "assignee must be an active workspace member",
        ));
    }
    if must_take_part && !crate::access::in_share_scope(tx, project_id, None, assignee).await? {
        return Err(CoreError::validation(
            "assignee must take part in the Project",
        ));
    }
    if !channel_allowed(tx, channel_id, assignee).await? {
        return Err(CoreError::validation(
            "assignee must be able to access the ticket channel",
        ));
    }
    Ok(())
}

pub async fn create_ticket(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    input: TicketInput,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let title = input.title.trim();
    if !title_is_valid(title) {
        return Err(CoreError::validation("title must be 1..300 characters"));
    }
    if !matches!(input.hierarchy_kind.as_str(), "EPIC" | "TASK" | "SUBTASK") {
        return Err(CoreError::validation("invalid hierarchyKind"));
    }
    if !priority_is_valid(input.priority.as_deref()) {
        return Err(CoreError::validation(
            "priority must be LOW, MEDIUM, HIGH, URGENT, or unset",
        ));
    }
    if !document_is_valid(&input.description, true) {
        return Err(CoreError::validation("description must be a JSON object"));
    }
    let body = json!({"title":title,"description":input.description,"hierarchyKind":input.hierarchy_kind,"parentId":input.parent_id,"channelId":input.channel_id,"statusId":input.status_id,"assigneeId":input.assignee_id,"priority":input.priority,"dueOn":input.due_on,"labelIds":input.label_ids,"typeId":input.type_id,"estimate":input.estimate.as_ref().map(|e| json!([e.value, e.unit])),"schedule":input.schedule.as_ref().map(|s| json!([s.planned_start_at, s.duration_minutes, s.is_schedule_milestone])),"fileIds":input.file_ids});
    let mut tx = pool.begin().await?;
    let (workspace, role) = require_project_role(&mut tx, project_id, actor_id, true).await?;
    if !channel_allowed(&mut tx, input.channel_id, actor_id).await? {
        return Err(CoreError::NotFound);
    }
    validate_labels(&mut tx, project_id, &input.label_ids).await?;
    if !channel_belongs_to_project(&mut tx, input.channel_id, project_id).await? {
        return Err(CoreError::validation("channel must belong to this project"));
    }
    if input.hierarchy_kind == "EPIC" && input.channel_id.is_some() {
        return Err(CoreError::validation("Epic cannot belong to a channel"));
    }
    validate_type(&mut tx, project_id, input.type_id).await?;
    check_parent(
        &mut tx,
        actor_id,
        project_id,
        &input.hierarchy_kind,
        input.channel_id,
        None,
        input.parent_id,
    )
    .await?;
    if let Some(assignee) = input.assignee_id {
        check_assignee(
            &mut tx,
            workspace,
            project_id,
            input.channel_id,
            assignee,
            true,
        )
        .await?;
    }
    let estimate = plan_fields::resolve_estimate(
        &mut tx,
        project_id,
        &input.hierarchy_kind,
        input.estimate.clone(),
    )
    .await?;
    let schedule = plan_fields::resolve_schedule(input.schedule.clone())?;
    let status_id = if let Some(id) = input.status_id {
        id
    } else {
        sqlx::query_scalar("SELECT id FROM penfii.workflow_statuses WHERE project_id=$1 AND is_initial AND retired_at IS NULL").bind(project_id).fetch_optional(&mut *tx).await?.ok_or_else(|| CoreError::conflict("workflow_unconfigured","project has no initial status"))?
    };
    let status_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.workflow_statuses WHERE project_id=$1 AND id=$2 AND retired_at IS NULL)").bind(project_id).bind(status_id).fetch_one(&mut *tx).await?;
    if !status_exists {
        return Err(CoreError::validation(
            "status must be active in this project workflow",
        ));
    }
    let initial_status:Uuid=sqlx::query_scalar("SELECT id FROM penfii.workflow_statuses WHERE project_id=$1 AND is_initial AND retired_at IS NULL").bind(project_id).fetch_one(&mut *tx).await?;
    if status_id != initial_status {
        return Err(CoreError::validation(
            "new tickets must start in the workflow initial state",
        ));
    }
    let command_id = Uuid::new_v4();
    if let Some(replay) =
        platform::replay::<Ticket>(&mut tx, workspace, actor_id, idem, "ticket.create", &body)
            .await?
    {
        tx.commit().await?;
        return Ok((replay, None));
    }
    let id = Uuid::new_v4();
    // One atomic increment per Project; the row lock is held until commit, so two creators can
    // never receive the same number, and a number is never handed out again.
    let counter = sqlx::query("UPDATE penfii.projects SET ticket_seq=ticket_seq+1 WHERE id=$1 RETURNING ticket_prefix,ticket_seq")
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;
    let prefix: String = counter.try_get("ticket_prefix")?;
    let sequence: i64 = counter.try_get("ticket_seq")?;
    let public_key = crate::ticket_key::format_key(&prefix, sequence);
    sqlx::query("INSERT INTO penfii.tickets(id,workspace_id,project_id,channel_id,public_key,title,hierarchy_kind,parent_id,status_id,assignee_id,created_by,priority,due_on,description,ticket_type_id,project_sequence,estimate_value,estimate_unit) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17::text::numeric,$18)")
        .bind(id).bind(workspace).bind(project_id).bind(input.channel_id).bind(&public_key).bind(title).bind(&input.hierarchy_kind).bind(input.parent_id).bind(status_id).bind(input.assignee_id).bind(actor_id).bind(&input.priority).bind(input.due_on).bind(&input.description).bind(input.type_id).bind(sequence).bind(estimate.as_ref().map(|e| e.value.clone())).bind(estimate.as_ref().map(|e| e.unit.clone())).execute(&mut *tx).await?;
    plan_fields::write_schedule(&mut tx, id, schedule.as_ref()).await?;
    replace_labels(&mut tx, id, &input.label_ids).await?;
    crate::ticket_files::attach_to_new_ticket(&mut tx, actor_id, workspace, id, &input.file_ids)
        .await?;
    // The Project row is already locked by the key counter above, so the new Ticket joins the
    // end of the shared order without a second writer slipping in.
    planning::append_order(&mut tx, project_id, id).await?;
    let row = sqlx::query_as::<_, TicketRow>(&format!(
        "SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let mut ticket = ticket(row);
    attach_labels(&mut tx, &mut ticket).await?;
    save_snapshot(&mut tx, &ticket, actor_id, command_id, None).await?;
    attach_planned_end(&mut tx, &mut ticket).await?;
    let (change_seq, _) = record_change(
        &mut tx,
        &ticket,
        actor_id,
        command_id,
        "ticket.created",
        json!({"ticketId":id,"revisionNo":1}),
    )
    .await?;
    platform::finish(&mut tx, workspace, actor_id, idem, 201, &ticket).await?;
    tx.commit().await?;
    let _ = role;
    Ok((
        ticket,
        Some(ChangeNotice::Ticket {
            project_id,
            change_seq,
            ticket_id: id,
            ticket_revision_no: 1,
            change_kind: "ticket.created".into(),
        }),
    ))
}

pub async fn update_ticket(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    expected_revision: i64,
    update: TicketUpdate,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let existing = sqlx::query_as::<_, TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL FOR UPDATE OF t")).bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let mut old = ticket(existing);
    require_ticket_editor(&mut tx, old.project_id, old.channel_id, actor_id).await?;
    attach_labels(&mut tx, &mut old).await?;
    let command_id = Uuid::new_v4();
    let body = json!({"ticketId":ticket_id,"expectedRevision":expected_revision,"title":update.title,"description":update.description,"assigneeId":update.assignee_id,"priority":update.priority,"dueOn":update.due_on,"labelIds":update.label_ids,"parentId":update.parent_id,"typeId":update.type_id,"estimate":update.estimate.as_ref().map(|e| e.as_ref().map(|e| json!([e.value, e.unit]))),"schedule":update.schedule.as_ref().map(|s| s.as_ref().map(|s| json!([s.planned_start_at, s.duration_minutes, s.is_schedule_milestone])))});
    if let Some(replay) = platform::replay::<Ticket>(
        &mut tx,
        old.workspace_id,
        actor_id,
        idem,
        "ticket.update",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((replay, None));
    }
    if old.revision_no != expected_revision {
        return Err(CoreError::conflict_extra(
            "version_conflict",
            "ticket revision conflict",
            serde_json::from_value(json!({"currentRevision":old.revision_no})).unwrap_or_default(),
        ));
    }
    if update
        .title
        .as_deref()
        .is_some_and(|s| !title_is_valid(s.trim()))
    {
        return Err(CoreError::validation("title must be 1..300 characters"));
    }
    if update
        .description
        .as_ref()
        .is_some_and(|value| !document_is_valid(value, false))
    {
        return Err(CoreError::validation("description must be a JSON object"));
    }
    let title = update.title.as_deref().map(str::trim).unwrap_or(&old.title);
    let description = update.description;
    let assignee = update.assignee_id.unwrap_or(old.assignee_id);
    if let Some(user) = assignee {
        // Only a new assignee must take part now; a ticket keeps the person it already has.
        check_assignee(
            &mut tx,
            old.workspace_id,
            old.project_id,
            old.channel_id,
            user,
            Some(user) != old.assignee_id,
        )
        .await?;
    }
    let priority = update.priority.unwrap_or(old.priority.clone());
    if priority != old.priority && !priority_is_valid(priority.as_deref()) {
        return Err(CoreError::validation(
            "priority must be LOW, MEDIUM, HIGH, URGENT, or unset",
        ));
    }
    let due_on = update.due_on.unwrap_or(old.due_on);
    let labels = update
        .label_ids
        .unwrap_or_else(|| old.labels.iter().map(|label| label.id).collect());
    validate_labels(&mut tx, old.project_id, &labels).await?;
    let parent_id = update.parent_id.unwrap_or(old.parent_id);
    if parent_id != old.parent_id {
        check_parent(
            &mut tx,
            actor_id,
            old.project_id,
            &old.hierarchy_kind,
            old.channel_id,
            Some(old.id),
            parent_id,
        )
        .await?;
    }
    let type_id = update
        .type_id
        .unwrap_or(old.ticket_type.as_ref().map(|t| t.id));
    if type_id != old.ticket_type.as_ref().map(|t| t.id) {
        validate_type(&mut tx, old.project_id, type_id).await?;
    }
    let estimate = match update.estimate {
        None => old.estimate.clone(),
        Some(input) => {
            plan_fields::resolve_estimate(&mut tx, old.project_id, &old.hierarchy_kind, input)
                .await?
        }
    };
    let schedule = match update.schedule {
        None => old.schedule.clone(),
        Some(input) => plan_fields::resolve_schedule(input)?,
    };
    if title == old.title
        && description
            .as_ref()
            .is_none_or(|value| value == &old.description)
        && assignee == old.assignee_id
        && priority == old.priority
        && due_on == old.due_on
        && parent_id == old.parent_id
        && type_id == old.ticket_type.as_ref().map(|t| t.id)
        && estimate == old.estimate
        && schedule == old.schedule
        && labels == old.labels.iter().map(|label| label.id).collect::<Vec<_>>()
    {
        attach_planned_end(&mut tx, &mut old).await?;
        platform::finish(&mut tx, old.workspace_id, actor_id, idem, 200, &old).await?;
        tx.commit().await?;
        return Ok((old, None));
    }
    let result = sqlx::query("UPDATE penfii.tickets SET title=$3,description=COALESCE($4,description),assignee_id=$5,priority=$6,due_on=$7,parent_id=$8,ticket_type_id=$9,estimate_value=$10::text::numeric,estimate_unit=$11,revision_no=revision_no+1,updated_at=now() WHERE id=$1 AND revision_no=$2")
        .bind(ticket_id).bind(expected_revision).bind(title).bind(&description).bind(assignee).bind(priority).bind(due_on).bind(parent_id).bind(type_id)
        .bind(estimate.as_ref().map(|e| e.value.clone())).bind(estimate.as_ref().map(|e| e.unit.clone())).execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "version_conflict",
            "ticket revision conflict",
        ));
    }
    replace_labels(&mut tx, ticket_id, &labels).await?;
    plan_fields::write_schedule(&mut tx, ticket_id, schedule.as_ref()).await?;
    let mut updated = ticket(
        sqlx::query_as::<_, TicketRow>(&format!(
            "SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1"
        ))
        .bind(ticket_id)
        .fetch_one(&mut *tx)
        .await?,
    );
    attach_labels(&mut tx, &mut updated).await?;
    save_snapshot(&mut tx, &updated, actor_id, command_id, None).await?;
    attach_derived(&mut tx, &mut updated, actor_id).await?;
    attach_planned_end(&mut tx, &mut updated).await?;
    let (seq, _) = record_change(
        &mut tx,
        &updated,
        actor_id,
        command_id,
        "ticket.updated",
        json!({"ticketId":ticket_id,"revisionNo":updated.revision_no}),
    )
    .await?;
    platform::finish(&mut tx, old.workspace_id, actor_id, idem, 200, &updated).await?;
    tx.commit().await?;
    Ok((
        updated,
        Some(ChangeNotice::Ticket {
            project_id: old.project_id,
            change_seq: seq,
            ticket_id,
            ticket_revision_no: expected_revision + 1,
            change_kind: "ticket.updated".into(),
        }),
    ))
}

/// What a status change did to a Ticket.
pub(crate) enum StatusOutcome {
    Unchanged(Ticket),
    Changed { updated: Ticket, change_seq: i64 },
}

/// Checks and applies one status change inside the caller's transaction. `old` is the locked,
/// current Ticket. Shared by the plain status command and by drag-and-drop moves.
pub(crate) async fn apply_status_change(
    tx: &mut PgConnection,
    actor_id: Uuid,
    old: Ticket,
    expected_revision: i64,
    status_id: Uuid,
    command_id: Uuid,
) -> Result<StatusOutcome, CoreError> {
    let ticket_id = old.id;
    if old.revision_no != expected_revision {
        return Err(CoreError::conflict_extra(
            "version_conflict",
            "ticket revision conflict",
            serde_json::from_value(json!({"currentRevision":old.revision_no})).unwrap_or_default(),
        ));
    }
    let target=sqlx::query("SELECT category FROM penfii.workflow_statuses WHERE project_id=$1 AND id=$2 AND retired_at IS NULL").bind(old.project_id).bind(status_id).fetch_optional(&mut *tx).await?.ok_or_else(||CoreError::validation("status must be active in this project workflow"))?;
    let target_category: String = target.try_get("category")?;
    let restricted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.workflow_transitions WHERE from_status_id=$1)",
    )
    .bind(old.status_id)
    .fetch_one(&mut *tx)
    .await?;
    if restricted {
        let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.workflow_transitions WHERE from_status_id=$1 AND to_status_id=$2)").bind(old.status_id).bind(status_id).fetch_one(&mut *tx).await?;
        if !allowed {
            return Err(CoreError::forbidden("workflow transition is not allowed"));
        }
    }
    if old.hierarchy_kind == "TASK" && target_category == "DONE" {
        let unfinished:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.tickets WHERE parent_id=$1 AND hierarchy_kind='SUBTASK' AND status_id IN (SELECT id FROM penfii.workflow_statuses WHERE project_id=$2 AND category NOT IN ('DONE','CANCELED')) AND archived_at IS NULL AND trashed_at IS NULL AND purged_at IS NULL)").bind(ticket_id).bind(old.project_id).fetch_one(&mut *tx).await?;
        if unfinished {
            return Err(CoreError::conflict(
                "unfinished_subtasks",
                "Task has unfinished Subtasks",
            ));
        }
    }
    if status_id == old.status_id {
        return Ok(StatusOutcome::Unchanged(old));
    }
    // A Dependency limits completion only: a successor may run in parallel, but it cannot enter
    // a DONE-category State while a predecessor is not DONE. The check is on the target category,
    // never on State names or order, and runs for every move (command, drag, any source State).
    if target_category == "DONE" {
        let (open_blockers, canceled_blockers) =
            planning::open_blockers(&mut *tx, ticket_id).await?;
        if open_blockers > 0 {
            return Err(CoreError::conflict_extra(
                "dependency_blocked",
                "Ticket cannot be completed while predecessors are unfinished",
                serde_json::from_value(
                    json!({"openBlockers":open_blockers,"canceledBlockers":canceled_blockers}),
                )
                .unwrap_or_default(),
            ));
        }
    }
    let result=sqlx::query("UPDATE penfii.tickets SET status_id=$3,revision_no=revision_no+1,updated_at=now() WHERE id=$1 AND revision_no=$2").bind(ticket_id).bind(expected_revision).bind(status_id).execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "version_conflict",
            "ticket revision conflict",
        ));
    }
    let mut updated = ticket(
        sqlx::query_as::<_, TicketRow>(&format!(
            "SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1"
        ))
        .bind(ticket_id)
        .fetch_one(&mut *tx)
        .await?,
    );
    attach_labels(&mut *tx, &mut updated).await?;
    save_snapshot(&mut *tx, &updated, actor_id, command_id, None).await?;
    attach_derived(&mut *tx, &mut updated, actor_id).await?;
    // The derived end is not part of the revision. A single-Ticket response still carries it,
    // the same way a read does, so replacing the open Ticket does not drop a calculable end.
    attach_planned_end(&mut *tx, &mut updated).await?;
    let (change_seq,_) = record_change(&mut *tx,&updated,actor_id,command_id,"ticket.status_changed",json!({"ticketId":ticket_id,"fromStatusId":old.status_id,"toStatusId":status_id,"revisionNo":updated.revision_no})).await?;
    Ok(StatusOutcome::Changed {
        updated,
        change_seq,
    })
}

/// A live Ticket with its Labels. `lock` takes the row lock for a command.
pub(crate) async fn load_live_ticket(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    lock: bool,
) -> Result<Ticket, CoreError> {
    let lock = if lock { " FOR UPDATE OF t" } else { "" };
    let row=sqlx::query_as::<_,TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL{lock}")).bind(ticket_id).fetch_optional(&mut *conn).await?.ok_or(CoreError::NotFound)?;
    let mut value = ticket(row);
    attach_labels(&mut *conn, &mut value).await?;
    Ok(value)
}

/// A Ticket that may be read: live or archived, never trashed or purged. Reads of an archived Ticket
/// use this; every command keeps [`load_live_ticket`], which is what makes an archived Ticket read-only.
pub(crate) async fn load_readable_ticket(
    conn: &mut PgConnection,
    ticket_id: Uuid,
) -> Result<Ticket, CoreError> {
    let row=sqlx::query_as::<_,TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.trashed_at IS NULL AND t.purged_at IS NULL")).bind(ticket_id).fetch_optional(&mut *conn).await?.ok_or(CoreError::NotFound)?;
    let mut value = ticket(row);
    attach_labels(&mut *conn, &mut value).await?;
    Ok(value)
}

async fn locked_editable_ticket(
    tx: &mut PgConnection,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<Ticket, CoreError> {
    let row=sqlx::query_as::<_,TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL FOR UPDATE OF t")).bind(ticket_id).fetch_optional(&mut *tx).await?.ok_or(CoreError::NotFound)?;
    let mut old = ticket(row);
    require_ticket_editor(&mut *tx, old.project_id, old.channel_id, actor_id).await?;
    attach_labels(&mut *tx, &mut old).await?;
    Ok(old)
}

pub async fn change_status(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    expected_revision: i64,
    status_id: Uuid,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let old = locked_editable_ticket(&mut tx, actor_id, ticket_id).await?;
    let command_id = Uuid::new_v4();
    let body =
        json!({"ticketId":ticket_id,"expectedRevision":expected_revision,"statusId":status_id});
    if let Some(replay) = platform::replay::<Ticket>(
        &mut tx,
        old.workspace_id,
        actor_id,
        idem,
        "ticket.change_status",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((replay, None));
    }
    let workspace = old.workspace_id;
    let project_id = old.project_id;
    let outcome = apply_status_change(
        &mut tx,
        actor_id,
        old,
        expected_revision,
        status_id,
        command_id,
    )
    .await?;
    let (updated, notice) = match outcome {
        StatusOutcome::Unchanged(mut ticket) => {
            attach_planned_end(&mut tx, &mut ticket).await?;
            (ticket, None)
        }
        StatusOutcome::Changed {
            updated,
            change_seq,
        } => {
            let notice = ChangeNotice::Ticket {
                project_id,
                change_seq,
                ticket_id,
                ticket_revision_no: updated.revision_no,
                change_kind: "ticket.status_changed".into(),
            };
            (updated, Some(notice))
        }
    };
    platform::finish(&mut tx, workspace, actor_id, idem, 200, &updated).await?;
    tx.commit().await?;
    Ok((updated, notice))
}

/// Drag-and-drop in one command: an optional State change plus an optional new place in the
/// shared order. A pure reorder is not a Ticket edit, so it adds no revision.
pub async fn move_ticket(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    expected_revision: Option<i64>,
    status_id: Option<Uuid>,
    placement: planning::Placement,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let mut tx = pool.begin().await?;
    let old = locked_editable_ticket(&mut tx, actor_id, ticket_id).await?;
    let command_id = Uuid::new_v4();
    let body = json!({"ticketId":ticket_id,"expectedRevision":expected_revision,"statusId":status_id,"placement":placement.to_json()});
    if let Some(replay) = platform::replay::<Ticket>(
        &mut tx,
        old.workspace_id,
        actor_id,
        idem,
        "ticket.move",
        &body,
    )
    .await?
    {
        tx.commit().await?;
        return Ok((replay, None));
    }
    let workspace = old.workspace_id;
    let project_id = old.project_id;
    let changes_status = status_id.is_some_and(|id| id != old.status_id);
    if changes_status && expected_revision.is_none() {
        return Err(CoreError::validation(
            "expectedRevision is required when the status changes",
        ));
    }
    // The Project row lock serializes order writers; take it before touching rows that follow it.
    planning::lock_project(&mut tx, project_id).await?;
    let mut current = old.clone();
    let mut notice = None;
    if changes_status {
        let outcome = apply_status_change(
            &mut tx,
            actor_id,
            old,
            expected_revision.unwrap_or_default(),
            status_id.unwrap_or_default(),
            command_id,
        )
        .await?;
        if let StatusOutcome::Changed {
            updated,
            change_seq,
        } = outcome
        {
            notice = Some(ChangeNotice::Ticket {
                project_id,
                change_seq,
                ticket_id,
                ticket_revision_no: updated.revision_no,
                change_kind: "ticket.status_changed".into(),
            });
            current = updated;
        }
    }
    let reordered =
        planning::reposition(&mut tx, actor_id, project_id, ticket_id, placement).await?;
    if reordered && notice.is_none() {
        let change_seq = project_change(
            &mut tx,
            project_id,
            ticket_id,
            current.revision_no,
            "ticket.reordered",
        )
        .await?;
        notice = Some(ChangeNotice::Ticket {
            project_id,
            change_seq,
            ticket_id,
            ticket_revision_no: current.revision_no,
            change_kind: "ticket.reordered".into(),
        });
    }
    attach_derived(&mut tx, &mut current, actor_id).await?;
    attach_planned_end(&mut tx, &mut current).await?;
    platform::finish(&mut tx, workspace, actor_id, idem, 200, &current).await?;
    tx.commit().await?;
    Ok((current, notice))
}

pub async fn children(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
) -> Result<Vec<Ticket>, CoreError> {
    let mut tx = pool.begin().await?;
    let access = read_access(&mut tx, actor_id, ticket_id).await?;
    // An archived parent shows its archived children too: they were archived with it.
    let archived_parent = !access.live;
    let rows = sqlx::query_as::<_, TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.parent_id=$1 AND ($2 OR t.archived_at IS NULL) AND t.trashed_at IS NULL AND t.purged_at IS NULL ORDER BY t.created_at,t.id"))
        .bind(ticket_id)
        .bind(archived_parent)
        .fetch_all(&mut *tx)
        .await?;
    let mut access: HashMap<Uuid, bool> = HashMap::new();
    let mut out = Vec::new();
    for row in rows {
        let mut item = ticket(row);
        if let Some(channel_id) = item.channel_id {
            if let std::collections::hash_map::Entry::Vacant(e) = access.entry(channel_id) {
                let allowed = channel_allowed(&mut tx, Some(channel_id), actor_id).await?;
                e.insert(allowed);
            }
            if !access[&channel_id] {
                continue;
            }
        }
        attach_labels(&mut tx, &mut item).await?;
        attach_derived(&mut tx, &mut item, actor_id).await?;
        out.push(item);
    }
    tx.commit().await?;
    Ok(out)
}
