//! Restoring a Ticket to an earlier revision.

use chrono::{DateTime, NaiveDate, Utc};
use penfii_contracts::{Ticket, TicketEstimate, TicketSchedule};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::changes::{record_change, save_snapshot, snapshot_value, status_names};
use super::tickets::{replace_labels, title_is_valid};
use super::{
    TICKET_COLUMNS, TICKET_FROM, TicketRow, attach_derived, attach_labels, attach_planned_end,
    ticket,
};
use crate::error::CoreError;
use crate::plan_fields::{self, ScheduleInput};
use crate::platform::{self, ChangeNotice};
use crate::project_role::{channel_allowed, require_ticket_editor};
use crate::work_history::{self, RESTORABLE_FIELDS};

#[derive(Debug, Clone)]
pub struct RestoreInput {
    pub source_revision: i64,
    pub fields: Vec<String>,
}

fn snapshot_uuid(snapshot: &Value, key: &str) -> Option<Uuid> {
    snapshot
        .get(key)
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
}

fn snapshot_label_ids(snapshot: &Value) -> Vec<Uuid> {
    snapshot
        .get("labels")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .filter_map(|text| Uuid::parse_str(text).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The estimate a snapshot holds, as `(value, unit)`.
fn snapshot_estimate(snapshot: &Value) -> Option<(String, String)> {
    let value = snapshot.get("estimateValue").and_then(Value::as_str)?;
    let unit = snapshot.get("estimateUnit").and_then(Value::as_str)?;
    Some((value.to_owned(), unit.to_owned()))
}

/// The schedule a snapshot holds, checked against today's rules.
fn snapshot_schedule(snapshot: &Value) -> Result<Option<TicketSchedule>, ()> {
    let Some(object) = snapshot.get("schedule").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let start = match object.get("plannedStartAt").and_then(Value::as_str) {
        Some(text) => Some(
            DateTime::parse_from_rfc3339(text)
                .map_err(|_| ())?
                .with_timezone(&Utc),
        ),
        None => None,
    };
    plan_fields::resolve_schedule(Some(ScheduleInput {
        planned_start_at: start,
        duration_minutes: object.get("durationMinutes").and_then(Value::as_i64),
        is_schedule_milestone: object
            .get("isScheduleMilestone")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }))
    .map_err(|_| ())
}

fn snapshot_due_on(snapshot: &Value) -> Result<Option<NaiveDate>, ()> {
    match snapshot.get("dueOn").and_then(Value::as_str) {
        None => Ok(None),
        Some(text) => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| ()),
    }
}

/// Whether one field of an older revision can be put back into the current Ticket now.
/// The reason is a stable code, never a hidden value.
pub(super) async fn restore_blocker(
    conn: &mut PgConnection,
    current: &Ticket,
    snapshot: &Value,
    field: &str,
) -> Result<Option<&'static str>, CoreError> {
    Ok(match field {
        "title" => {
            let valid = snapshot
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|title| title_is_valid(title.trim()));
            (!valid).then_some("invalid_value")
        }
        "dueOn" => snapshot_due_on(snapshot)
            .is_err()
            .then_some("invalid_value"),
        "assignee" => match snapshot_uuid(snapshot, "assigneeId") {
            None => None,
            Some(user) => {
                let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.workspace_members WHERE workspace_id=$1 AND user_id=$2 AND state='ACTIVE')")
                    .bind(current.workspace_id)
                    .bind(user)
                    .fetch_one(&mut *conn)
                    .await?;
                if !active
                    || !crate::access::in_share_scope(conn, current.project_id, None, user).await?
                    || !channel_allowed(conn, current.channel_id, user).await?
                {
                    Some("assignee_unavailable")
                } else {
                    None
                }
            }
        },
        "labels" => {
            let ids = snapshot_label_ids(snapshot);
            if ids.is_empty() {
                None
            } else {
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM penfii.labels WHERE project_id=$1 AND id=ANY($2) AND retired_at IS NULL")
                    .bind(current.project_id)
                    .bind(&ids)
                    .fetch_one(&mut *conn)
                    .await?;
                (count != ids.len() as i64).then_some("label_unavailable")
            }
        }
        "estimate" => match snapshot_estimate(snapshot) {
            None => None,
            Some((_, unit)) => {
                let project_unit: String =
                    sqlx::query_scalar("SELECT estimate_unit FROM penfii.projects WHERE id=$1")
                        .bind(current.project_id)
                        .fetch_one(&mut *conn)
                        .await?;
                (current.hierarchy_kind != "TASK" || project_unit == "NONE" || project_unit != unit)
                    .then_some("estimate_unit_changed")
            }
        },
        "schedule" => snapshot_schedule(snapshot)
            .is_err()
            .then_some("invalid_value"),
        "type" => match snapshot_uuid(snapshot, "typeId") {
            None => None,
            Some(type_id) => {
                let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM penfii.ticket_types WHERE id=$1 AND project_id=$2 AND retired_at IS NULL)")
                    .bind(type_id)
                    .bind(current.project_id)
                    .fetch_one(&mut *conn)
                    .await?;
                (!active).then_some("type_unavailable")
            }
        },
        _ => None,
    })
}

pub(super) async fn live_ticket(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    actor_id: Uuid,
) -> Result<Ticket, CoreError> {
    let row = sqlx::query_as::<_, TicketRow>(&format!(
        "SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.purged_at IS NULL"
    ))
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(CoreError::NotFound)?;
    let mut value = ticket(row);
    attach_labels(conn, &mut value).await?;
    attach_derived(conn, &mut value, actor_id).await?;
    attach_planned_end(conn, &mut value).await?;
    Ok(value)
}

struct RestoredValues {
    title: String,
    description: Value,
    assignee: Option<Uuid>,
    priority: Option<String>,
    due_on: Option<NaiveDate>,
    type_id: Option<Uuid>,
    estimate: Option<TicketEstimate>,
    schedule: Option<TicketSchedule>,
}

/// The field values the restored revision will have: for each field in `changed`, the one from the
/// `source` snapshot; every other field keeps its current value. No database access, and no
/// permission is decided here: `restore_blocker` has already vetted the snapshot's references.
fn restored_values(
    old: &Ticket,
    source: &Value,
    changed: &[String],
) -> Result<RestoredValues, CoreError> {
    let has = |name: &str| changed.iter().any(|field| field == name);
    let title = if has("title") {
        source
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&old.title)
            .trim()
            .to_owned()
    } else {
        old.title.clone()
    };
    let description = if has("description") {
        source.get("description").cloned().unwrap_or(Value::Null)
    } else {
        old.description.clone()
    };
    let assignee = if has("assignee") {
        snapshot_uuid(source, "assigneeId")
    } else {
        old.assignee_id
    };
    let priority = if has("priority") {
        source
            .get("priority")
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        old.priority.clone()
    };
    let due_on = if has("dueOn") {
        snapshot_due_on(source).map_err(|_| CoreError::validation("invalid dueOn"))?
    } else {
        old.due_on
    };
    let type_id = if has("type") {
        snapshot_uuid(source, "typeId")
    } else {
        old.ticket_type.as_ref().map(|t| t.id)
    };
    let estimate = if has("estimate") {
        snapshot_estimate(source).map(|(value, unit)| TicketEstimate { value, unit })
    } else {
        old.estimate.clone()
    };
    let schedule = if has("schedule") {
        snapshot_schedule(source).map_err(|_| CoreError::validation("invalid schedule"))?
    } else {
        old.schedule.clone()
    };
    Ok(RestoredValues {
        title,
        description,
        assignee,
        priority,
        due_on,
        type_id,
        estimate,
        schedule,
    })
}

/// Puts selected fields of an older revision back as a new edit. The old revisions stay as
/// they are; the result is revision N+1 that records where the values came from.
pub async fn restore_ticket(
    pool: &PgPool,
    actor_id: Uuid,
    ticket_id: Uuid,
    expected_revision: i64,
    input: RestoreInput,
    idem: &str,
) -> Result<(Ticket, Option<ChangeNotice>), CoreError> {
    let mut fields = input.fields.clone();
    fields.sort();
    fields.dedup();
    if fields.is_empty()
        || fields
            .iter()
            .any(|f| !RESTORABLE_FIELDS.contains(&f.as_str()))
    {
        return Err(CoreError::validation(
            "fields must be a non-empty list of restorable fields",
        ));
    }
    if input.source_revision < 1 {
        return Err(CoreError::validation("sourceRevision must be positive"));
    }
    let mut tx = pool.begin().await?;
    let existing = sqlx::query_as::<_, TicketRow>(&format!("SELECT {TICKET_COLUMNS} FROM {TICKET_FROM} WHERE t.id=$1 AND t.archived_at IS NULL AND t.trashed_at IS NULL AND t.purged_at IS NULL FOR UPDATE OF t"))
        .bind(ticket_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(CoreError::NotFound)?;
    let mut old = ticket(existing);
    require_ticket_editor(&mut tx, old.project_id, old.channel_id, actor_id).await?;
    attach_labels(&mut tx, &mut old).await?;
    let body = json!({"ticketId":ticket_id,"expectedRevision":expected_revision,"sourceRevision":input.source_revision,"fields":fields});
    if let Some(replay) = platform::replay::<Ticket>(
        &mut tx,
        old.workspace_id,
        actor_id,
        idem,
        "ticket.restore",
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
    let source: Value = sqlx::query_scalar(
        "SELECT snapshot FROM penfii.ticket_revisions WHERE ticket_id=$1 AND revision_no=$2",
    )
    .bind(ticket_id)
    .bind(input.source_revision)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    let names = status_names(&mut tx, old.project_id).await?;
    let current_snapshot = snapshot_value(&mut tx, &old).await?;
    let changed: Vec<String> = work_history::diff(
        &work_history::view(&current_snapshot, &names),
        &work_history::view(&source, &names),
    )
    .into_iter()
    .map(|change| change.field)
    .filter(|field| fields.contains(field))
    .collect();
    if changed.is_empty() {
        attach_derived(&mut tx, &mut old, actor_id).await?;
        platform::finish(&mut tx, old.workspace_id, actor_id, idem, 200, &old).await?;
        tx.commit().await?;
        return Ok((old, None));
    }
    let mut blocked = Vec::new();
    // An estimate written in another unit than the Project's now is skipped on its own and the
    // rest of the restore goes on; any other field that cannot be put back
    // refuses the whole restore, as before.
    let mut skipped = Vec::new();
    for field in &changed {
        if let Some(reason) = restore_blocker(&mut tx, &old, &source, field).await? {
            let entry = json!({"field":field,"reason":reason});
            if field == "estimate" {
                skipped.push(entry);
            } else {
                blocked.push(entry);
            }
        }
    }
    if !blocked.is_empty() || (!skipped.is_empty() && skipped.len() == changed.len()) {
        blocked.extend(skipped);
        return Err(CoreError::conflict_extra(
            "restore_not_applicable",
            "some selected fields cannot be restored now",
            serde_json::from_value(json!({"fields":blocked})).unwrap_or_default(),
        ));
    }
    let changed: Vec<String> = changed
        .into_iter()
        .filter(|field| field != "estimate" || skipped.is_empty())
        .collect();
    let values = restored_values(&old, &source, &changed)?;
    let RestoredValues {
        title,
        description,
        assignee,
        priority,
        due_on,
        type_id,
        estimate,
        schedule,
    } = values;
    let has = |name: &str| changed.iter().any(|field| field == name);
    let result = sqlx::query("UPDATE penfii.tickets SET title=$3,description=$4,assignee_id=$5,priority=$6,due_on=$7,ticket_type_id=$8,estimate_value=$9::text::numeric,estimate_unit=$10,revision_no=revision_no+1,updated_at=now() WHERE id=$1 AND revision_no=$2")
        .bind(ticket_id).bind(expected_revision).bind(&title).bind(&description).bind(assignee).bind(&priority).bind(due_on).bind(type_id)
        .bind(estimate.as_ref().map(|e| e.value.clone())).bind(estimate.as_ref().map(|e| e.unit.clone()))
        .execute(&mut *tx).await?;
    if result.rows_affected() != 1 {
        return Err(CoreError::conflict(
            "version_conflict",
            "ticket revision conflict",
        ));
    }
    if has("labels") {
        replace_labels(&mut tx, ticket_id, &snapshot_label_ids(&source)).await?;
    }
    if has("schedule") {
        plan_fields::write_schedule(&mut tx, ticket_id, schedule.as_ref()).await?;
    }
    let updated = live_ticket(&mut tx, ticket_id, actor_id).await?;
    let command_id = Uuid::new_v4();
    let metadata = json!({"selected":fields,"applied":changed,"skipped":skipped});
    save_snapshot(
        &mut tx,
        &updated,
        actor_id,
        command_id,
        Some((input.source_revision, metadata)),
    )
    .await?;
    let (seq, _) = record_change(
        &mut tx,
        &updated,
        actor_id,
        command_id,
        "ticket.restored",
        json!({"ticketId":ticket_id,"revisionNo":updated.revision_no,"sourceRevision":input.source_revision,"fields":changed,"skipped":skipped}),
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
            change_kind: "ticket.restored".into(),
        }),
    ))
}
